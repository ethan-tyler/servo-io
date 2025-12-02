//! HTTP handlers for the worker API
//!
//! Implements:
//! - POST /execute - Execute a workflow (with signature verification)
//! - GET /health - Health check endpoint

use crate::executor::WorkflowExecutor;
use crate::rate_limiter::{self, RateLimitError};
use crate::secrets_provider::SecretsProvider;
use crate::security::{extract_signature, verify_signature};
use crate::types::{ExecuteResponse, HealthResponse, SchedulerPayload, TaskPayload};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use servo_cloud_gcp::trace_context::{extract_trace_context, has_trace_context};
use std::sync::Arc;
use tracing::{error, info, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub executor: Arc<WorkflowExecutor>,
    pub secrets_provider: Arc<SecretsProvider>,
    pub oidc_validator: Arc<crate::oidc::OidcValidator>,
    pub tenant_rate_limiter: Arc<rate_limiter::TenantRateLimiter>,
    pub ip_rate_limiter: Arc<rate_limiter::IpRateLimiter>,
}

/// Execute a workflow task
///
/// This endpoint:
/// 1. Verifies HMAC signature
/// 2. Decodes base64 payload
/// 3. Responds immediately with 200 OK
/// 4. Spawns background task for execution
///
/// # Request Format
///
/// - Header: `X-Servo-Signature: <hex-encoded-hmac-sha256>`
/// - Body: Base64-encoded JSON payload
///
/// # Response
///
/// Returns 200 OK immediately after spawning execution task.
/// Actual execution happens in background.
pub async fn execute_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ExecuteResponse>, ExecuteError> {
    // OIDC validation (caller identity) - FIRST
    // Verifies that the request is from Google Cloud Tasks
    if state.oidc_validator.config().enabled {
        let auth_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(ExecuteError::MissingOidcToken)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(ExecuteError::InvalidOidcToken)?;

        state
            .oidc_validator
            .validate_token(token)
            .await
            .map_err(|e| {
                warn!(error = %e, "OIDC token validation failed");
                ExecuteError::InvalidOidcToken
            })?;

        info!("OIDC token validated successfully");
    }

    // HMAC validation (payload integrity) - SECOND
    // Verifies that the task body hasn't been tampered with
    // Extract signature from headers
    let signature_header = headers
        .get("x-servo-signature")
        .and_then(|v| v.to_str().ok());
    let signature =
        extract_signature(signature_header).map_err(|_| ExecuteError::MissingSignature)?;

    // Verify signature with any valid secret (supports rotation)
    let valid_secrets = state.secrets_provider.get_all_valid_secrets().await;
    if valid_secrets.is_empty() {
        error!("No valid secrets available for signature verification");
        return Err(ExecuteError::InvalidSignature);
    }

    let mut signature_valid = false;
    for secret in &valid_secrets {
        if verify_signature(&body, signature, secret).is_ok() {
            signature_valid = true;
            break;
        }
    }

    if !signature_valid {
        error!(
            secrets_count = valid_secrets.len(),
            "Signature verification failed with all available secrets"
        );
        return Err(ExecuteError::InvalidSignature);
    }

    info!(
        secrets_count = valid_secrets.len(),
        "Signature verified successfully"
    );

    // Decode base64 payload
    let decoded = STANDARD.decode(&body).map_err(|e| {
        error!(error = %e, "Failed to decode base64 payload");
        ExecuteError::InvalidPayload("Invalid base64 encoding".to_string())
    })?;

    // Deserialize JSON
    let payload: TaskPayload = serde_json::from_slice(&decoded).map_err(|e| {
        error!(error = %e, "Failed to deserialize payload");
        ExecuteError::InvalidPayload(format!("Invalid JSON: {}", e))
    })?;

    let execution_id = payload.execution_id;
    let tenant_id = &payload.tenant_id;

    // Extract trace context from Cloud Tasks headers for distributed tracing
    let has_trace = has_trace_context(&headers);
    let parent_context = extract_trace_context(&headers);

    info!(
        execution_id = %execution_id,
        workflow_id = %payload.workflow_id,
        tenant_id = %tenant_id,
        asset_count = payload.execution_plan.len(),
        has_trace_context = has_trace,
        "Received execution request"
    );

    // Per-tenant rate limiting (after auth, before expensive work)
    if let Err(rate_limit_error) = state.tenant_rate_limiter.check_tenant(tenant_id) {
        warn!(
            tenant_id = %tenant_id,
            execution_id = %execution_id,
            error = %rate_limit_error,
            "Tenant rate limit exceeded"
        );
        return Err(ExecuteError::RateLimitExceeded(rate_limit_error));
    }

    // Create a span for the background execution that links to the parent trace
    // Use OpenTelemetrySpanExt to set the parent context from the extracted trace headers
    let execution_span = tracing::info_span!(
        "worker.execute_workflow",
        otel.kind = "consumer",
        execution_id = %execution_id,
        workflow_id = %payload.workflow_id,
        tenant_id = %tenant_id,
        asset_count = payload.execution_plan.len(),
    );

    // Link the span to the parent trace context from Cloud Tasks headers
    // This ensures end-to-end trace continuity: orchestrator → Cloud Tasks → worker
    execution_span.set_parent(parent_context);

    // Spawn background task for execution
    // This allows us to return 200 OK immediately (Cloud Tasks requirement)
    // The task is instrumented with the span to maintain trace context
    let executor = state.executor.clone();
    tokio::spawn(
        async move {
            match executor.execute(payload).await {
                Ok(()) => {
                    info!(
                        execution_id = %execution_id,
                        "Workflow execution completed successfully"
                    );
                }
                Err(e) => {
                    error!(
                        execution_id = %execution_id,
                        error = %e,
                        "Workflow execution failed"
                    );
                }
            }
        }
        .instrument(execution_span),
    );

    // Return immediate response
    Ok(Json(ExecuteResponse::accepted(execution_id)))
}

