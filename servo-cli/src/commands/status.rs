//! Status command

use anyhow::Result;
use servo_runtime::ExecutionState;
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;

/// Check the status of an execution.
///
/// Looks up the execution by ID for the given tenant and prints basic details.
/// In production, this would likely return structured JSON; here we log to stdout.
pub async fn execute(execution_id: &str, tenant_id: &str, database_url: &str) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);
    let exec_id = uuid::Uuid::parse_str(execution_id)?;

    let execution = storage.get_execution(exec_id, &tenant).await?;

    let state: ExecutionState = execution.state.as_str().try_into()?;

    println!("Execution: {}", execution.id);
    println!("Workflow: {}", execution.workflow_id);
    println!("Tenant: {}", tenant_id);
    println!("State: {:?}", state);
    if let Some(started) = execution.started_at {
        println!("Started: {}", started);
    }
    if let Some(completed) = execution.completed_at {
        println!("Completed: {}", completed);
    }
    if let Some(err) = execution.error_message {
        println!("Error: {}", err);
    }

    Ok(())
}
