//! HTTP handlers for the worker API
//!
//! Implements:
//! - POST /execute - Execute a workflow (with signature verification)
//! - GET /health - Health check endpoint

use crate::executor::WorkflowExecutor;
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
use std::sync::Arc;
use tracing::{error, info, warn};

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub executor: Arc<WorkflowExecutor>,
    pub hmac_secret: String,
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
    // Extract signature from headers
    let signature_header = headers.get("x-servo-signature").and_then(|v| v.to_str().ok());
    let signature =
        extract_signature(signature_header).map_err(|_| ExecuteError::MissingSignature)?;

    // Verify signature
    verify_signature(&body, signature, &state.hmac_secret)
        .map_err(|e| {
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

    info!(
        execution_id = %execution_id,
        workflow_id = %payload.workflow_id,
        tenant_id = %payload.tenant_id,
        asset_count = payload.execution_plan.len(),
        "Received execution request"
    );

    // Spawn background task for execution
    // This allows us to return 200 OK immediately (Cloud Tasks requirement)
    let executor = state.executor.clone();
    tokio::spawn(async move {
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
    });

    // Return immediate response
    Ok(Json(ExecuteResponse::accepted(execution_id)))
}

/// Health check endpoint
///
/// Returns 200 OK if the service is running.
/// Used by Cloud Run for liveness and readiness probes.
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse::healthy())
}

/// Error types for execute handler
#[derive(Debug)]
pub enum ExecuteError {
    MissingSignature,
    InvalidSignature,
    InvalidPayload(String),
}

impl IntoResponse for ExecuteError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ExecuteError::MissingSignature => {
                (StatusCode::UNAUTHORIZED, "Missing signature header")
            }
            ExecuteError::InvalidSignature => {
                (StatusCode::UNAUTHORIZED, "Invalid signature")
            }
            ExecuteError::InvalidPayload(ref msg) => {
                warn!(error = %msg, "Invalid payload received");
                (StatusCode::BAD_REQUEST, "Invalid payload")
            }
        };

        (status, message).into_response()
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
