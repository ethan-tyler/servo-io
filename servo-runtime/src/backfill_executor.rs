//! Backfill executor for processing partition backfill jobs
//!
//! The BackfillExecutor polls for pending backfill jobs and processes them
//! by executing each partition sequentially. It uses atomic claiming with
//! FOR UPDATE SKIP LOCKED to safely support multiple executor instances.
//!
//! ## Pause/Resume Support
//!
//! Jobs can be paused by the user. When paused:
//! - The executor stops at the next partition boundary
//! - The checkpoint (last completed partition) is recorded
//! - The job transitions to 'paused' state
//!
//! When resumed:
//! - The job transitions to 'resuming' state
//! - The executor claims it and transitions to 'running'
//! - Execution continues from where it left off

use crate::eta::EtaCalculator;
use crate::metrics::{
    BACKFILL_ETA_DISTRIBUTION, BACKFILL_JOBS_ACTIVE, BACKFILL_JOB_CANCELLATION_TOTAL,
    BACKFILL_JOB_CLAIM_TOTAL, BACKFILL_JOB_ETA_SECONDS, BACKFILL_JOB_PROGRESS_RATIO,
    BACKFILL_PARTITION_CLAIM_TOTAL, BACKFILL_PARTITION_COMPLETION_TOTAL, BACKFILL_PARTITION_DURATION,
};
use crate::orchestrator::ExecutionOrchestrator;
use crate::Result;
use chrono::Utc;
use servo_storage::{BackfillJobModel, BackfillPartitionModel, BackfillProgressUpdate, PostgresStorage, TenantId};
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
    /// Stale heartbeat threshold - jobs not updated in this time can be reclaimed (default: 120 seconds)
    pub stale_heartbeat_threshold: Duration,
    /// Timeout per partition execution (default: 5 minutes)
    pub partition_timeout: Duration,
}

impl Default for BackfillExecutorConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(5),
            max_concurrent_jobs: 1,
            heartbeat_interval: Duration::from_secs(30),
            max_partition_retries: 3,
            partition_delay: Duration::from_millis(100),
            stale_heartbeat_threshold: Duration::from_secs(120),
            partition_timeout: Duration::from_secs(300),
        }
    }
}

/// Backfill executor for processing partition backfill jobs
pub struct BackfillExecutor {
    storage: Arc<PostgresStorage>,
    orchestrator: ExecutionOrchestrator,
    config: BackfillExecutorConfig,
    tenant_id: TenantId,
    executor_id: String,
}

impl BackfillExecutor {
    /// Create a new backfill executor
    pub fn new(
        storage: Arc<PostgresStorage>,
        orchestrator: ExecutionOrchestrator,
        config: BackfillExecutorConfig,
        tenant_id: TenantId,
    ) -> Self {
        // Generate unique executor ID for this instance
        let executor_id = format!("executor-{}", Uuid::new_v4());
        Self {
            storage,
            orchestrator,
            config,
            tenant_id,
            executor_id,
        }
    }

    /// Start the executor loop (runs until cancelled)
    #[instrument(skip(self), fields(tenant_id = %self.tenant_id.as_str(), executor_id = %self.executor_id))]
    pub async fn run(&self) -> Result<()> {
        info!("Starting backfill executor {}", self.executor_id);
        let mut poll_interval = interval(self.config.poll_interval);

        loop {
            poll_interval.tick().await;

            match self.poll_and_process_jobs().await {
                Ok(processed) => {
                    if processed > 0 {
                        debug!("Processed {} backfill jobs", processed);
                    }
                }
                Err(e) => {
                    error!("Error polling backfill jobs: {}", e);
                }
            }
        }
    }

    /// Poll for pending jobs and process them using atomic claiming
    async fn poll_and_process_jobs(&self) -> Result<usize> {
        let stale_threshold_secs = self.config.stale_heartbeat_threshold.as_secs() as i64;
        let mut processed = 0;

        // Try to claim and process jobs up to max_concurrent_jobs
        for _ in 0..self.config.max_concurrent_jobs {
            // Atomically claim a job
            let claimed_job = self
                .storage
                .claim_pending_backfill_job(&self.executor_id, stale_threshold_secs, &self.tenant_id)
                .await
                .map_err(|e| crate::Error::Internal(e.to_string()))?;

            match claimed_job {
                Some(job) => {
                    // Determine if this was a new claim or reclaim (based on previous state)
                    let claim_type = if job.started_at.is_some() {
                        "reclaim"
                    } else {
                        "new"
                    };
                    BACKFILL_JOB_CLAIM_TOTAL
                        .with_label_values(&[claim_type, "success"])
                        .inc();

                    // Track active running jobs (low-cardinality metric)
                    BACKFILL_JOBS_ACTIVE.with_label_values(&["running"]).inc();

                    info!(
                        job_id = %job.id,
                        asset = %job.asset_name,
                        claim_type = claim_type,
                        "Claimed backfill job"
                    );
                    match self.process_job(job).await {
                        Ok(_) => processed += 1,
                        Err(e) => {
                            error!("Failed to process backfill job: {}", e);
                        }
                    }

                    // Decrement running jobs counter when done (regardless of success/failure)
                    BACKFILL_JOBS_ACTIVE.with_label_values(&["running"]).dec();
                }
                None => {
                    // No more jobs to claim
                    BACKFILL_JOB_CLAIM_TOTAL
                        .with_label_values(&["new", "none_available"])
                        .inc();
                    break;
                }
            }
        }

        Ok(processed)
    }

