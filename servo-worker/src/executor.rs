//! Workflow execution engine with timeout enforcement
//!
//! The executor:
//! - Transitions execution state (Pending → Running → Succeeded/Failed/Timeout)
//! - Enforces execution timeout using tokio::select!
//! - Handles errors and records failure details

use crate::types::TaskPayload;
use servo_runtime::state_machine::ExecutionState;
use servo_runtime::ExecutionOrchestrator;
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{error, info, instrument, warn};

/// Errors that can occur during workflow execution
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ExecutionError {
    #[error("State transition failed: {0}")]
    StateTransition(String),

    #[error("Execution timeout after {0:?}")]
    Timeout(Duration),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Workflow executor with timeout enforcement
pub struct WorkflowExecutor {
    storage: Arc<PostgresStorage>,
    timeout: Duration,
}

impl WorkflowExecutor {
    /// Create a new workflow executor
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage for state management
    /// * `timeout` - Maximum execution duration (default: 600s for Cloud Run)
    pub fn new(storage: Arc<PostgresStorage>, timeout: Duration) -> Self {
        Self { storage, timeout }
    }

    /// Get a reference to the storage for health checks
    pub fn storage(&self) -> &Arc<PostgresStorage> {
        &self.storage
    }

    /// Execute a workflow with timeout enforcement
    ///
    /// This method:
    /// 1. Transitions execution from Pending → Running
    /// 2. Executes workflow with timeout using tokio::select!
    /// 3. Transitions to Succeeded/Failed/Timeout based on result
    ///
    /// # Arguments
    ///
    /// * `payload` - Task payload containing execution details
    ///
    /// # Returns
    ///
    /// `Ok(())` if execution completes (success or failure are both valid completions)
    ///
    /// # Errors
    ///
    /// Returns an error only if state management fails
    #[instrument(
        skip(self, payload),
        fields(
            execution_id = %payload.execution_id,
            workflow_id = %payload.workflow_id,
            tenant_id = %payload.tenant_id,
        )
    )]
    pub async fn execute(&self, payload: TaskPayload) -> Result<(), ExecutionError> {
        let tenant_id = TenantId::new(&payload.tenant_id);
        let orchestrator =
            ExecutionOrchestrator::new(self.storage.clone(), servo_runtime::RetryPolicy::default());

        info!(
            asset_count = payload.execution_plan.len(),
            timeout_seconds = self.timeout.as_secs(),
            "Starting workflow execution"
        );

        // Transition to Running state
        orchestrator
            .transition_state(
                payload.execution_id,
                ExecutionState::Pending,
                ExecutionState::Running,
                &tenant_id,
            )
            .await
            .map_err(|e| ExecutionError::StateTransition(format!("Failed to start: {}", e)))?;

        info!("Execution transitioned to Running state");

        // Execute workflow with timeout
        let result = tokio::select! {
            res = self.execute_workflow(&payload) => res,
            _ = tokio::time::sleep(self.timeout) => {
                warn!(
                    timeout_seconds = self.timeout.as_secs(),
                    "Execution timeout exceeded"
                );
                Err(ExecutionError::Timeout(self.timeout))
            }
        };

        // Handle result and transition to final state
        match result {
            Ok(()) => {
                info!("Workflow execution succeeded");
                orchestrator
                    .transition_state(
                        payload.execution_id,
                        ExecutionState::Running,
                        ExecutionState::Succeeded,
                        &tenant_id,
                    )
                    .await
                    .map_err(|e| {
                        ExecutionError::StateTransition(format!("Failed to mark success: {}", e))
                    })?;
            }
            Err(ExecutionError::Timeout(duration)) => {
                error!(timeout = ?duration, "Execution timed out");
                orchestrator
                    .transition_state(
                        payload.execution_id,
                        ExecutionState::Running,
                        ExecutionState::Timeout,
                        &tenant_id,
                    )
                    .await
                    .map_err(|e| {
                        ExecutionError::StateTransition(format!("Failed to mark timeout: {}", e))
                    })?;

                orchestrator
                    .record_failure(
                        payload.execution_id,
                        &format!("Execution timeout after {:?}", duration),
                        &tenant_id,
                    )
                    .await
                    .map_err(|e| {
                        ExecutionError::StateTransition(format!("Failed to record timeout: {}", e))
                    })?;
            }
            Err(e) => {
                error!(error = %e, "Workflow execution failed");
                orchestrator
                    .record_failure(payload.execution_id, &e.to_string(), &tenant_id)
                    .await
                    .map_err(|e| {
                        ExecutionError::StateTransition(format!("Failed to record failure: {}", e))
                    })?;
            }
        }

        info!("Execution completed");
        Ok(())
    }

    /// Execute the workflow assets in order
    ///
    /// This is a placeholder for actual workflow execution logic.
    /// In a real implementation, this would:
    /// - Fetch asset definitions from storage
    /// - Execute each asset in the execution_plan order
    /// - Handle dependencies and data flow
    ///
    /// # Arguments
    ///
    /// * `payload` - Task payload containing execution plan
    #[instrument(skip(self, payload), fields(execution_id = %payload.execution_id))]
    async fn execute_workflow(&self, payload: &TaskPayload) -> Result<(), ExecutionError> {
        info!(
            asset_count = payload.execution_plan.len(),
            "Executing workflow assets"
        );

        // TODO: Implement actual workflow execution
        // For now, this is a placeholder that simulates work
        for (index, asset_id) in payload.execution_plan.iter().enumerate() {
            info!(
                asset_id = %asset_id,
                position = index,
                "Executing asset"
            );

            // Placeholder: In real implementation, fetch and execute asset
            // let asset = self.storage.get_asset(*asset_id, &tenant_id).await?;
            // execute_asset(asset).await?;

            // Simulate work
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("All assets executed successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use servo_storage::TenantId;
    use std::time::Duration;
    use uuid::Uuid;

    #[test]
    fn test_executor_creation() {
        // This test requires a real database connection
        // In practice, you'd use a test database or mock
        let timeout = Duration::from_secs(600);
        // let storage = Arc::new(PostgresStorage::new("postgresql://test").await.unwrap());
        // let executor = WorkflowExecutor::new(storage, timeout);
        // assert_eq!(executor.timeout, timeout);
    }

    #[test]
    fn test_execution_error_types() {
        let err1 = ExecutionError::StateTransition("test".to_string());
        assert!(err1.to_string().contains("State transition"));

        let err2 = ExecutionError::Timeout(Duration::from_secs(10));
        assert!(err2.to_string().contains("timeout"));

        let err3 = ExecutionError::Storage("test".to_string());
        assert!(err3.to_string().contains("Storage"));

        let err4 = ExecutionError::Internal("test".to_string());
        assert!(err4.to_string().contains("Internal"));
    }

    // Note: Full integration tests require a running database
    // and are covered in the integration test suite
}
