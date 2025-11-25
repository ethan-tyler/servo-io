//! HTTP handlers for the worker API
//!
//! Implements:
//! - POST /execute - Execute a workflow (with signature verification)
//! - GET /health - Health check endpoint

use crate::executor::WorkflowExecutor;
use crate::rate_limiter::{self, RateLimitError};
use crate::security::{extract_signature, verify_signature};
use crate::types::{ExecuteResponse, HealthResponse, TaskPayload};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use servo_cloud_gcp::trace_context::{extract_trace_context, has_trace_context};
use std::sync::Arc;
use tracing::{error, info, warn, Instrument};

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub executor: Arc<WorkflowExecutor>,
    pub hmac_secret: String,
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

    // Verify signature
    verify_signature(&body, signature, &state.hmac_secret).map_err(|e| {
        error!(error = %e, "Signature verification failed");
        ExecuteError::InvalidSignature
    })?;

    info!("Signature verified successfully");

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
    let _parent_context = extract_trace_context(&headers);

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
    let execution_span = tracing::info_span!(
        "worker.execute_workflow",
        otel.kind = "consumer",
        execution_id = %execution_id,
        workflow_id = %payload.workflow_id,
        tenant_id = %tenant_id,
        asset_count = payload.execution_plan.len(),
    );

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
