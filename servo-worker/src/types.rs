//! Type definitions for worker HTTP API
//!
//! This module defines the data contracts between the orchestrator (via Cloud Tasks)
//! and the worker.

use serde::{Deserialize, Serialize};
use servo_core::PartitionExecutionContext;
use uuid::Uuid;

/// Payload for scheduler-triggered workflow execution
///
/// This is sent by Cloud Scheduler to trigger a scheduled workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerPayload {
    /// UUID of the workflow to execute
    pub workflow_id: Uuid,

    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,

    /// What triggered this execution
    pub triggered_by: String,
}

/// Task payload sent from orchestrator via Cloud Tasks
///
/// This is the data contract between the orchestrator and worker.
/// The orchestrator pre-compiles the workflow into an execution plan
/// to eliminate compilation overhead on the worker hot path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    /// UUID of the execution record in storage
    pub execution_id: Uuid,

    /// UUID of the workflow to execute
    pub workflow_id: Uuid,

    /// Tenant identifier for multi-tenant isolation
    pub tenant_id: String,

    /// Optional idempotency key for deduplication
    pub idempotency_key: Option<String>,

    /// Pre-compiled execution plan (topologically sorted asset IDs)
    ///
    /// This eliminates the need for the worker to compile workflows on the hot path,
    /// reducing latency by 50-100ms per execution.
    pub execution_plan: Vec<Uuid>,

    /// Partition context for partitioned asset execution
    ///
    /// When present, this provides the partition key and metadata for the
    /// compute function being executed. Used by backfill operations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_context: Option<PartitionExecutionContext>,
}

/// Response from the execute endpoint
///
/// The worker responds immediately with 200 OK and execution status,
/// then processes the workflow in the background.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    /// UUID of the execution
    pub execution_id: Uuid,

    /// Current execution status
    pub status: String,

    /// Optional message (e.g., for errors during validation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ExecuteResponse {
    /// Create a success response indicating execution has started
    pub fn accepted(execution_id: Uuid) -> Self {
        Self {
            execution_id,
            status: "accepted".to_string(),
            message: None,
        }
    }

    /// Create an error response
    #[allow(dead_code)]
    pub fn error(execution_id: Uuid, message: impl Into<String>) -> Self {
        Self {
            execution_id,
            status: "error".to_string(),
            message: Some(message.into()),
        }
    }
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,

    /// Optional version information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl HealthResponse {
    /// Create a healthy response
    pub fn healthy() -> Self {
        Self {
            status: "healthy".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_payload_serialization() {
        let payload = TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "tenant-123".to_string(),
            idempotency_key: Some("idempotency-key".to_string()),
            execution_plan: vec![Uuid::new_v4(), Uuid::new_v4()],
            partition_context: None,
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: TaskPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(payload.execution_id, deserialized.execution_id);
        assert_eq!(payload.workflow_id, deserialized.workflow_id);
        assert_eq!(payload.tenant_id, deserialized.tenant_id);
        assert_eq!(
            payload.execution_plan.len(),
            deserialized.execution_plan.len()
        );
        assert!(deserialized.partition_context.is_none());
    }

    #[test]
    fn test_task_payload_with_partition_context() {
        let partition_ctx = PartitionExecutionContext::new("2024-01-15")
            .with_partition_type("daily")
            .with_timezone("UTC");

        let payload = TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "tenant-123".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
            partition_context: Some(partition_ctx),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("partition_context"));
        assert!(json.contains("2024-01-15"));

        let deserialized: TaskPayload = serde_json::from_str(&json).unwrap();
        assert!(deserialized.partition_context.is_some());
        let ctx = deserialized.partition_context.unwrap();
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
    }

    #[test]
    fn test_task_payload_backward_compatible() {
        // Payloads without partition_context should still deserialize
        let json = r#"{
            "execution_id": "550e8400-e29b-41d4-a716-446655440000",
            "workflow_id": "550e8400-e29b-41d4-a716-446655440001",
            "tenant_id": "test-tenant",
            "idempotency_key": null,
            "execution_plan": []
        }"#;

        let payload: TaskPayload = serde_json::from_str(json).unwrap();
        assert!(payload.partition_context.is_none());
        assert_eq!(payload.tenant_id, "test-tenant");
    }

    #[test]
    fn test_execute_response_accepted() {
        let execution_id = Uuid::new_v4();
        let response = ExecuteResponse::accepted(execution_id);

        assert_eq!(response.execution_id, execution_id);
        assert_eq!(response.status, "accepted");
        assert!(response.message.is_none());
    }

    #[test]
    fn test_execute_response_error() {
        let execution_id = Uuid::new_v4();
        let response = ExecuteResponse::error(execution_id, "Something went wrong");

        assert_eq!(response.execution_id, execution_id);
        assert_eq!(response.status, "error");
        assert_eq!(response.message, Some("Something went wrong".to_string()));
    }

    #[test]
    fn test_health_response() {
        let response = HealthResponse::healthy();

        assert_eq!(response.status, "healthy");
        assert!(response.version.is_some());
    }
}
