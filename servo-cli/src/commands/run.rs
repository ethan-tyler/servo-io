//! Run command

use anyhow::{Context, Result};
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy, ExecutionState};
use servo_storage::{postgres::PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

use crate::polling;

/// Execution status returned by the run command
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Execution completed successfully
    Succeeded(Uuid),
    /// Execution failed
    Failed(Uuid),
    /// Execution timed out
    Timeout(Uuid),
    /// Execution was cancelled
    Cancelled(Uuid),
    /// Execution started asynchronously (--wait not used)
    AsyncStarted(Uuid),
}

pub async fn execute(
    workflow_name: &str,
    params: Option<&str>,
    wait: bool,
    timeout_secs: u64,
    poll_interval_secs: u64,
    database_url: &str,
) -> Result<ExecutionStatus> {
    info!("Running workflow: {}", workflow_name);

    if let Some(params) = params {
        info!("With parameters: {}", params);
        warn!("Parameters are not yet supported - will be ignored");
    }

    // Parse workflow_name as UUID
    // TODO: Add support for workflow name lookup
    let workflow_id = workflow_name
        .parse::<Uuid>()
        .context("workflow_name must be a valid UUID. Name lookup not yet implemented.")?;

    // Get tenant ID from environment
    let tenant_id = std::env::var("TENANT_ID")
        .context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(tenant_id);

    // Create storage
    info!("Connecting to database...");
    let storage = Arc::new(PostgresStorage::new(database_url).await?);

    // Check if workflow exists
    info!("Looking up workflow {}...", workflow_id);
    let _workflow = storage
        .get_workflow(workflow_id, &tenant_id)
        .await
        .context("Workflow not found")?;
    info!("Workflow found: {}", _workflow.name);

    // Create orchestrator
    let retry_policy = RetryPolicy::default();
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), retry_policy);

    // Start execution
    info!("Starting execution...");
    let execution_id = orchestrator
        .start_execution(workflow_id, &tenant_id, None)
        .await
        .context("Failed to start execution")?;

    info!("✅ Execution started: {}", execution_id);

    // If --wait flag is set, poll until completion
    if wait {
        info!("Waiting for execution to complete...");
        let timeout_duration = Duration::from_secs(timeout_secs);
        let poll_interval_duration = Duration::from_secs(poll_interval_secs);

        match polling::poll_execution_status(
            execution_id,
            &tenant_id,
            &storage,
            timeout_duration,
            poll_interval_duration,
        )
        .await
        {
            Ok(final_state) => {
                match final_state {
                    ExecutionState::Succeeded => {
                        info!("✅ Execution completed successfully");
                        Ok(ExecutionStatus::Succeeded(execution_id))
                    }
                    ExecutionState::Failed => {
                        warn!("❌ Execution failed");
                        Ok(ExecutionStatus::Failed(execution_id))
                    }
                    ExecutionState::Timeout => {
                        warn!("⏱️  Execution timed out");
                        Ok(ExecutionStatus::Timeout(execution_id))
                    }
                    ExecutionState::Cancelled => {
                        warn!("🚫 Execution was cancelled");
                        Ok(ExecutionStatus::Cancelled(execution_id))
                    }
                    _ => {
                        warn!("⚠️  Execution ended in unexpected state: {:?}", final_state);
                        Ok(ExecutionStatus::Failed(execution_id))
                    }
                }
            }
            Err(e) => {
                warn!("Failed to wait for execution: {}", e);
                info!("Execution ID: {}", execution_id);
                info!("Use 'servo status {}' to check execution status", execution_id);
                Err(e)
            }
        }
    } else {
        // Return execution ID for user to track
        println!("{}", execution_id);
        Ok(ExecutionStatus::AsyncStarted(execution_id))
    }
}
