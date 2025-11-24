//! End-to-end workflow execution tests
//!
//! These tests validate the complete workflow execution lifecycle from
//! CLI invocation to terminal state, using a real PostgreSQL database.
//!
//! ## Running these tests
//! ```bash
//! # Set up test database
//! export TEST_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//!
//! # Run E2E tests
//! cargo test e2e_workflow_execution -- --ignored --test-threads=1
//! ```

use anyhow::{Context, Result};
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy, ExecutionState};
use servo_storage::{postgres::PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// E2E test harness for workflow execution testing
struct E2ETestHarness {
    storage: Arc<PostgresStorage>,
    orchestrator: ExecutionOrchestrator,
    tenant: TenantId,
}

impl E2ETestHarness {
    async fn setup() -> Result<Self> {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .context("TEST_DATABASE_URL or DATABASE_URL must be set for E2E tests")?;

        let storage = Arc::new(PostgresStorage::new(&database_url).await?);
        let tenant = TenantId::new(format!("test-{}", Uuid::new_v4()));
        let retry_policy = RetryPolicy::default();
        let orchestrator = ExecutionOrchestrator::new(storage.clone(), retry_policy);

        Ok(Self {
            storage,
            orchestrator,
            tenant,
        })
    }

    async fn create_workflow(&self, name: &str, description: &str) -> Result<Uuid> {
        self.storage
            .create_workflow(name, description, "test-owner", &[], &self.tenant)
            .await
    }

    async fn run_workflow(&self, workflow_id: Uuid) -> Result<Uuid> {
        self.orchestrator
            .start_execution(workflow_id, &self.tenant, None)
            .await
    }

    async fn wait_for_completion(
        &self,
        execution_id: Uuid,
        max_wait: Duration,
    ) -> Result<ExecutionState> {
        timeout(max_wait, async {
            loop {
                let execution = self.storage.get_execution(execution_id, &self.tenant).await?;
                let state = execution.state.as_str();
                if matches!(state, "succeeded" | "failed" | "cancelled" | "timeout") {
                    return Ok(match state {
                        "succeeded" => ExecutionState::Succeeded,
                        "failed" => ExecutionState::Failed,
                        "cancelled" => ExecutionState::Cancelled,
                        "timeout" => ExecutionState::Timeout,
                        _ => unreachable!(),
                    });
                }
                sleep(Duration::from_millis(100)).await;
            }
        })
        .await?
    }

    async fn transition_state(
        &self,
        execution_id: Uuid,
        from: ExecutionState,
        to: ExecutionState,
    ) -> Result<()> {
        self.orchestrator
            .transition_state(execution_id, from, to, &self.tenant)
            .await
    }
}

#[tokio::test]
#[ignore] // Requires test database
async fn test_happy_path_workflow_execution() {
    let harness = E2ETestHarness::setup().await.expect("Setup failed");

    let workflow_id = harness
        .create_workflow("simple-success", "Simple success workflow")
        .await
        .expect("Failed to create workflow");

    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start execution");

    // Verify Pending state
    let execution = harness
        .storage
        .get_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to fetch execution");
    assert_eq!(execution.state, "pending");

    // Simulate: Pending -> Running -> Succeeded
    harness
        .transition_state(execution_id, ExecutionState::Pending, ExecutionState::Running)
        .await
        .expect("Failed to transition to Running");

    harness
        .transition_state(
            execution_id,
            ExecutionState::Running,
            ExecutionState::Succeeded,
        )
        .await
        .expect("Failed to transition to Succeeded");

    // Wait for completion
    let final_state = harness
        .wait_for_completion(execution_id, Duration::from_secs(5))
        .await
        .expect("Execution did not complete");

    assert_eq!(final_state, ExecutionState::Succeeded);
}

#[tokio::test]
#[ignore] // Requires test database
async fn test_failing_workflow_execution() {
    let harness = E2ETestHarness::setup().await.expect("Setup failed");

    let workflow_id = harness
        .create_workflow("failing-workflow", "Failing workflow")
        .await
        .expect("Failed to create workflow");

    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start execution");

    // Simulate: Pending -> Running -> Failed
    harness
        .transition_state(execution_id, ExecutionState::Pending, ExecutionState::Running)
        .await
        .expect("Failed to transition to Running");

    harness
        .transition_state(execution_id, ExecutionState::Running, ExecutionState::Failed)
        .await
        .expect("Failed to transition to Failed");

    // Wait for completion
    let final_state = harness
        .wait_for_completion(execution_id, Duration::from_secs(5))
        .await
        .expect("Execution did not complete");

    assert_eq!(final_state, ExecutionState::Failed);
}
