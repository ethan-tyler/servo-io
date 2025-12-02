//! Execution orchestrator for managing workflow execution lifecycle
//!
//! The orchestrator is stateless and thread-safe, managing execution state
//! through the PostgreSQL storage layer with Row-Level Security (RLS) enforcement.
//!
//! # Example
//!
//! ```no_run
//! use servo_runtime::orchestrator::ExecutionOrchestrator;
//! use servo_runtime::state_machine::ExecutionState;
//! use servo_storage::{PostgresStorage, TenantId};
//! use std::sync::Arc;
//! use uuid::Uuid;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let storage = Arc::new(PostgresStorage::new("postgresql://...").await?);
//! let orchestrator = ExecutionOrchestrator::new(storage, Default::default());
//!
//! let tenant = TenantId::new("tenant-123");
//! let workflow_id = Uuid::new_v4();
//!
//! // Start execution
//! let execution_id = orchestrator
//!     .start_execution(workflow_id, &tenant, None)
//!     .await?;
//!
//! // Transition to running
//! orchestrator
//!     .transition_state(execution_id, ExecutionState::Pending, ExecutionState::Running, &tenant)
//!     .await?;
//!
//! // Complete successfully
//! orchestrator
//!     .transition_state(execution_id, ExecutionState::Running, ExecutionState::Succeeded, &tenant)
//!     .await?;
//! # Ok(())
//! # }
//! ```

use crate::retry::RetryPolicy;
use crate::state_machine::ExecutionState;
use crate::task_enqueuer::TaskEnqueuer;
use crate::Result;
use chrono::Utc;
use servo_core::PartitionExecutionContext;
use servo_storage::{ExecutionModel, PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, instrument, warn};
use uuid::Uuid;

/// Stateless execution orchestrator for managing workflow execution lifecycle.
///
/// The orchestrator is thread-safe and can be safely shared across multiple
/// async tasks. All state is persisted to PostgreSQL with Row-Level Security
/// enforcement for multi-tenant isolation.
#[derive(Clone)]
pub struct ExecutionOrchestrator {
    storage: Arc<PostgresStorage>,
    retry_policy: RetryPolicy,
    task_enqueuer: Option<Arc<dyn TaskEnqueuer>>,
}

impl ExecutionOrchestrator {
    /// Create a new execution orchestrator
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance (must be configured with servo_app role)
    /// * `retry_policy` - Retry policy for handling transient failures
    pub fn new(storage: Arc<PostgresStorage>, retry_policy: RetryPolicy) -> Self {
        Self {
            storage,
            retry_policy,
            task_enqueuer: None,
        }
    }

    /// Create a new execution orchestrator with a task enqueuer
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance (must be configured with servo_app role)
    /// * `retry_policy` - Retry policy for handling transient failures
    /// * `task_enqueuer` - Task enqueuer for triggering workflow execution (e.g., Cloud Tasks)
    pub fn with_task_enqueuer(
        storage: Arc<PostgresStorage>,
        retry_policy: RetryPolicy,
        task_enqueuer: Arc<dyn TaskEnqueuer>,
    ) -> Self {
        Self {
            storage,
            retry_policy,
            task_enqueuer: Some(task_enqueuer),
        }
    }

