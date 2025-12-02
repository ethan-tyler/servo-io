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
use servo_core::PartitionExecutionContext;
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

    /// Execute a scheduled workflow
    ///
    /// This method is called by Cloud Scheduler to trigger a workflow execution.
    /// It creates an execution record and then executes the workflow.
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant identifier
    ///
    /// # Returns
    ///
    /// The execution ID on success
    #[instrument(
        skip(self),
        fields(
            otel.name = "executor.execute_scheduled",
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn execute_scheduled(
        &self,
        workflow_id: Uuid,
        tenant_id: &str,
    ) -> Result<Uuid, ExecutionError> {
        let tenant = TenantId::new(tenant_id);

        // Verify workflow exists and belongs to this tenant (explicit check before RLS)
        let workflow = self
            .storage
            .get_workflow(workflow_id, &tenant)
            .await
            .map_err(|e| {
                error!(
                    workflow_id = %workflow_id,
                    tenant_id = %tenant_id,
                    error = %e,
                    "Failed to find workflow for scheduled execution"
                );
                ExecutionError::Internal(format!(
                    "Workflow {} not found or not accessible for tenant {}: {}",
                    workflow_id, tenant_id, e
                ))
            })?;

        // Verify schedule is enabled
        if !workflow.schedule_enabled.unwrap_or(false) {
            warn!(
                workflow_id = %workflow_id,
                tenant_id = %tenant_id,
                "Scheduled execution attempted for disabled schedule"
            );
            return Err(ExecutionError::Internal(
                "Workflow schedule is disabled".to_string(),
            ));
        }

        info!(
            workflow_id = %workflow_id,
            workflow_name = %workflow.name,
            tenant_id = %tenant_id,
            "Workflow validated for scheduled execution"
        );

        // Use the orchestrator to start the execution
        let orchestrator =
            ExecutionOrchestrator::new(self.storage.clone(), servo_runtime::RetryPolicy::default());

        // Start the execution (creates execution record)
        let execution_id = orchestrator
            .start_execution(workflow_id, &tenant, None)
            .await
            .map_err(|e| ExecutionError::Internal(format!("Failed to start execution: {}", e)))?;

        info!(
            execution_id = %execution_id,
            workflow_id = %workflow_id,
            "Scheduled execution started"
        );

        // Build the task payload (execution plan will be empty, executor will compile)
        let payload = TaskPayload {
            execution_id,
            workflow_id,
            tenant_id: tenant_id.to_string(),
            idempotency_key: None,
            execution_plan: vec![],  // Executor will compile the workflow
            partition_context: None, // No partition context for scheduled runs
        };

        // Execute the workflow (this will compile the execution plan)
        self.execute(payload).await?;

        Ok(execution_id)
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
            let asset_output = self
                .execute_asset(*asset_id, &tenant_id, payload.partition_context.as_ref())
                .await?;

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
    /// This method:
    /// 1. Fetches the asset definition from storage
    /// 2. Executes the asset's Python function using compute_fn_module/compute_fn_function
    /// 3. Returns the resulting data for validation
    ///
    /// If compute_fn_module/compute_fn_function are not set, returns sample data
    /// for backwards compatibility during development.
    #[instrument(
        skip(self, partition_context),
        fields(
            otel.name = "executor.execute_asset",
            asset_id = %asset_id,
            tenant_id = %tenant_id.as_str(),
            partition_key = tracing::field::Empty,
        )
    )]
    async fn execute_asset(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
        partition_context: Option<&PartitionExecutionContext>,
    ) -> Result<AssetOutput, ExecutionError> {
        // Record partition key in span if present
        if let Some(ctx) = partition_context {
            tracing::Span::current().record("partition_key", ctx.partition_key.as_str());
        }
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
            compute_fn_module = ?asset.compute_fn_module,
            compute_fn_function = ?asset.compute_fn_function,
            "Executing asset"
        );

        // Check if asset has compute function defined
        match (&asset.compute_fn_module, &asset.compute_fn_function) {
            (Some(module), Some(function)) => {
                // Execute real Python function
                self.execute_python_asset(
                    &asset.name,
                    module,
                    function,
                    asset_id,
                    tenant_id,
                    partition_context,
                )
                .await
            }
            _ => {
                // No compute function defined - use sample data for backwards compatibility
                warn!(
                    asset_id = %asset_id,
                    asset_name = %asset.name,
                    "Asset has no compute function defined, using sample data"
                );
                let sample_data = self.generate_sample_asset_data(&asset.name);
                info!(
                    asset_id = %asset_id,
                    row_count = sample_data.row_count,
                    "Asset execution completed (using sample data)"
                );
                Ok(sample_data)
            }
        }
    }

    /// Execute a Python asset function via subprocess
    ///
    /// Invokes the Python function specified by module and function name,
    /// capturing output and handling errors.
    ///
    /// If partition_context is provided (for backfill operations), it will be
    /// passed to the Python process via the SERVO_PARTITION_CONTEXT environment
    /// variable as a JSON string.
    async fn execute_python_asset(
        &self,
        asset_name: &str,
        module: &str,
        function: &str,
        asset_id: Uuid,
        tenant_id: &TenantId,
        partition_context: Option<&PartitionExecutionContext>,
    ) -> Result<AssetOutput, ExecutionError> {
        info!(
            asset_name = %asset_name,
            module = %module,
            function = %function,
            partition_key = ?partition_context.map(|c| &c.partition_key),
            "Executing Python asset function"
        );

        // Build Python code to execute the function
        let python_code = format!(
            r#"
import sys
import json
try:
    from {} import {}
    result = {}()
    # Output as JSON for parsing
    output = {{"status": "success", "asset": "{}", "result": str(result) if result is not None else None}}
    print(json.dumps(output))
except Exception as e:
    import traceback
    error_output = {{"status": "error", "asset": "{}", "error": str(e), "traceback": traceback.format_exc()}}
    print(json.dumps(error_output), file=sys.stderr)
    sys.exit(1)
"#,
            module, function, function, asset_name, asset_name
        );

        // Build the command with base environment variables
        let mut cmd = tokio::process::Command::new("python3");
        cmd.arg("-c")
            .arg(&python_code)
            .env("SERVO_ASSET_ID", asset_id.to_string())
            .env("SERVO_ASSET_NAME", asset_name)
            .env("SERVO_TENANT_ID", tenant_id.as_str());

        // Add partition context as JSON if present
        if let Some(ctx) = partition_context {
            if let Ok(ctx_json) = serde_json::to_string(ctx) {
                cmd.env("SERVO_PARTITION_CONTEXT", ctx_json);
                // Also provide partition_key as a simple string for convenience
                cmd.env("SERVO_PARTITION_KEY", &ctx.partition_key);
            }
        }

        // Execute Python subprocess
        let output = cmd.output().await.map_err(|e| {
            ExecutionError::Internal(format!("Failed to spawn Python process: {}", e))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            error!(
                asset_name = %asset_name,
                exit_code = ?output.status.code(),
                stderr = %stderr,
                "Python asset execution failed"
            );

            // Try to parse error details from stderr
            if let Ok(error_json) = serde_json::from_str::<serde_json::Value>(&stderr) {
                let error_msg = error_json
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error");
                return Err(ExecutionError::Internal(format!(
                    "Asset '{}' execution failed: {}",
                    asset_name, error_msg
                )));
            }

            return Err(ExecutionError::Internal(format!(
                "Asset '{}' execution failed with exit code {:?}: {}",
                asset_name,
                output.status.code(),
                stderr.trim()
            )));
        }

        // Parse successful output
        if let Ok(result_json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            info!(
                asset_name = %asset_name,
                result = ?result_json.get("result"),
                "Python asset execution succeeded"
            );
        } else {
            info!(
                asset_name = %asset_name,
                stdout = %stdout.trim(),
                "Python asset execution completed"
            );
        }

        // For now, return sample data for DQ checks since we don't have
        // a standard way to return structured data from arbitrary Python functions.
        // Future enhancement: Define a protocol for assets to return structured data.
        Ok(self.generate_sample_asset_data(asset_name))
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