    /// Process a single backfill job (already claimed and in running state)
    #[instrument(skip(self), fields(job_id = %job.id, asset = %job.asset_name))]
    async fn process_job(&self, job: BackfillJobModel) -> Result<()> {
        info!("Processing backfill job {} for asset {}", job.id, job.asset_name);

        let mut completed_count = job.completed_partitions;
        let mut failed_count = job.failed_partitions;
        let skipped_count = job.skipped_partitions;
        let mut last_completed_partition: Option<String> = job.checkpoint_partition_key.clone();

        // Initialize ETA calculator with existing average if available
        let mut eta_calculator = if let Some(avg_ms) = job.avg_partition_duration_ms {
            EtaCalculator::with_initial_avg(0.3, avg_ms)
        } else {
            EtaCalculator::new(0.3)
        };

        // Process partitions using atomic claiming
        loop {
            // Check if job was cancelled or paused before processing next partition
            let is_cancelled = self
                .storage
                .is_backfill_job_cancelled(job.id, &self.tenant_id)
                .await
                .map_err(|e| crate::Error::Internal(e.to_string()))?;

            if is_cancelled {
                BACKFILL_JOB_CANCELLATION_TOTAL
                    .with_label_values(&["detected_during_processing"])
                    .inc();
                info!(
                    job_id = %job.id,
                    completed = completed_count,
                    "Job cancelled, stopping partition processing"
                );
                // Job is already in cancelled state, just return
                return Ok(());
            }

            // Check if job was paused
            let is_paused = self
                .storage
                .is_backfill_job_paused(job.id, &self.tenant_id)
                .await
                .map_err(|e| crate::Error::Internal(e.to_string()))?;

            if is_paused {
                info!(
                    job_id = %job.id,
                    completed = completed_count,
                    checkpoint = ?last_completed_partition,
                    "Job paused, stopping partition processing at checkpoint"
                );
                // Job is already in paused state (set by pause_backfill_job), just return
                return Ok(());
            }

            // Update heartbeat before claiming next partition
            self.update_heartbeat(job.id).await?;

            // Atomically claim a partition
            let claimed_partition = self
                .storage
                .claim_pending_partition(job.id, self.config.max_partition_retries, &self.tenant_id)
                .await
                .map_err(|e| crate::Error::Internal(e.to_string()))?;

            match claimed_partition {
                Some(partition) => {
                    // Track partition claim type (new vs retry)
                    let claim_type = if partition.attempt_count > 1 {
                        "retry"
                    } else {
                        "new"
                    };
                    BACKFILL_PARTITION_CLAIM_TOTAL
                        .with_label_values(&[claim_type, "success"])
                        .inc();

                    // Small delay between partitions
                    if completed_count > 0 || failed_count > 0 {
                        tokio::time::sleep(self.config.partition_delay).await;
                    }

                    // Execute the partition and track duration
                    let partition_start = std::time::Instant::now();
                    match self.execute_partition(&job, &partition).await {
                        Ok(_) => {
                            let duration_ms = partition_start.elapsed().as_millis() as u64;
                            let duration_secs = duration_ms as f64 / 1000.0;

                            BACKFILL_PARTITION_DURATION
                                .with_label_values(&["completed"])
                                .observe(duration_secs);
                            BACKFILL_PARTITION_COMPLETION_TOTAL
                                .with_label_values(&["completed"])
                                .inc();

                            // Update ETA calculator
                            eta_calculator.update(duration_ms);

                            completed_count += 1;
                            last_completed_partition = Some(partition.partition_key.clone());

                            // Calculate remaining partitions and ETA
                            let remaining = job.total_partitions - completed_count - failed_count - skipped_count;
                            let eta = eta_calculator.estimate_completion_time(remaining as u32);

                            info!(
                                partition_key = %partition.partition_key,
                                completed = completed_count,
                                total = job.total_partitions,
                                duration_ms = duration_ms,
                                eta = ?eta.map(|t| t.format("%H:%M:%S").to_string()),
                                "Partition completed"
                            );
                        }
                        Err(e) => {
                            let duration_secs = partition_start.elapsed().as_secs_f64();
                            BACKFILL_PARTITION_DURATION
                                .with_label_values(&["failed"])
                                .observe(duration_secs);

                            // Check if we've exceeded max retries
                            if partition.attempt_count >= self.config.max_partition_retries {
                                BACKFILL_PARTITION_COMPLETION_TOTAL
                                    .with_label_values(&["failed"])
                                    .inc();
                                failed_count += 1;
                                warn!(
                                    partition_key = %partition.partition_key,
                                    attempts = partition.attempt_count,
                                    error = %e,
                                    "Partition failed permanently (max retries exceeded)"
                                );
                            } else {
                                warn!(
                                    partition_key = %partition.partition_key,
                                    attempts = partition.attempt_count,
                                    max_retries = self.config.max_partition_retries,
                                    error = %e,
                                    "Partition failed, will retry"
                                );
                            }
                        }
                    }

                    // Update job progress with ETA (idempotent - always set absolute values)
                    let remaining = job.total_partitions - completed_count - failed_count - skipped_count;
                    let estimated_completion = eta_calculator.estimate_completion_time(remaining as u32);

                    // Emit progress metrics
                    let progress_ratio = if job.total_partitions > 0 {
                        completed_count as f64 / job.total_partitions as f64
                    } else {
                        0.0
                    };
                    BACKFILL_JOB_PROGRESS_RATIO
                        .with_label_values(&[&job.id.to_string(), &job.asset_name])
                        .set(progress_ratio);

                    if let Some(eta) = &estimated_completion {
                        let eta_seconds = (*eta - Utc::now()).num_seconds().max(0) as f64;

                        // Low-cardinality ETA distribution (recommended)
                        BACKFILL_ETA_DISTRIBUTION
                            .with_label_values(&[])
                            .observe(eta_seconds);

                        // High-cardinality per-job ETA (deprecated)
                        BACKFILL_JOB_ETA_SECONDS
                            .with_label_values(&[&job.id.to_string(), &job.asset_name])
                            .set(eta_seconds);
                    }

                    let progress = BackfillProgressUpdate {
                        completed: completed_count,
                        failed: failed_count,
                        skipped: skipped_count,
                        avg_partition_duration_ms: Some(eta_calculator.avg_duration_ms()),
                        estimated_completion_at: estimated_completion,
                        checkpoint_partition_key: last_completed_partition.clone(),
                    };
                    self.update_job_progress_with_eta(job.id, &progress).await?;
                }
                None => {
                    // No more partitions to process - job is done
                    break;
                }
            }
        }

        // Determine final job state
        let total_processed = completed_count + failed_count + skipped_count;
        let final_state = if failed_count > 0 && completed_count == 0 {
            "failed"
        } else if total_processed >= job.total_partitions {
            "completed"
        } else {
            // Still have unprocessed partitions - shouldn't happen but handle gracefully
            warn!(
                "Job {} has unprocessed partitions: {} of {} done",
                job.id, total_processed, job.total_partitions
            );
            "completed"
        };

        let error_message = if failed_count > 0 {
            Some(format!(
                "{} of {} partitions failed after {} retries each",
                failed_count, job.total_partitions, self.config.max_partition_retries
            ))
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
            job_id = %job.id,
            completed = completed_count,
            failed = failed_count,
            skipped = skipped_count,
            final_state = final_state,
            "Backfill job finished"
        );

        Ok(())
    }

