//! E2E Test Harness
//!
//! Provides infrastructure for end-to-end testing of workflow execution.
//!
//! ## Requirements
//! - PostgreSQL test database (set via TEST_DATABASE_URL environment variable)
//! - Database must be initialized with schema and RLS policies
//!
//! ## Future Enhancements
//! - Use testcontainers to spin up ephemeral Postgres instances
//! - Add mock worker for deterministic task execution
//! - Support parallel test execution with isolated tenants

use anyhow::{Context, Result};
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy, ExecutionState};
use servo_storage::{models::WorkflowModel, postgres::PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// E2E test harness for workflow execution testing
pub struct E2ETestHarness {
    pub storage: Arc<PostgresStorage>,
    pub orchestrator: ExecutionOrchestrator,
    pub tenant: TenantId,
}

impl E2ETestHarness {
    /// Set up a new test harness instance
    ///
    /// Requires TEST_DATABASE_URL environment variable to be set.
    /// Uses a unique tenant ID for each test to ensure isolation.
    pub async fn setup() -> Result<Self> {
        // Get test database URL from environment
        let database_url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .context("TEST_DATABASE_URL or DATABASE_URL must be set for E2E tests")?;

        // Connect to database
        let storage = Arc::new(
            PostgresStorage::new(&database_url)
                .await
                .context("Failed to connect to test database")?,
        );

        // Create unique tenant ID for test isolation
        let tenant = TenantId::new(format!("test-{}", Uuid::new_v4()));

        // Create orchestrator without task enqueuer (for simple tests)
        let retry_policy = RetryPolicy::default();
        let orchestrator = ExecutionOrchestrator::new(storage.clone(), retry_policy);

        Ok(Self {
            storage,
            orchestrator,
            tenant,
        })
    }

    /// Create a workflow in the database
    pub async fn create_workflow(&self, name: &str, description: &str) -> Result<Uuid> {
        let workflow_id = self
            .storage
            .create_workflow(name, description, "test-owner", &[], &self.tenant)
            .await
            .context("Failed to create workflow")?;

        Ok(workflow_id)
    }

    /// Start a workflow execution
    pub async fn run_workflow(&self, workflow_id: Uuid) -> Result<Uuid> {
        let execution_id = self
            .orchestrator
            .start_execution(workflow_id, &self.tenant, None)
            .await
            .context("Failed to start execution")?;

        Ok(execution_id)
    }

    /// Wait for execution to reach a terminal state
    ///
    /// Polls the execution status until it reaches a terminal state or times out.
    pub async fn wait_for_completion(
        &self,
        execution_id: Uuid,
        max_wait: Duration,
    ) -> Result<ExecutionState> {
        let poll_interval = Duration::from_millis(100);

        timeout(max_wait, async {
            loop {
                let execution = self
                    .storage
                    .get_execution(execution_id, &self.tenant)
                    .await?;

                // Check if terminal state
                let state = execution.state.as_str();
                if matches!(state, "Succeeded" | "Failed" | "Cancelled" | "Timeout") {
                    // Parse state
                    return Ok(match state {
                        "Succeeded" => ExecutionState::Succeeded,
                        "Failed" => ExecutionState::Failed,
                        "Cancelled" => ExecutionState::Cancelled,
                        "Timeout" => ExecutionState::Timeout,
                        _ => unreachable!(),
                    });
                }

                sleep(poll_interval).await;
            }
        })
        .await
        .context("Execution did not complete within timeout")?
    }

    /// Manually transition execution state (for testing)
    ///
    /// This is a test helper that directly transitions execution state.
    pub async fn transition_execution_state(
        &self,
        execution_id: Uuid,
        from_state: ExecutionState,
        to_state: ExecutionState,
    ) -> Result<()> {
        self.orchestrator
            .transition_state(execution_id, from_state, to_state, &self.tenant)
            .await
            .context("Failed to transition state")?;

        Ok(())
    }

    /// Clean up test resources
    pub async fn teardown(self) -> Result<()> {
        // In a real implementation with testcontainers, we would:
        // 1. Stop and remove the container
        // 2. Clean up any temporary resources
        //
        // For now, with shared test database, tenant isolation is sufficient
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires test database
    async fn test_harness_setup() {
        let harness = E2ETestHarness::setup().await.expect("Failed to setup harness");

        // Verify we can create a workflow
        let workflow_id = harness
            .create_workflow("test-workflow", "Test workflow description")
            .await
            .expect("Failed to create workflow");

        assert_ne!(workflow_id, Uuid::nil());

        harness.teardown().await.expect("Failed to teardown");
    }
}
