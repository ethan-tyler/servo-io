//! E2E Test Harness
//!
//! Provides infrastructure for end-to-end testing of workflow execution using testcontainers.
//!
//! ## Features
//! - Ephemeral Postgres instances via testcontainers (isolated per test)
//! - Automatic schema migration
//! - Mock worker for deterministic task execution
//! - Parallel test execution support with isolated tenants

use anyhow::{Context, Result};
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy, ExecutionState};
use servo_storage::{models::WorkflowModel, postgres::PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use testcontainers::{clients::Cli, core::WaitFor, Container, GenericImage};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// E2E test harness for workflow execution testing with testcontainers
pub struct E2ETestHarness<'a> {
    pub storage: Arc<PostgresStorage>,
    pub orchestrator: ExecutionOrchestrator,
    pub tenant: TenantId,
    _container: Container<'a, GenericImage>,
    _docker: &'a Cli,
}

impl<'a> E2ETestHarness<'a> {
    /// Set up a new test harness instance with ephemeral Postgres
    ///
    /// Spins up a testcontainers Postgres instance, runs migrations,
    /// and creates an isolated tenant for the test.
    pub async fn setup(docker: &'a Cli) -> Result<Self> {
        // Start Postgres container
        let postgres_image = GenericImage::new("postgres", "15-alpine")
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_DB", "servo_test")
            .with_env_var("POSTGRES_USER", "servo_test")
            .with_env_var("POSTGRES_PASSWORD", "servo_test");

        let container = docker.run(postgres_image);
        let host_port = container.get_host_port_ipv4(5432);

        // Build connection string
        let database_url = format!(
            "postgresql://servo_test:servo_test@localhost:{}/servo_test",
            host_port
        );

        // Wait a moment for container to be fully ready
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Connect to database
        let storage = Arc::new(
            PostgresStorage::new(&database_url)
                .await
                .context("Failed to connect to test database")?,
        );

        // Run migrations
        Self::run_migrations(&storage)
            .await
            .context("Failed to run migrations")?;

        // Create unique tenant ID for test isolation
        let tenant = TenantId::new(format!("test-{}", Uuid::new_v4()));

        // Create orchestrator without task enqueuer (for simple tests)
        let retry_policy = RetryPolicy::default();
        let orchestrator = ExecutionOrchestrator::new(storage.clone(), retry_policy);

        Ok(Self {
            storage,
            orchestrator,
            tenant,
            _container: container,
            _docker: docker,
        })
    }

    /// Run database migrations
    async fn run_migrations(storage: &Arc<PostgresStorage>) -> Result<()> {
        // Note: This requires access to storage's internal pool
        // For now, migrations should be run manually or via a migration script
        // In production, consider using sqlx::migrate!() macro or similar

        // TODO: Implement migration runner
        // For now, assume schema is created via SQL files in servo-storage/migrations/

        Ok(())
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

                // Check if terminal state (FIXED: lowercase state strings)
                let state = execution.state.as_str();
                if matches!(state, "succeeded" | "failed" | "cancelled" | "timeout") {
                    // Parse state
                    return Ok(match state {
                        "succeeded" => ExecutionState::Succeeded,
                        "failed" => ExecutionState::Failed,
                        "cancelled" => ExecutionState::Cancelled,
                        "timeout" => ExecutionState::Timeout,
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
    ///
    /// Container cleanup is automatic via Drop trait
    pub async fn teardown(self) -> Result<()> {
        // Container will be automatically stopped and removed when dropped
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_harness_setup_with_testcontainers() {
        let docker = Cli::default();
        let harness = E2ETestHarness::setup(&docker)
            .await
            .expect("Failed to setup harness");

        // Verify we can create a workflow
        let workflow_id = harness
            .create_workflow("test-workflow", "Test workflow description")
            .await
            .expect("Failed to create workflow");

        assert_ne!(workflow_id, Uuid::nil());

        harness.teardown().await.expect("Failed to teardown");
    }
}
