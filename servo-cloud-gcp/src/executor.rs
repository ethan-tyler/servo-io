//! Cloud Run executor implementation
//!
//! # Architecture
//!
//! This executor orchestrates asynchronous workflow execution using GCP's serverless
//! infrastructure. The execution flow is:
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
//! │  CloudRun       │     │   Cloud Tasks   │     │  servo-worker   │
//! │  Executor       │────▶│   Queue         │────▶│  (Cloud Run)    │
//! │  (this module)  │     │                 │     │                 │
//! └─────────────────┘     └─────────────────┘     └─────────────────┘
//!         │                                               │
//!         │                                               │
//!         ▼                                               ▼
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      PostgreSQL                                  │
//! │  (execution records, workflow definitions, tenant isolation)    │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Execution Flow
//!
//! 1. **execute()**: Creates an execution record in PostgreSQL (state: `pending`)
//! 2. **enqueue**: Dispatches a task to Cloud Tasks with execution metadata
//! 3. **Cloud Tasks**: HTTP POST to servo-worker with HMAC-signed payload
//! 4. **servo-worker**: Receives task, updates state to `running`, executes workflow
//! 5. **completion**: Worker updates state to `succeeded` or `failed`
//!
//! ## Important Limitations
//!
//! - **Asynchronous**: `execute()` returns immediately after enqueueing. Use `status()`
//!   to poll for completion.
//! - **Cancellation**: `cancel()` updates the database state but cannot stop an
//!   already-running worker. Workers should periodically check for cancellation.
//! - **No Cloud Run Jobs API**: This executor uses Cloud Tasks → Cloud Run Service,
//!   NOT the Cloud Run Jobs API. The servo-worker must be deployed as a Cloud Run
//!   service that accepts HTTP requests.
//!
//! ## Idempotency
//!
//! When an `idempotency_key` is provided to `execute_with_idempotency()`:
//! - The key is persisted in the `executions.idempotency_key` column
//! - On duplicate requests, the existing execution is returned (no new task enqueued)
//! - Idempotency is scoped per tenant via PostgreSQL RLS

use crate::metrics::{
    EXECUTOR_CANCEL_DURATION, EXECUTOR_CANCEL_TOTAL, EXECUTOR_EXECUTE_DURATION,
    EXECUTOR_EXECUTE_TOTAL, EXECUTOR_STATUS_DURATION,
};
use crate::queue::CloudTasksQueue;
use crate::{Error, Result};
use async_trait::async_trait;
use chrono::Utc;
use servo_core::WorkflowId;
use servo_runtime::executor::{ExecutionId, ExecutionResult};
use servo_runtime::Executor;
use servo_storage::models::ExecutionModel;
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;
use uuid::Uuid;

/// Execution states as stored in the database
pub mod states {
    pub const PENDING: &str = "pending";
    pub const RUNNING: &str = "running";
    pub const SUCCEEDED: &str = "succeeded";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
}

/// Cloud Run executor for running workflows via Cloud Tasks
///
/// This executor orchestrates workflow execution by:
/// 1. Recording executions in PostgreSQL for state tracking
/// 2. Enqueueing tasks to Cloud Tasks for async processing
/// 3. Cloud Tasks triggers Cloud Run workers which execute the actual workflow
///
/// # Example
///
/// ```rust,ignore
/// let executor = CloudRunExecutor::new(
///     storage,
///     queue,
///     TenantId::new("tenant-123"),
/// );
///
/// let result = executor.execute(workflow_id).await?;
/// println!("Execution started: {:?}", result.execution_id);
/// ```
pub struct CloudRunExecutor {
    /// PostgreSQL storage for execution records
    storage: Arc<PostgresStorage>,
    /// Cloud Tasks queue for async task dispatch
    queue: Arc<CloudTasksQueue>,
    /// Tenant context for multi-tenant isolation
    tenant_id: TenantId,
    /// GCP project ID (for metrics/logging)
    project_id: String,
    /// GCP region
    region: String,
}

impl CloudRunExecutor {
    /// Create a new Cloud Run executor
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance
    /// * `queue` - Cloud Tasks queue for task dispatch
    /// * `tenant_id` - Tenant context for RLS
    /// * `project_id` - GCP project ID
    /// * `region` - GCP region
    pub fn new(
        storage: Arc<PostgresStorage>,
        queue: Arc<CloudTasksQueue>,
        tenant_id: TenantId,
        project_id: String,
        region: String,
    ) -> Self {
        Self {
            storage,
            queue,
            tenant_id,
            project_id,
            region,
        }
    }