/// Execute a workflow triggered by Cloud Scheduler
///
/// This endpoint:
/// 1. Verifies OIDC token (no HMAC signature - Scheduler uses OIDC only)
/// 2. Creates an execution record
/// 3. Spawns background task for execution
///
/// # Path Parameters
///
/// * `workflow_id` - UUID of the workflow to execute
///
/// # Request Format
///
/// - Header: `Authorization: Bearer <oidc-token>`
/// - Header: `X-Servo-Tenant-Id: <tenant-id>`
/// - Body: JSON payload with `workflow_id`, `tenant_id`, `triggered_by`
///
/// # Response
///
/// Returns 200 OK with execution_id after spawning execution task.
pub async fn scheduler_execute_handler(
    State(state): State<AppState>,
    Path(workflow_id): Path<uuid::Uuid>,
    headers: HeaderMap,
    Json(payload): Json<SchedulerPayload>,
) -> Result<Json<ExecuteResponse>, SchedulerExecuteError> {
    // OIDC validation (caller identity) - REQUIRED for Cloud Scheduler
    if state.oidc_validator.config().enabled {
        let auth_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(SchedulerExecuteError::MissingOidcToken)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(SchedulerExecuteError::InvalidOidcToken)?;

        state
            .oidc_validator
            .validate_token(token)
            .await
            .map_err(|e| {
                warn!(error = %e, "OIDC token validation failed for scheduler request");
                SchedulerExecuteError::InvalidOidcToken
            })?;

        info!("OIDC token validated successfully for scheduler request");
    }

    // Validate payload workflow_id matches path
    if payload.workflow_id != workflow_id {
        warn!(
            path_workflow_id = %workflow_id,
            payload_workflow_id = %payload.workflow_id,
            "Workflow ID mismatch between path and payload"
        );
        return Err(SchedulerExecuteError::WorkflowIdMismatch);
    }

    let tenant_id = &payload.tenant_id;

    info!(
        workflow_id = %workflow_id,
        tenant_id = %tenant_id,
        triggered_by = %payload.triggered_by,
        "Received scheduler execution request"
    );

    // Per-tenant rate limiting (after auth, before expensive work)
    if let Err(rate_limit_error) = state.tenant_rate_limiter.check_tenant(tenant_id) {
        warn!(
            tenant_id = %tenant_id,
            workflow_id = %workflow_id,
            error = %rate_limit_error,
            "Tenant rate limit exceeded for scheduler request"
        );
        return Err(SchedulerExecuteError::RateLimitExceeded(rate_limit_error));
    }

    // Clone executor for background task
    let executor = state.executor.clone();
    let tenant_id_clone = tenant_id.clone();

    // Spawn background task for execution
    tokio::spawn(async move {
        match executor
            .execute_scheduled(workflow_id, &tenant_id_clone)
            .await
        {
            Ok(execution_id) => {
                info!(
                    execution_id = %execution_id,
                    workflow_id = %workflow_id,
                    "Scheduled workflow execution completed successfully"
                );
            }
            Err(e) => {
                error!(
                    workflow_id = %workflow_id,
                    error = %e,
                    "Scheduled workflow execution failed"
                );
            }
        }
    });

    // Return immediate response with a placeholder execution ID
    // The actual execution_id is created in the background task
    Ok(Json(ExecuteResponse {
        execution_id: workflow_id, // Use workflow_id as placeholder
        status: "accepted".to_string(),
        message: Some("Scheduled execution started".to_string()),
    }))
}

