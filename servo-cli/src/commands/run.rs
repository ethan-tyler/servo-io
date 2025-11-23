//! Run command

use anyhow::{Context, Result};
use servo_cloud_gcp::{auth::GcpAuth, config::GcpConfig, queue::CloudTasksQueue};
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy};
use servo_storage::{postgres::PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

use crate::polling;

pub async fn execute(
    workflow_name: &str,
    params: Option<&str>,
    wait: bool,
    timeout_secs: u64,
    poll_interval_secs: u64,
    database_url: &str,
) -> Result<()> {
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

    // Create orchestrator with optional task enqueuer
    let retry_policy = RetryPolicy::default();
    let orchestrator = match load_gcp_enqueuer().await {
        Ok(enqueuer) => {
            info!("GCP task enqueuer configured");
            ExecutionOrchestrator::with_enqueuer(storage.clone(), retry_policy, enqueuer)
        }
        Err(e) => {
            warn!("GCP task enqueuer not configured: {}", e);
            warn!("Execution will be created but not enqueued to Cloud Tasks");
            ExecutionOrchestrator::new(storage.clone(), retry_policy)
        }
    };

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
                use servo_runtime::ExecutionState;
                match final_state {
                    ExecutionState::Succeeded => {
                        info!("✅ Execution completed successfully");
                        std::process::exit(0);
                    }
                    ExecutionState::Failed => {
                        warn!("❌ Execution failed");
                        std::process::exit(1);
                    }
                    ExecutionState::Timeout => {
                        warn!("⏱️  Execution timed out");
                        std::process::exit(1);
                    }
                    ExecutionState::Cancelled => {
                        warn!("🚫 Execution was cancelled");
                        std::process::exit(1);
                    }
                    _ => {
                        warn!("⚠️  Execution ended in unexpected state: {:?}", final_state);
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to wait for execution: {}", e);
                info!("Execution ID: {}", execution_id);
                info!("Use 'servo status {}' to check execution status", execution_id);
                std::process::exit(1);
            }
        }
    } else {
        // Return execution ID for user to track
        println!("{}", execution_id);
        Ok(())
    }
}

/// Load GCP configuration and create task enqueuer
async fn load_gcp_enqueuer() -> Result<Arc<dyn servo_runtime::task_enqueuer::TaskEnqueuer>> {
    // Load GCP configuration from environment
    let gcp_config = GcpConfig::from_env()
        .context("Failed to load GCP configuration from environment")?;

    // Create GCP auth from service account JSON
    let gcp_auth = Arc::new(
        GcpAuth::from_service_account_json(&gcp_config.service_account_key_json)
            .context("Failed to create GCP authentication")?,
    );

    // Create Cloud Tasks queue enqueuer
    let enqueuer = Arc::new(CloudTasksQueue::from_config(gcp_config, gcp_auth)?);

    Ok(enqueuer as Arc<dyn servo_runtime::task_enqueuer::TaskEnqueuer>)
}