    /// Map storage errors to runtime domain errors
    ///
    /// This preserves semantic meaning for proper retry classification:
    /// - NotFound → NotFound (non-retryable, business logic error)
    /// - TenantViolation → TenantViolation (non-retryable, security error)
    /// - ValidationError → Execution (non-retryable, business logic error)
    /// - PoolExhausted, ConnectionFailed → Internal (retryable)
    /// - Database → Internal (may be retryable depending on sqlx error type)
    fn map_storage_error(e: servo_storage::Error, context: &str) -> crate::Error {
        use servo_storage::Error as StorageError;

        match e {
            StorageError::NotFound(msg) => {
                error!(error = %msg, context = %context, "Resource not found");
                crate::Error::NotFound(msg)
            }
            StorageError::TenantViolation(msg) => {
                error!(error = %msg, context = %context, "Tenant isolation violation");
                crate::Error::TenantViolation(msg)
            }
            StorageError::ValidationError(msg) => {
                error!(error = %msg, context = %context, "Validation error");
                crate::Error::Execution(msg)
            }
            StorageError::AlreadyExists(msg) => {
                error!(error = %msg, context = %context, "Resource already exists");
                crate::Error::Execution(msg)
            }
            StorageError::PoolExhausted(_)
            | StorageError::ConnectionFailed(_)
            | StorageError::Database(_) => {
                let err_msg = format!("{}: {}", context, e);
                error!(error = %err_msg, "Transient storage error");
                crate::Error::Internal(err_msg)
            }
            _ => {
                let err_msg = format!("{}: {}", context, e);
                error!(error = %err_msg, "Storage error");
                crate::Error::Internal(err_msg)
            }
        }
    }