    /// Execute a single partition (already claimed and in running state)
    #[instrument(skip(self, job), fields(partition_key = %partition.partition_key, attempt = partition.attempt_count))]
    async fn execute_partition(
        &self,
        job: &BackfillJobModel,
        partition: &BackfillPartitionModel,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        // Execute the asset for this partition with idempotency key
        let idempotency_key = format!("backfill:{}:{}:{}", job.id, partition.partition_key, partition.attempt_count);
        let execution_result = self
            .execute_asset_partition(job.asset_id, &partition.partition_key, &idempotency_key)
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
                // Mark partition as failed (will be retried if under max_retries)
                // If the DB update fails, log it but still return the original execution error
                if let Err(db_err) = self
                    .storage
                    .fail_backfill_partition(
                        partition.id,
                        &e.to_string(),
                        duration_ms,
                        &self.tenant_id,
                    )
                    .await
                {
                    warn!("Failed to update partition failure state: {}", db_err);
                }
                Err(e)
            }
        }
    }

    /// Execute an asset for a specific partition with idempotency
    async fn execute_asset_partition(
        &self,
        asset_id: Uuid,
        partition_key: &str,
        idempotency_key: &str,
    ) -> Result<Uuid> {
        debug!(
            asset_id = %asset_id,
            partition_key = %partition_key,
            idempotency_key = %idempotency_key,
            "Executing asset partition"
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

        // Start execution with idempotency key to prevent duplicates on retry
        let execution_id = self
            .orchestrator
            .start_execution(workflow.id, &self.tenant_id, Some(idempotency_key.to_string()))
            .await
            .map_err(|e| crate::Error::Execution(e.to_string()))?;

        // Wait for execution to complete (with timeout)
        let result = self
            .wait_for_execution(execution_id, self.config.partition_timeout)
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

    /// Update job progress counters with ETA information
    async fn update_job_progress_with_eta(
        &self,
        job_id: Uuid,
        progress: &BackfillProgressUpdate,
    ) -> Result<()> {
        self.storage
            .update_backfill_job_progress_with_eta(job_id, progress, &self.tenant_id)
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
        assert_eq!(config.stale_heartbeat_threshold, Duration::from_secs(120));
        assert_eq!(config.partition_timeout, Duration::from_secs(300));
    }

    #[test]
    fn test_custom_config() {
        let config = BackfillExecutorConfig {
            poll_interval: Duration::from_secs(10),
            max_concurrent_jobs: 5,
            heartbeat_interval: Duration::from_secs(60),
            max_partition_retries: 5,
            partition_delay: Duration::from_millis(200),
            stale_heartbeat_threshold: Duration::from_secs(300),
            partition_timeout: Duration::from_secs(600),
        };
        assert_eq!(config.max_concurrent_jobs, 5);
        assert_eq!(config.max_partition_retries, 5);
        assert_eq!(config.stale_heartbeat_threshold, Duration::from_secs(300));
    }

    #[test]
    fn test_idempotency_key_format() {
        let job_id = Uuid::new_v4();
        let partition_key = "2024-01-15";
        let attempt = 2;
        let key = format!("backfill:{}:{}:{}", job_id, partition_key, attempt);

        assert!(key.starts_with("backfill:"));
        assert!(key.contains(partition_key));
        assert!(key.ends_with(":2"));
    }

    #[test]
    fn test_valid_state_transitions() {
        // Define valid state transitions for backfill jobs
        let valid_transitions = vec![
            ("pending", "running"),
            ("running", "paused"),
            ("running", "completed"),
            ("running", "failed"),
            ("running", "cancelled"),
            ("paused", "resuming"),
            ("paused", "cancelled"),
            ("resuming", "running"),
        ];

        // All valid states
        let all_states = ["pending", "running", "paused", "resuming", "completed", "failed", "cancelled"];
        let terminal_states = ["completed", "failed", "cancelled"];

        for (from, to) in &valid_transitions {
            // Just verify these are valid states
            assert!(all_states.contains(from), "Invalid 'from' state: {}", from);
            assert!(all_states.contains(to), "Invalid 'to' state: {}", to);
        }

        // Verify terminal states have no outgoing transitions
        for terminal in terminal_states {
            for (from, _) in &valid_transitions {
                assert_ne!(*from, terminal, "Terminal state {} should not have outgoing transitions", terminal);
            }
        }
    }

    #[test]
    fn test_pause_boundary_behavior() {
        // This test documents the expected pause boundary behavior:
        // - When pause is requested, the current partition completes
        // - The checkpoint is set to the last completed partition
        // - No new partitions are picked up after the current one

        // Simulate partition keys for a job
        let partitions = vec!["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04", "2024-01-05"];

        // Simulate processing up to partition 3 when pause is requested
        let completed_before_pause = vec!["2024-01-01", "2024-01-02"];
        let in_flight_when_pause = "2024-01-03"; // This will complete
        let not_started = vec!["2024-01-04", "2024-01-05"]; // These won't be picked up

        // Expected checkpoint after pause
        let expected_checkpoint = in_flight_when_pause; // Last completed partition

        // Verify the checkpoint logic
        let completed_after_pause: Vec<&str> = completed_before_pause
            .iter()
            .copied()
            .chain(std::iter::once(in_flight_when_pause))
            .collect();

        assert_eq!(completed_after_pause.last(), Some(&expected_checkpoint));
        assert_eq!(not_started.len(), 2); // These should remain pending after pause

        // When resumed, processing should start after the checkpoint
        let resume_index = partitions.iter().position(|p| *p == expected_checkpoint).unwrap() + 1;
        assert_eq!(partitions[resume_index], "2024-01-04"); // First partition after checkpoint
    }
}