    /// Create a Cloud Run executor from environment configuration
    ///
    /// Reads configuration from environment variables:
    /// - GCP_PROJECT_ID
    /// - GCP_LOCATION
    /// - GCP_QUEUE_NAME
    /// - GCP_WORKER_URL
    /// - GCP_SERVICE_ACCOUNT_KEY_JSON
    /// - SERVO_HMAC_SECRET
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance
    /// * `tenant_id` - Tenant context for RLS
    pub async fn from_env(storage: Arc<PostgresStorage>, tenant_id: TenantId) -> Result<Self> {
        let config = crate::config::GcpConfig::from_env()?;

        let queue = CloudTasksQueue::new(
            config.project_id.clone(),
            config.location.clone(),
            config.queue_name,
            config.worker_url,
            &config.service_account_key_json,
            config.hmac_secret,
        )?;

        Ok(Self {
            storage,
            queue: Arc::new(queue),
            tenant_id,
            project_id: config.project_id,
            region: config.location,
        })
    }

    /// Execute a workflow with an optional idempotency key
    ///
    /// If an idempotency key is provided and a matching execution already exists,
    /// returns the existing execution instead of creating a new one.
    #[tracing::instrument(
        name = "executor.execute_with_idempotency",
        skip(self),
        fields(
            workflow_id = %workflow_id.0,
            idempotency_key = ?idempotency_key,
            tenant_id = %self.tenant_id.as_str(),
            execution_id = tracing::field::Empty,
        )
    )]
    pub async fn execute_with_idempotency(
        &self,
        workflow_id: WorkflowId,
        idempotency_key: Option<String>,
    ) -> Result<ExecutionResult> {
        let timer = EXECUTOR_EXECUTE_DURATION.start_timer();

        // Generate new execution ID
        let execution_id = Uuid::new_v4();
        let now = Utc::now();

        // Create execution model
        let execution = ExecutionModel {
            id: execution_id,
            workflow_id: workflow_id.0,
            state: states::PENDING.to_string(),
            tenant_id: Some(self.tenant_id.as_str().to_string()),
            idempotency_key: idempotency_key.clone(),
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        // Try to create execution (handles idempotency)
        let (actual_execution_id, was_created) = self
            .storage
            .create_execution_or_get_existing(&execution, &self.tenant_id)
            .await
            .map_err(|e| {
                EXECUTOR_EXECUTE_TOTAL.with_label_values(&["error"]).inc();
                Error::Internal(format!("Failed to create execution: {}", e))
            })?;

        // Record execution_id in span
        tracing::Span::current().record("execution_id", actual_execution_id.to_string().as_str());

        if !was_created {
            // Idempotent case: return existing execution
            tracing::info!(
                execution_id = %actual_execution_id,
                idempotency_key = ?idempotency_key,
                "Returning existing execution (idempotent)"
            );

            // Get existing execution state
            let existing = self
                .storage
                .get_execution(actual_execution_id, &self.tenant_id)
                .await
                .map_err(|e| Error::Internal(format!("Failed to get execution: {}", e)))?;

            timer.observe_duration();
            EXECUTOR_EXECUTE_TOTAL
                .with_label_values(&["idempotent"])
                .inc();

            return Ok(ExecutionResult {
                execution_id: ExecutionId(actual_execution_id),
                success: existing.state == states::SUCCEEDED,
                error: existing.error_message,
            });
        }

        // Enqueue task to Cloud Tasks
        let task_name = self
            .queue
            .enqueue(
                actual_execution_id,
                workflow_id.0,
                self.tenant_id.as_str(),
                idempotency_key,
                vec![], // Execution plan compiled by worker
            )
            .await
            .map_err(|e| {
                EXECUTOR_EXECUTE_TOTAL.with_label_values(&["error"]).inc();
                Error::Internal(format!("Failed to enqueue task: {}", e))
            })?;

        tracing::info!(
            execution_id = %actual_execution_id,
            task_name = %task_name,
            workflow_id = %workflow_id.0,
            "Workflow execution enqueued successfully"
        );

        timer.observe_duration();
        EXECUTOR_EXECUTE_TOTAL.with_label_values(&["success"]).inc();

        Ok(ExecutionResult {
            execution_id: ExecutionId(actual_execution_id),
            success: false, // Still pending
            error: None,
        })
    }

    /// Get detailed execution status from the database
    pub async fn get_execution_details(&self, execution_id: ExecutionId) -> Result<ExecutionModel> {
        self.storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get execution: {}", e)))
    }
}

