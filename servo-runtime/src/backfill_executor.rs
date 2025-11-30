//! Backfill executor for processing partition backfill jobs
//!
//! The BackfillExecutor polls for pending backfill jobs and processes them
//! by executing each partition sequentially. It handles state transitions,
//! progress tracking, and error handling.

use crate::orchestrator::ExecutionOrchestrator;
use crate::Result;
use servo_storage::{BackfillJobModel, BackfillPartitionModel, PostgresStorage, TenantId};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

/// Configuration for the backfill executor
#[derive(Debug, Clone)]
pub struct BackfillExecutorConfig {
    /// How often to poll for new jobs (default: 5 seconds)
    pub poll_interval: Duration,
    /// Maximum number of concurrent jobs to process (default: 1)
    pub max_concurrent_jobs: usize,
    /// Heartbeat interval for running jobs (default: 30 seconds)
    pub heartbeat_interval: Duration,
    /// Maximum retries per partition (default: 3)
    pub max_partition_retries: i32,
    /// Delay between partition executions (default: 100ms)
    pub partition_delay: Duration,
}

impl Default for BackfillExecutorConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            max_concurrent_jobs: 1,
            heartbeat_interval: Duration::from_secs(30),
            max_partition_retries: 3,
            partition_delay: Duration::from_millis(100),
        }
    }
}

/// Backfill executor for processing partition backfill jobs
pub struct BackfillExecutor {
    storage: Arc<PostgresStorage>,
    orchestrator: ExecutionOrchestrator,
    config: BackfillExecutorConfig,
    tenant_id: TenantId,
}