    /// Start a new workflow execution
    ///
    /// Creates an execution record in the database with initial state `Pending`.
    /// The operation is performed within a transaction to ensure atomicity.
    ///
    /// If an idempotency key is provided, the orchestrator will check for an existing
    /// execution with the same (tenant_id, workflow_id, idempotency_key) tuple and
    /// return the existing execution_id if found, preventing duplicate executions.
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant ID for multi-tenant isolation (enforced by RLS)
    /// * `idempotency_key` - Optional idempotency key for preventing duplicate executions
    ///
    /// # Returns
    ///
    /// The UUID of the created (or existing) execution
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The workflow doesn't exist
    /// - Database connection fails
    /// - RLS policy blocks access (cross-tenant attempt)
    #[instrument(
        skip(self),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id.as_str(),
            idempotency_key = ?idempotency_key,
        )
    )]
    pub async fn start_execution(
        &self,
        workflow_id: Uuid,
        tenant_id: &TenantId,
        idempotency_key: Option<String>,
    ) -> Result<Uuid> {
        let start = Instant::now();
        tracing::debug!("Starting execution");

        // Create execution in database with Pending state
        // Use atomic create_or_get_existing to handle idempotency without race conditions
        let execution = ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id,
            state: ExecutionState::Pending.into(),
            tenant_id: Some(tenant_id.as_str().to_string()),
            idempotency_key: idempotency_key.clone(),
            started_at: None, // Will be set when transitioning to Running
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let (execution_id, was_created) = if idempotency_key.is_some() {
            // Use atomic idempotent create for requests with idempotency keys
            self.storage
                .create_execution_or_get_existing(&execution, tenant_id)
                .await
                .map_err(|e| Self::map_storage_error(e, "Failed to create execution"))?
        } else {
            // No idempotency key - use regular create
            self.storage
                .create_execution(&execution, tenant_id)
                .await
                .map_err(|e| Self::map_storage_error(e, "Failed to create execution"))?;
            (execution.id, true)
        };

        if !was_created {
            tracing::info!(
                execution_id = %execution_id,
                workflow_id = %workflow_id,
                idempotency_key = ?idempotency_key,
                "Returning existing execution (idempotency hit)"
            );
            return Ok(execution_id);
        }

        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            warn!(
                execution_id = %execution_id,
                duration_ms = elapsed.as_millis(),
                "Slow execution creation"
            );
        }
        tracing::debug!(execution_id = %execution_id, "Execution created");

        // Enqueue task for execution if task enqueuer is configured
        if let Some(enqueuer) = &self.task_enqueuer {
            tracing::debug!("Enqueueing execution task");

            // TODO: Compile execution plan from workflow DAG
            // Currently passes empty vec - worker must fetch workflow and compile plan
            // Future: Pre-compile topological sort of workflow assets here
            let execution_plan = Vec::new();

            // Idempotency is enforced at orchestrator level via atomic insert
            // The key is passed to task enqueuer for completeness
            enqueuer
                .enqueue(
                    execution_id,
                    workflow_id,
                    tenant_id,
                    idempotency_key.clone(),
                    execution_plan,
                    None, // No partition context for non-backfill executions
                )
                .await
                .map_err(|e| {
                    error!(
                        execution_id = %execution_id,
                        error = %e,
                        "Failed to enqueue execution task"
                    );
                    crate::Error::Internal(format!("Failed to enqueue task: {}", e))
                })?;

            tracing::debug!(
                execution_id = %execution_id,
                "Execution task enqueued successfully"
            );
        }

        Ok(execution_id)
    }

    /// Start a workflow execution with partition context (for backfill operations)
    ///
    /// Similar to `start_execution`, but includes partition context that will be
    /// passed through to the worker and ultimately to the Python compute function.
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant context for RLS and isolation
    /// * `idempotency_key` - Optional key for idempotent execution
    /// * `partition_context` - Partition context with partition key and metadata
    #[instrument(
        skip(self, partition_context),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id.as_str(),
            idempotency_key = ?idempotency_key,
            partition_key = %partition_context.partition_key,
        )
    )]
    pub async fn start_execution_with_partition(
        &self,
        workflow_id: Uuid,
        tenant_id: &TenantId,
        idempotency_key: Option<String>,
        partition_context: PartitionExecutionContext,
    ) -> Result<Uuid> {
        let start = Instant::now();
        tracing::debug!("Starting partitioned execution");

        // Create execution in database with Pending state
        let execution = ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id,
            state: ExecutionState::Pending.into(),
            tenant_id: Some(tenant_id.as_str().to_string()),
            idempotency_key: idempotency_key.clone(),
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let (execution_id, was_created) = if idempotency_key.is_some() {
            self.storage
                .create_execution_or_get_existing(&execution, tenant_id)
                .await
                .map_err(|e| Self::map_storage_error(e, "Failed to create execution"))?
        } else {
            self.storage
                .create_execution(&execution, tenant_id)
                .await
                .map_err(|e| Self::map_storage_error(e, "Failed to create execution"))?;
            (execution.id, true)
        };

        if !was_created {
            tracing::info!(
                execution_id = %execution_id,
                workflow_id = %workflow_id,
                partition_key = %partition_context.partition_key,
                "Returning existing execution (idempotency hit)"
            );
            return Ok(execution_id);
        }

        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            warn!(
                execution_id = %execution_id,
                duration_ms = elapsed.as_millis(),
                "Slow execution creation"
            );
        }
        tracing::debug!(execution_id = %execution_id, "Partitioned execution created");

        // Enqueue task with partition context
        if let Some(enqueuer) = &self.task_enqueuer {
            tracing::debug!("Enqueueing partitioned execution task");

            let execution_plan = Vec::new();

            enqueuer
                .enqueue(
                    execution_id,
                    workflow_id,
                    tenant_id,
                    idempotency_key.clone(),
                    execution_plan,
                    Some(partition_context),
                )
                .await
                .map_err(|e| {
                    error!(
                        execution_id = %execution_id,
                        error = %e,
                        "Failed to enqueue partitioned execution task"
                    );
                    crate::Error::Internal(format!("Failed to enqueue task: {}", e))
                })?;

            tracing::debug!(
                execution_id = %execution_id,
                "Partitioned execution task enqueued successfully"
            );
        }

        Ok(execution_id)
    }

    /// Transition execution to a new state
    ///
    /// Validates that the state transition is legal before persisting to the database.
    /// Invalid transitions are rejected without modifying the database.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - UUID of the execution
    /// * `from_state` - Expected current state (for validation)
    /// * `to_state` - Target state to transition to
    /// * `tenant_id` - Tenant ID for RLS enforcement
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The execution doesn't exist or belongs to a different tenant
    /// - The current state doesn't match `from_state`
    /// - The transition is invalid (e.g., Pending -> Succeeded)
    /// - Database connection fails
    #[instrument(
        skip(self),
        fields(
            execution_id = %execution_id,
            from_state = ?from_state,
            to_state = ?to_state,
            tenant_id = %tenant_id.as_str(),
        )
    )]
    pub async fn transition_state(
        &self,
        execution_id: Uuid,
        from_state: ExecutionState,
        to_state: ExecutionState,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let start = Instant::now();
        tracing::debug!("Transitioning execution state");

        // Validate transition is legal
        if !Self::is_valid_transition(from_state, to_state) {
            let err_msg = format!(
                "Invalid state transition from {:?} to {:?}",
                from_state, to_state
            );
            error!(error = %err_msg);
            return Err(crate::Error::Execution(err_msg));
        }

        // Get current execution to verify state matches expectation
        let current = self
            .storage
            .get_execution(execution_id, tenant_id)
            .await
            .map_err(|e| Self::map_storage_error(e, "Failed to get execution"))?;

        let current_state: ExecutionState = current.state.as_str().try_into().map_err(|e| {
            error!(error = %e, "Invalid state in database");
            e
        })?;
        if current_state != from_state {
            let err_msg = format!(
                "State mismatch: expected {:?}, got {:?}",
                from_state, current_state
            );
            error!(error = %err_msg);
            return Err(crate::Error::Execution(err_msg));
        }

        // Update execution state
        let mut updated = current.clone();
        updated.state = to_state.into();
        updated.updated_at = Utc::now();

        // Set timestamps based on state
        match to_state {
            ExecutionState::Running => {
                if updated.started_at.is_none() {
                    updated.started_at = Some(Utc::now());
                }
            }
            ExecutionState::Succeeded
            | ExecutionState::Failed
            | ExecutionState::Timeout
            | ExecutionState::Cancelled => {
                if updated.completed_at.is_none() {
                    updated.completed_at = Some(Utc::now());
                }
            }
            _ => {}
        }

        self.storage
            .update_execution(&updated, tenant_id)
            .await
            .map_err(|e| Self::map_storage_error(e, "Failed to update execution"))?;

        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            warn!(duration_ms = elapsed.as_millis(), "Slow state transition");
        }

        tracing::debug!(new_state = ?to_state, "State transition complete");
        Ok(())
    }

    /// Record execution failure with error message
    ///
    /// Transitions the execution to Failed state and stores the error message.
    /// This is a convenience method that combines state transition and error recording.
    ///
    /// # Arguments
    ///
    /// * `execution_id` - UUID of the execution
    /// * `error` - Error message to record
    /// * `tenant_id` - Tenant ID for RLS enforcement
    #[instrument(
        skip(self, error),
        fields(
            execution_id = %execution_id,
            tenant_id = %tenant_id.as_str(),
        )
    )]
    pub async fn record_failure(
        &self,
        execution_id: Uuid,
        error: &str,
        tenant_id: &TenantId,
    ) -> Result<()> {
        tracing::debug!(error = %error, "Recording execution failure");

        let current = self
            .storage
            .get_execution(execution_id, tenant_id)
            .await
            .map_err(|e| Self::map_storage_error(e, "Failed to get execution"))?;

        let current_state: ExecutionState = current.state.as_str().try_into().map_err(|e| {
            error!(error = %e, "Invalid state in database");
            e
        })?;

        // Validate we can transition to Failed from current state
        if !Self::is_valid_transition(current_state, ExecutionState::Failed) {
            let err_msg = format!("Cannot transition to Failed from state {:?}", current_state);
            error!(error = %err_msg);
            return Err(crate::Error::Execution(err_msg));
        }

        let mut updated = current;
        updated.state = ExecutionState::Failed.into();
        updated.error_message = Some(error.to_string());
        updated.completed_at = Some(Utc::now());
        updated.updated_at = Utc::now();

        self.storage
            .update_execution(&updated, tenant_id)
            .await
            .map_err(|e| Self::map_storage_error(e, "Failed to record failure"))?;

        tracing::debug!("Execution failure recorded");
        Ok(())
    }

    /// Validate if a state transition is legal
    ///
    /// This is a pure function that checks if transitioning from one state
    /// to another is allowed by the state machine rules.
    ///
    /// # Allowed Transitions
    ///
    /// - Pending → Running
    /// - Running → Succeeded, Failed, Timeout, Cancelled
    /// - Failed → Running (retry)
    ///
    /// # Arguments
    ///
    /// * `from` - Current state
    /// * `to` - Target state
    ///
    /// # Returns
    ///
    /// `true` if the transition is valid, `false` otherwise
    pub fn is_valid_transition(from: ExecutionState, to: ExecutionState) -> bool {
        use ExecutionState::*;

        matches!(
            (from, to),
            (Pending, Running)
                | (Running, Succeeded | Failed | Timeout | Cancelled)
                | (Failed, Running) // Allow retry
        )
    }

    /// Get the retry policy used by this orchestrator
    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }
}

