//! Workflow execution engine with timeout enforcement
//!
//! The executor:
//! - Transitions execution state (Pending → Running → Succeeded/Failed/Timeout)
//! - Enforces execution timeout using tokio::select!
//! - Runs data quality checks after each asset execution
//! - Handles blocking check failures and records results
//! - Handles errors and records failure details

use crate::check_validator::{validate_check, AssetOutput};
use crate::metrics::{record_check_execution, record_execution, Timer};
use crate::pii::PiiFilter;
use crate::types::TaskPayload;
use servo_runtime::state_machine::ExecutionState;
use servo_runtime::ExecutionOrchestrator;
use servo_storage::models::CheckResultModel;
use servo_storage::{Json, PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

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

    #[error("Blocking check failed for asset {asset_id}: {message}")]
    BlockingCheckFailure {
        asset_id: uuid::Uuid,
        check_name: String,
        message: String,
    },

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Workflow executor with timeout enforcement
pub struct WorkflowExecutor {
    storage: Arc<PostgresStorage>,
    timeout: Duration,
    pii_filter: PiiFilter,
}

impl WorkflowExecutor {
    /// Create a new workflow executor
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage for state management
    /// * `timeout` - Maximum execution duration (default: 600s for Cloud Run)
    pub fn new(storage: Arc<PostgresStorage>, timeout: Duration) -> Self {
        Self {
            storage,
            timeout,
            pii_filter: PiiFilter::default_filter(),
        }
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
            otel.name = "executor.execute",
            execution_id = %payload.execution_id,
            workflow_id = %payload.workflow_id,
            tenant_id = %payload.tenant_id,
            otel.status_code = tracing::field::Empty,
        )
    )]
    pub async fn execute(&self, payload: TaskPayload) -> Result<(), ExecutionError> {
        let tenant_id = TenantId::new(&payload.tenant_id);
        let tenant_id_str = payload.tenant_id.clone();
        let asset_count = payload.execution_plan.len();
        let timer = Timer::start();

        let orchestrator =
            ExecutionOrchestrator::new(self.storage.clone(), servo_runtime::RetryPolicy::default());

        info!(
            asset_count = asset_count,
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
        let duration_secs = timer.elapsed_secs();

        match result {
            Ok(()) => {
                info!(
                    duration_secs = duration_secs,
                    "Workflow execution succeeded"
                );
                record_execution("succeeded", &tenant_id_str, duration_secs, asset_count);

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
                error!(timeout = ?duration, duration_secs = duration_secs, "Execution timed out");
                record_execution("timeout", &tenant_id_str, duration_secs, asset_count);

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
                error!(error = %e, duration_secs = duration_secs, "Workflow execution failed");
                record_execution("failed", &tenant_id_str, duration_secs, asset_count);

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
    /// This method:
    /// - Executes each asset in the execution_plan order
    /// - Runs data quality checks after each asset
    /// - Stores check results in the database
    /// - Halts execution if blocking checks fail
    ///
    /// # Arguments
    ///
    /// * `payload` - Task payload containing execution plan
    #[instrument(
        skip(self, payload),
        fields(
            otel.name = "executor.execute_workflow",
            execution_id = %payload.execution_id,
            workflow_id = %payload.workflow_id,
            tenant_id = %payload.tenant_id,
            asset_count = payload.execution_plan.len(),
        )
    )]
    async fn execute_workflow(&self, payload: &TaskPayload) -> Result<(), ExecutionError> {
        let tenant_id = TenantId::new(&payload.tenant_id);

        info!(
            asset_count = payload.execution_plan.len(),
            "Executing workflow assets"
        );

        for (index, asset_id) in payload.execution_plan.iter().enumerate() {
            info!(
                asset_id = %asset_id,
                position = index,
                "Executing asset"
            );

            // Execute the asset and get output data
            let asset_output = self.execute_asset(*asset_id, &tenant_id).await?;

            // Run data quality checks for this asset with the output data
            let check_results = self
                .run_asset_checks(*asset_id, payload.execution_id, &tenant_id, &asset_output)
                .await?;

            // Check for blocking failures
            for result in &check_results {
                if result.blocking && result.outcome == "failed" {
                    let check_name = self.get_check_name(result.check_id, &tenant_id).await;
                    error!(
                        asset_id = %asset_id,
                        check_id = %result.check_id,
                        check_name = %check_name,
                        error_message = ?result.error_message,
                        "Blocking check failed - halting execution"
                    );
                    return Err(ExecutionError::BlockingCheckFailure {
                        asset_id: *asset_id,
                        check_name,
                        message: result
                            .error_message
                            .clone()
                            .unwrap_or_else(|| "Check failed".to_string()),
                    });
                }
            }

            info!(
                asset_id = %asset_id,
                checks_run = check_results.len(),
                "Asset execution completed"
            );
        }

        info!("All assets executed successfully");
        Ok(())
    }

    /// Execute an asset and return its output data
    ///
    /// In the full implementation, this would:
    /// 1. Fetch the asset definition from storage
    /// 2. Execute the asset's SQL/Python code
    /// 3. Return the resulting data for validation
    ///
    /// Currently returns sample data for development/testing.
    #[instrument(
        skip(self),
        fields(
            otel.name = "executor.execute_asset",
            asset_id = %asset_id,
            tenant_id = %tenant_id.as_str(),
        )
    )]
    async fn execute_asset(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<AssetOutput, ExecutionError> {
        // Fetch asset definition to understand what we're executing
        let asset = self
            .storage
            .get_asset(asset_id, tenant_id)
            .await
            .map_err(|e| ExecutionError::Storage(format!("Failed to fetch asset: {}", e)))?;

        debug!(
            asset_id = %asset_id,
            asset_name = %asset.name,
            asset_type = %asset.asset_type,
            "Executing asset"
        );

        // Simulate asset execution time
        tokio::time::sleep(Duration::from_millis(100)).await;

        // TODO: Implement actual asset execution
        // For now, return sample data that allows checks to actually execute.
        // This makes the DQ framework functional for testing while asset execution
        // is being developed.
        //
        // Future implementation should:
        // 1. Parse asset definition (SQL query, Python code, etc.)
        // 2. Execute against the data source
        // 3. Return the actual output rows

        // Generate sample data for demonstration - this allows checks to actually
        // execute and produce real pass/fail results
        let sample_data = self.generate_sample_asset_data(&asset.name);

        info!(
            asset_id = %asset_id,
            row_count = sample_data.row_count,
            "Asset execution completed (using sample data)"
        );

        Ok(sample_data)
    }

    /// Generate sample data for an asset (temporary until real execution is implemented)
    ///
    /// This allows the data quality checks to actually execute and produce real
    /// pass/fail results, demonstrating the check validation logic works correctly.
    fn generate_sample_asset_data(&self, _asset_name: &str) -> AssetOutput {
        use serde_json::json;

        // Sample data that will exercise various check types
        AssetOutput::from_rows(vec![
            json!({"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30, "status": "active"}),
            json!({"id": 2, "name": "Bob", "email": "bob@example.com", "age": 25, "status": "active"}),
            json!({"id": 3, "name": "Charlie", "email": "charlie@example.com", "age": 35, "status": "pending"}),
            json!({"id": 4, "name": "Diana", "email": "diana@example.com", "age": 28, "status": "active"}),
            json!({"id": 5, "name": "Eve", "email": "eve@example.com", "age": 32, "status": "inactive"}),
        ])
    }

    /// Run all data quality checks for an asset
    ///
    /// Fetches check definitions from storage, executes them against the asset
    /// output data, and stores the results.
    #[instrument(
        skip(self, asset_output),
        fields(
            otel.name = "executor.run_asset_checks",
            asset_id = %asset_id,
            execution_id = %execution_id,
            tenant_id = %tenant_id.as_str(),
        )
    )]
    async fn run_asset_checks(
        &self,
        asset_id: Uuid,
        execution_id: Uuid,
        tenant_id: &TenantId,
        asset_output: &AssetOutput,
    ) -> Result<Vec<CheckResultModel>, ExecutionError> {
        // Fetch enabled checks defined for this asset
        let checks = self
            .storage
            .list_asset_checks(asset_id, tenant_id, true) // enabled_only=true
            .await
            .map_err(|e| ExecutionError::Storage(format!("Failed to fetch checks: {}", e)))?;

        if checks.is_empty() {
            return Ok(vec![]);
        }

        info!(
            asset_id = %asset_id,
            check_count = checks.len(),
            "Running data quality checks"
        );

        let timer = Timer::start();
        let mut results = Vec::with_capacity(checks.len());

        for check in &checks {
            let check_timer = Timer::start();

            // Execute check against the asset output data
            let outcome = self.execute_check(check, asset_output);
            let duration_ms = (check_timer.elapsed_secs() * 1000.0) as i64;

            // Extract outcome fields (failed_samples is moved, so extract it first)
            let outcome_str = outcome.outcome_str().to_string();
            let failed_count = outcome.failed_count;
            let total_count = outcome.total_count;
            let error_message = outcome.error_message;

            // Apply PII filtering to failure samples
            let filtered_samples = self
                .pii_filter
                .redact_samples(outcome.failed_samples)
                .map(Json);

            let result = CheckResultModel {
                id: Uuid::new_v4(),
                check_id: check.id,
                execution_id,
                asset_id,
                outcome: outcome_str,
                severity: check.severity.clone(),
                blocking: check.blocking,
                failed_row_count: failed_count,
                total_row_count: total_count,
                error_message,
                failed_samples: filtered_samples,
                duration_ms,
                executed_at: chrono::Utc::now(),
                metadata: None,
                tenant_id: tenant_id.as_str().to_string().into(),
            };

            results.push(result);
        }

        // Batch insert check results
        if !results.is_empty() {
            self.storage
                .create_check_results_batch(&results, tenant_id)
                .await
                .map_err(|e| {
                    ExecutionError::Storage(format!("Failed to store check results: {}", e))
                })?;

            let duration_secs = timer.elapsed_secs();
            let passed = results.iter().filter(|r| r.outcome == "passed").count();
            let failed = results.iter().filter(|r| r.outcome == "failed").count();
            let skipped = results.iter().filter(|r| r.outcome == "skipped").count();
            record_check_execution(tenant_id.as_str(), passed, failed, skipped, duration_secs);
        }

        Ok(results)
    }

    /// Execute a single check against asset data and return the outcome
    ///
    /// This method validates asset output data against the check definition,
    /// producing real pass/fail results with row counts and failure samples.
    ///
    /// Supported check types:
    /// - not_null: Verify columns contain no null values
    /// - unique: Verify columns have unique values
    /// - in_range: Verify numeric values fall within bounds
    /// - regex: Verify string values match a pattern
    /// - row_count: Verify total row count is within bounds
    /// - accepted_values: Verify values are in an allowed set
    /// - no_duplicate_rows: Verify no duplicate rows exist
    fn execute_check(
        &self,
        check: &servo_storage::models::AssetCheckModel,
        asset_data: &AssetOutput,
    ) -> CheckOutcomeResult {
        let check_type = &check.check_type.0;

        debug!(
            check_name = %check.name,
            check_type = ?check_type,
            row_count = asset_data.row_count,
            "Executing check validation"
        );

        // Execute the check validation using the check_validator module
        let result = validate_check(check_type, asset_data);

        // Convert ValidationResult to CheckOutcomeResult
        if result.error_message.is_some() && result.failed_count.is_none() {
            // Check errored (couldn't execute)
            info!(
                check_name = %check.name,
                error = ?result.error_message,
                "Check could not execute"
            );
            CheckOutcomeResult {
                passed: false,
                skipped: true, // Mark as skipped since it couldn't actually validate
                failed_count: None,
                total_count: None,
                error_message: result.error_message,
                failed_samples: None,
            }
        } else if result.passed {
            info!(
                check_name = %check.name,
                total_count = ?result.total_count,
                "Check passed"
            );
            CheckOutcomeResult {
                passed: true,
                skipped: false,
                failed_count: result.failed_count,
                total_count: result.total_count,
                error_message: None,
                failed_samples: None,
            }
        } else {
            warn!(
                check_name = %check.name,
                failed_count = ?result.failed_count,
                total_count = ?result.total_count,
                blocking = check.blocking,
                "Check failed"
            );
            CheckOutcomeResult {
                passed: false,
                skipped: false,
                failed_count: result.failed_count,
                total_count: result.total_count,
                error_message: result.error_message,
                failed_samples: result.failed_samples,
            }
        }
    }

    /// Get check name by ID for error reporting
    async fn get_check_name(&self, check_id: Uuid, tenant_id: &TenantId) -> String {
        match self.storage.get_asset_check(check_id, tenant_id).await {
            Ok(check) => check.name,
            Err(_) => format!("check-{}", check_id),
        }
    }
}

/// Result of executing a data quality check
struct CheckOutcomeResult {
    passed: bool,
    skipped: bool,
    failed_count: Option<i64>,
    total_count: Option<i64>,
    error_message: Option<String>,
    /// Sample of rows that failed the check (for debugging).
    /// Will be PII-filtered before storage.
    failed_samples: Option<serde_json::Value>,
}

impl CheckOutcomeResult {
    fn outcome_str(&self) -> &'static str {
        if self.skipped {
            "skipped"
        } else if self.passed {
            "passed"
        } else {
            "failed"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_executor_creation() {
        // This test requires a real database connection
        // In practice, you'd use a test database or mock
        let _timeout = Duration::from_secs(600);
        // let storage = Arc::new(PostgresStorage::new("postgresql://test").await.unwrap());
        // let executor = WorkflowExecutor::new(storage, _timeout);
        // assert_eq!(executor.timeout, _timeout);
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