/// Error types for scheduler execute handler
#[derive(Debug)]
pub enum SchedulerExecuteError {
    MissingOidcToken,
    InvalidOidcToken,
    WorkflowIdMismatch,
    RateLimitExceeded(RateLimitError),
}

impl IntoResponse for SchedulerExecuteError {
    fn into_response(self) -> Response {
        let (status, message, retry_after) = match self {
            SchedulerExecuteError::MissingOidcToken => (
                StatusCode::UNAUTHORIZED,
                "Missing OIDC token".to_string(),
                None,
            ),
            SchedulerExecuteError::InvalidOidcToken => (
                StatusCode::UNAUTHORIZED,
                "Invalid OIDC token".to_string(),
                None,
            ),
            SchedulerExecuteError::WorkflowIdMismatch => (
                StatusCode::BAD_REQUEST,
                "Workflow ID in path does not match payload".to_string(),
                None,
            ),
            SchedulerExecuteError::RateLimitExceeded(ref err) => {
                let retry_after_secs = match err {
                    RateLimitError::TenantLimitExceeded { .. } => 1,
                    RateLimitError::IpLimitExceeded { .. } => 60,
                };
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("Rate limit exceeded: {}", err),
                    Some(retry_after_secs),
                )
            }
        };

        let mut response = (status, message).into_response();
        if let Some(retry_secs) = retry_after {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_str(&retry_secs.to_string()).unwrap(),
            );
        }
        response
    }
}

/// Health check endpoint (liveness probe)
///
/// Returns 200 OK if the service is running.
/// This is a simple liveness check - it doesn't verify external dependencies.
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse::healthy())
}

/// Readiness check endpoint
///
/// Returns 200 OK if the service is ready to accept traffic.
/// Checks:
/// - Database connectivity
/// - Token acquisition capability (if configured)
///
/// This is separate from /health to properly support liveness vs readiness probes.
pub async fn ready_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Check database connectivity
    let db_check = state.executor.storage().health_check().await;

    if db_check.is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "not_ready",
                "reason": "database_unreachable"
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ready",
            "checks": {
                "database": "ok"
            }
        })),
    )
        .into_response()
}