#[async_trait]
impl Executor for CloudRunExecutor {
    /// Execute a workflow
    ///
    /// Creates an execution record and enqueues a task to Cloud Tasks.
    /// Returns immediately with the execution ID (async execution).
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - ID of the workflow to execute
    ///
    /// # Returns
    ///
    /// `ExecutionResult` with the execution ID. The `success` field will be
    /// `false` initially since the execution is still pending.
    #[tracing::instrument(
        name = "executor.execute",
        skip(self),
        fields(
            workflow_id = %workflow_id.0,
            tenant_id = %self.tenant_id.as_str(),
            project_id = %self.project_id,
            region = %self.region,
        )
    )]
    async fn execute(&self, workflow_id: WorkflowId) -> servo_runtime::Result<ExecutionResult> {
        self.execute_with_idempotency(workflow_id, None)
            .await
            .map_err(|e| servo_runtime::Error::Internal(e.to_string()))
    }

    /// Cancel a running execution
    ///
    /// Updates the execution state to "cancelled". Note that if the task
    /// is already running on Cloud Run, cancellation may not take immediate
    /// effect - the worker should check for cancellation signals.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - ID of the execution to cancel
    #[tracing::instrument(
        name = "executor.cancel",
        skip(self),
        fields(
            execution_id = %execution_id.0,
            tenant_id = %self.tenant_id.as_str(),
        )
    )]
    async fn cancel(&self, execution_id: ExecutionId) -> servo_runtime::Result<()> {
        let timer = EXECUTOR_CANCEL_DURATION.start_timer();

        // Get current execution state
        let mut execution = self
            .storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await
            .map_err(|e| {
                EXECUTOR_CANCEL_TOTAL.with_label_values(&["error"]).inc();
                servo_runtime::Error::NotFound(format!("Execution not found: {}", e))
            })?;

        // Check if cancellation is valid
        match execution.state.as_str() {
            states::PENDING | states::RUNNING => {
                // Can cancel
            }
            states::SUCCEEDED | states::FAILED | states::CANCELLED => {
                // Already terminal state
                tracing::info!(
                    execution_id = %execution_id.0,
                    current_state = %execution.state,
                    "Execution already in terminal state, skipping cancellation"
                );
                timer.observe_duration();
                EXECUTOR_CANCEL_TOTAL.with_label_values(&["skipped"]).inc();
                return Ok(());
            }
            _ => {
                tracing::warn!(
                    execution_id = %execution_id.0,
                    state = %execution.state,
                    "Unknown execution state"
                );
            }
        }

        // Update to cancelled state
        execution.state = states::CANCELLED.to_string();
        execution.completed_at = Some(Utc::now());
        execution.updated_at = Utc::now();

        self.storage
            .update_execution(&execution, &self.tenant_id)
            .await
            .map_err(|e| {
                EXECUTOR_CANCEL_TOTAL.with_label_values(&["error"]).inc();
                servo_runtime::Error::Internal(format!("Failed to update execution: {}", e))
            })?;

        tracing::info!(
            execution_id = %execution_id.0,
            "Execution cancelled successfully"
        );

        timer.observe_duration();
        EXECUTOR_CANCEL_TOTAL.with_label_values(&["success"]).inc();

        Ok(())
    }

    /// Get the status of an execution
    ///
    /// Queries the database for the current execution state.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - ID of the execution to check
    ///
    /// # Returns
    ///
    /// `ExecutionResult` with the current state
    #[tracing::instrument(
        name = "executor.status",
        skip(self),
        fields(
            execution_id = %execution_id.0,
            tenant_id = %self.tenant_id.as_str(),
        )
    )]
    async fn status(&self, execution_id: ExecutionId) -> servo_runtime::Result<ExecutionResult> {
        let timer = EXECUTOR_STATUS_DURATION.start_timer();

        let execution = self
            .storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await
            .map_err(|e| servo_runtime::Error::NotFound(format!("Execution not found: {}", e)))?;

        let success = execution.state == states::SUCCEEDED;
        let error = if execution.state == states::FAILED {
            execution.error_message.clone()
        } else {
            None
        };

        timer.observe_duration();

        Ok(ExecutionResult {
            execution_id,
            success,
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_states() {
        assert_eq!(states::PENDING, "pending");
        assert_eq!(states::RUNNING, "running");
        assert_eq!(states::SUCCEEDED, "succeeded");
        assert_eq!(states::FAILED, "failed");
        assert_eq!(states::CANCELLED, "cancelled");
    }
}