impl BackfillExecutor {
    /// Create a new backfill executor
    pub fn new(
        storage: Arc<PostgresStorage>,
        orchestrator: ExecutionOrchestrator,
        config: BackfillExecutorConfig,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            storage,
            orchestrator,
            config,
            tenant_id,
        }
    }

    /// Start the executor loop (runs until cancelled)
    #[instrument(skip(self), fields(tenant_id = %self.tenant_id.as_str()))]
    pub async fn run(&self) -> Result<()> {
        info!("Starting backfill executor");
        let mut poll_interval = interval(self.config.poll_interval);

        loop {
            poll_interval.tick().await;

            match self.poll_and_process_jobs().await {
                Ok(processed) => {
                    if processed > 0 {
                        info!("Processed {} backfill jobs", processed);
                    }
                }
                Err(e) => {
                    error!("Error polling backfill jobs: {}", e);
                }
            }
        }
    }

    /// Poll for pending jobs and process them
    async fn poll_and_process_jobs(&self) -> Result<usize> {
        // Get pending jobs that need processing
        let pending_jobs = self
            .storage
            .list_backfill_jobs(&self.tenant_id, Some("pending"), 10, 0)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        let mut processed = 0;
        for job in pending_jobs.into_iter().take(self.config.max_concurrent_jobs) {
            match self.process_job(job).await {
                Ok(_) => processed += 1,
                Err(e) => {
                    error!("Failed to process backfill job: {}", e);
                }
            }
        }

        Ok(processed)
    }

    /// Process a single backfill job
    #[instrument(skip(self), fields(job_id = %job.id, asset = %job.asset_name))]
    async fn process_job(&self, job: BackfillJobModel) -> Result<()> {
        info!("Processing backfill job {} for asset {}", job.id, job.asset_name);

        // Transition job to running state
        self.storage
            .transition_backfill_job_state(job.id, "pending", "running", None, &self.tenant_id)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        // Get all partitions for this job
        let partitions = self
            .storage
            .list_backfill_partitions(job.id, &self.tenant_id)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        let total_partitions = partitions.len();
        let mut completed_count = 0;
        let mut failed_count = 0;
        let mut skipped_count = 0;

        // Process each partition sequentially
        for partition in partitions {
            // Skip already completed/failed partitions (for resume scenarios)
            if partition.state == "completed" {
                completed_count += 1;
                continue;
            }
            if partition.state == "failed" && partition.attempt_count >= self.config.max_partition_retries {
                failed_count += 1;
                continue;
            }
            if partition.state == "skipped" {
                skipped_count += 1;
                continue;
            }

            // Small delay between partitions to avoid overwhelming the system
            if completed_count > 0 || failed_count > 0 {
                tokio::time::sleep(self.config.partition_delay).await;
            }

            // Update heartbeat
            self.update_heartbeat(job.id).await?;

            // Execute the partition
            match self.execute_partition(&job, &partition).await {
                Ok(_) => {
                    completed_count += 1;
                    info!(
                        "Partition {} completed ({}/{})",
                        partition.partition_key,
                        completed_count,
                        total_partitions
                    );
                }
                Err(e) => {
                    failed_count += 1;
                    warn!(
                        "Partition {} failed: {} ({} failures so far)",
                        partition.partition_key, e, failed_count
                    );
                }
            }

            // Update job progress
            self.update_job_progress(job.id, completed_count, failed_count, skipped_count)
                .await?;
        }

        // Determine final job state
        let final_state = if failed_count > 0 && completed_count == 0 {
            "failed"
        } else if failed_count > 0 {
            // Some succeeded, some failed - mark as completed with errors in message
            "completed"
        } else {
            "completed"
        };

        let error_message = if failed_count > 0 {
            Some(format!("{} of {} partitions failed", failed_count, total_partitions))
        } else {
            None
        };

        // Transition to final state
        self.storage
            .transition_backfill_job_state(
                job.id,
                "running",
                final_state,
                error_message.as_deref(),
                &self.tenant_id,
            )
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        info!(
            "Backfill job {} finished: {} completed, {} failed, {} skipped",
            job.id, completed_count, failed_count, skipped_count
        );

        Ok(())
    }

    /// Execute a single partition
    #[instrument(skip(self, job), fields(partition_key = %partition.partition_key))]
    async fn execute_partition(
        &self,
        job: &BackfillJobModel,
        partition: &BackfillPartitionModel,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        // Transition partition to running
        self.storage
            .transition_backfill_partition_state(
                partition.id,
                &partition.state,
                "running",
                None,
                &self.tenant_id,
            )
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        // Execute the asset for this partition
        // This creates and runs an execution for the asset with the partition key as context
        let execution_result = self
            .execute_asset_partition(job.asset_id, &partition.partition_key)
            .await;

        let duration_ms = start.elapsed().as_millis() as i64;

        match execution_result {
            Ok(execution_id) => {
                // Mark partition as completed
                self.storage
                    .complete_backfill_partition(
                        partition.id,
                        Some(execution_id),
                        duration_ms,
                        &self.tenant_id,
                    )
                    .await
                    .map_err(|e| crate::Error::Internal(e.to_string()))?;
                Ok(())
            }
            Err(e) => {
                // Mark partition as failed
                self.storage
                    .fail_backfill_partition(
                        partition.id,
                        &e.to_string(),
                        duration_ms,
                        &self.tenant_id,
                    )
                    .await
                    .map_err(|e| crate::Error::Internal(e.to_string()))?;
                Err(e)
            }
        }
    }

    /// Execute an asset for a specific partition
    async fn execute_asset_partition(
        &self,
        asset_id: Uuid,
        partition_key: &str,
    ) -> Result<Uuid> {
        // Look up the workflow associated with this asset
        // For now, we assume the asset name maps to a workflow name
        // In a full implementation, this would query the asset's associated workflow

        debug!(
            "Executing asset {} for partition {}",
            asset_id, partition_key
        );

        // Get the asset to find its workflow
        let asset = self
            .storage
            .get_asset(asset_id, &self.tenant_id)
            .await
            .map_err(|e| crate::Error::NotFound(format!("Asset not found: {}", e)))?;

        // Look up workflow by asset name (convention: asset name = workflow name)
        let workflows = self
            .storage
            .list_workflows(&self.tenant_id, 100, 0)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))?;

        let workflow = workflows
            .iter()
            .find(|w| w.name == asset.name)
            .ok_or_else(|| {
                crate::Error::NotFound(format!("No workflow found for asset '{}'", asset.name))
            })?;

        // Start execution with partition context
        let execution_id = self
            .orchestrator
            .start_execution(
                workflow.id,
                &self.tenant_id,
                Some(format!("backfill:{}", partition_key)),
            )
            .await
            .map_err(|e| crate::Error::Execution(e.to_string()))?;

        // Wait for execution to complete (with timeout)
        let timeout = Duration::from_secs(300); // 5 minute timeout per partition
        let result = self
            .wait_for_execution(execution_id, timeout)
            .await?;

        if result {
            Ok(execution_id)
        } else {
            Err(crate::Error::Execution("Execution failed".to_string()))
        }
    }

    /// Wait for an execution to complete
    async fn wait_for_execution(&self, execution_id: Uuid, timeout: Duration) -> Result<bool> {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(500);

        loop {
            if start.elapsed() > timeout {
                return Err(crate::Error::Timeout(format!(
                    "Execution {} timed out after {:?}",
                    execution_id, timeout
                )));
            }

            let execution = self
                .storage
                .get_execution(execution_id, &self.tenant_id)
                .await
                .map_err(|e| crate::Error::Internal(e.to_string()))?;

            match execution.state.as_str() {
                "succeeded" => return Ok(true),
                "failed" | "cancelled" => return Ok(false),
                _ => {
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }

    /// Update job heartbeat
    async fn update_heartbeat(&self, job_id: Uuid) -> Result<()> {
        self.storage
            .update_backfill_job_heartbeat(job_id, &self.tenant_id)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))
    }

    /// Update job progress counters
    async fn update_job_progress(
        &self,
        job_id: Uuid,
        completed: i32,
        failed: i32,
        skipped: i32,
    ) -> Result<()> {
        self.storage
            .update_backfill_job_progress(job_id, completed, failed, skipped, &self.tenant_id)
            .await
            .map_err(|e| crate::Error::Internal(e.to_string()))
    }
}

/// Run the backfill executor as a background service
pub async fn run_backfill_executor(
    storage: Arc<PostgresStorage>,
    orchestrator: ExecutionOrchestrator,
    tenant_id: TenantId,
    config: Option<BackfillExecutorConfig>,
) -> Result<()> {
    let executor = BackfillExecutor::new(
        storage,
        orchestrator,
        config.unwrap_or_default(),
        tenant_id,
    );
    executor.run().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = BackfillExecutorConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(5));
        assert_eq!(config.max_concurrent_jobs, 1);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(config.max_partition_retries, 3);
        assert_eq!(config.partition_delay, Duration::from_millis(100));
    }

    #[test]
    fn test_custom_config() {
        let config = BackfillExecutorConfig {
            poll_interval: Duration::from_secs(10),
            max_concurrent_jobs: 5,
            heartbeat_interval: Duration::from_secs(60),
            max_partition_retries: 5,
            partition_delay: Duration::from_millis(200),
        };
        assert_eq!(config.max_concurrent_jobs, 5);
        assert_eq!(config.max_partition_retries, 5);
    }
}