// Implement conversion from ExecutionState to String
impl From<ExecutionState> for String {
    fn from(state: ExecutionState) -> Self {
        match state {
            ExecutionState::Pending => "pending".to_string(),
            ExecutionState::Running => "running".to_string(),
            ExecutionState::Succeeded => "succeeded".to_string(),
            ExecutionState::Failed => "failed".to_string(),
            ExecutionState::Cancelled => "cancelled".to_string(),
            ExecutionState::Timeout => "timeout".to_string(),
        }
    }
}

// Implement fallible conversion from &str to ExecutionState
impl std::convert::TryFrom<&str> for ExecutionState {
    type Error = crate::Error;

    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        match s {
            "pending" => Ok(ExecutionState::Pending),
            "running" => Ok(ExecutionState::Running),
            "succeeded" => Ok(ExecutionState::Succeeded),
            "failed" => Ok(ExecutionState::Failed),
            "cancelled" => Ok(ExecutionState::Cancelled),
            "timeout" => Ok(ExecutionState::Timeout),
            _ => Err(crate::Error::InvalidState(format!(
                "Unknown execution state: {}",
                s
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        use ExecutionState::*;

        // Valid transitions
        assert!(ExecutionOrchestrator::is_valid_transition(Pending, Running));
        assert!(ExecutionOrchestrator::is_valid_transition(
            Running, Succeeded
        ));
        assert!(ExecutionOrchestrator::is_valid_transition(Running, Failed));
        assert!(ExecutionOrchestrator::is_valid_transition(Running, Timeout));
        assert!(ExecutionOrchestrator::is_valid_transition(
            Running, Cancelled
        ));
        assert!(ExecutionOrchestrator::is_valid_transition(Failed, Running)); // Retry
    }

    #[test]
    fn test_invalid_transitions() {
        use ExecutionState::*;

        // Invalid transitions
        assert!(!ExecutionOrchestrator::is_valid_transition(
            Pending, Succeeded
        ));
        assert!(!ExecutionOrchestrator::is_valid_transition(Pending, Failed));
        assert!(!ExecutionOrchestrator::is_valid_transition(
            Succeeded, Running
        ));
        assert!(!ExecutionOrchestrator::is_valid_transition(
            Succeeded, Failed
        ));
        assert!(!ExecutionOrchestrator::is_valid_transition(
            Cancelled, Running
        ));
    }

    #[test]
    fn test_state_string_conversion() {
        use ExecutionState::*;

        let states = vec![
            (Pending, "pending"),
            (Running, "running"),
            (Succeeded, "succeeded"),
            (Failed, "failed"),
            (Cancelled, "cancelled"),
            (Timeout, "timeout"),
        ];

        for (state, expected) in states {
            let string: String = state.into();
            assert_eq!(string, expected);

            let recovered: ExecutionState =
                expected.try_into().expect("Valid state should convert");
            assert_eq!(state, recovered);
        }
    }

    #[test]
    fn test_unknown_state_returns_error() {
        use std::convert::TryInto;
        let result: std::result::Result<ExecutionState, crate::Error> = "unknown_state".try_into();
        assert!(
            result.is_err(),
            "Unknown state should return InvalidState error"
        );
        match result {
            Err(crate::Error::InvalidState(msg)) => {
                assert!(msg.contains("unknown_state"));
            }
            _ => panic!("Expected InvalidState error"),
        }
    }
}