/// Metrics endpoint
///
/// Exposes Prometheus-format metrics.
/// Optionally authenticated with Bearer token (if SERVO_METRICS_TOKEN is set).
/// Rate limited per IP address to prevent abuse.
///
/// Returns all registered Prometheus metrics in text format.
pub async fn metrics_handler(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    use prometheus::{Encoder, TextEncoder};

    // IP-based rate limiting (early check to prevent abuse)
    let client_ip = addr.ip();
    if let Err(rate_limit_error) = state.ip_rate_limiter.check_ip(client_ip) {
        warn!(
            ip = %client_ip,
            error = %rate_limit_error,
            "Metrics endpoint rate limit exceeded"
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(axum::http::header::RETRY_AFTER, "60")],
            "Rate limit exceeded for metrics endpoint",
        )
            .into_response();
    }

    // Optional authentication check
    if let Ok(expected_token) = std::env::var("SERVO_METRICS_TOKEN") {
        let auth_header = headers.get("authorization").and_then(|v| v.to_str().ok());

        if let Some(auth) = auth_header {
            if let Some(token) = auth.strip_prefix("Bearer ") {
                if token != expected_token {
                    return (StatusCode::UNAUTHORIZED, "Invalid metrics token").into_response();
                }
            } else {
                return (StatusCode::UNAUTHORIZED, "Invalid authorization format").into_response();
            }
        } else {
            return (StatusCode::UNAUTHORIZED, "Missing authorization header").into_response();
        }
    }

    // Gather metrics from all registries
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();

    let mut buffer = vec![];
    match encoder.encode(&metric_families, &mut buffer) {
        Ok(()) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4",
            )],
            buffer,
        )
            .into_response(),
        Err(e) => {
            error!(error = %e, "Failed to encode metrics");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to encode metrics",
            )
                .into_response()
        }
    }
}

/// Error types for execute handler
#[derive(Debug)]
pub enum ExecuteError {
    MissingOidcToken,
    InvalidOidcToken,
    MissingSignature,
    InvalidSignature,
    InvalidPayload(String),
    RateLimitExceeded(RateLimitError),
}

impl IntoResponse for ExecuteError {
    fn into_response(self) -> Response {
        let (status, message, retry_after) = match self {
            ExecuteError::MissingOidcToken => (
                StatusCode::UNAUTHORIZED,
                "Missing OIDC token".to_string(),
                None,
            ),
            ExecuteError::InvalidOidcToken => (
                StatusCode::UNAUTHORIZED,
                "Invalid OIDC token".to_string(),
                None,
            ),
            ExecuteError::MissingSignature => (
                StatusCode::UNAUTHORIZED,
                "Missing signature header".to_string(),
                None,
            ),
            ExecuteError::InvalidSignature => (
                StatusCode::UNAUTHORIZED,
                "Invalid signature".to_string(),
                None,
            ),
            ExecuteError::InvalidPayload(ref msg) => {
                warn!(error = %msg, "Invalid payload received");
                (StatusCode::BAD_REQUEST, "Invalid payload".to_string(), None)
            }
            ExecuteError::RateLimitExceeded(ref err) => {
                // Return 429 with Retry-After header
                let retry_after_secs = match err {
                    RateLimitError::TenantLimitExceeded { .. } => {
                        // For per-second limits, suggest retrying after 1 second
                        1
                    }
                    RateLimitError::IpLimitExceeded { .. } => {
                        // For per-minute limits, suggest retrying after 60 seconds
                        60
                    }
                };
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("Rate limit exceeded: {}", err),
                    Some(retry_after_secs),
                )
            }
        };

        // Build response with optional Retry-After header
        let mut response = (status, message).into_response();
        if let Some(retry_secs) = retry_after {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_str(&retry_secs.to_string()).unwrap(),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_error_responses() {
        let missing_sig = ExecuteError::MissingSignature;
        let response = missing_sig.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let invalid_sig = ExecuteError::InvalidSignature;
        let response = invalid_sig.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let invalid_payload = ExecuteError::InvalidPayload("test error".to_string());
        let response = invalid_payload.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_health_handler() {
        let response = health_handler().await;
        assert_eq!(response.0.status, "healthy");
        assert!(response.0.version.is_some());
    }

    // Note: Full integration tests for execute_handler require a running database
    // and are covered in the integration test suite
}
