//! Prometheus metrics for Servo runtime operations
//!
//! This module defines metrics for tracking execution orchestration and database operations.

use lazy_static::lazy_static;
use prometheus::{
    register_gauge_vec, register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    GaugeVec, HistogramVec, IntCounterVec, IntGaugeVec,
};

lazy_static! {
    /// Duration of database operations in seconds
    ///
    /// Labels:
    /// - operation: "create_execution", "update_state", "find_by_id", "find_by_idempotency_key"
    ///
    /// Buckets: 0.001s to 10s (1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s)
    pub static ref DB_OPERATION_DURATION: HistogramVec = register_histogram_vec!(
        "servo_db_operation_duration_seconds",
        "Duration of database operations",
        &["operation"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("servo_db_operation_duration_seconds metric registration");

    /// Total number of execution creation attempts
    ///
    /// Labels:
    /// - status: "success", "duplicate" (idempotency hit), "failure"
    pub static ref EXECUTION_CREATE_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_execution_create_total",
        "Total number of execution creation attempts",
        &["status"]
    )
    .expect("servo_execution_create_total metric registration");

    /// Total number of execution state transitions
    ///
    /// Labels:
    /// - from_state: "pending", "running", "failed", "succeeded"
    /// - to_state: "pending", "running", "failed", "succeeded", "retry"
    /// - status: "success", "failure"
    pub static ref EXECUTION_STATE_TRANSITION_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_execution_state_transition_total",
        "Total number of execution state transitions",
        &["from_state", "to_state", "status"]
    )
    .expect("servo_execution_state_transition_total metric registration");

    /// Total number of backfill job claims
    ///
    /// Labels:
    /// - claim_type: "new" (pending job), "reclaim" (stale heartbeat recovery)
    /// - status: "success", "none_available"
    pub static ref BACKFILL_JOB_CLAIM_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_job_claim_total",
        "Total number of backfill job claim attempts",
        &["claim_type", "status"]
    )
    .expect("servo_backfill_job_claim_total metric registration");

    /// Total number of backfill partition claims
    ///
    /// Labels:
    /// - claim_type: "new" (pending), "retry" (failed partition retry)
    /// - status: "success", "none_available"
    pub static ref BACKFILL_PARTITION_CLAIM_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_partition_claim_total",
        "Total number of backfill partition claim attempts",
        &["claim_type", "status"]
    )
    .expect("servo_backfill_partition_claim_total metric registration");

    /// Total number of backfill partition completions
    ///
    /// Labels:
    /// - status: "completed", "failed", "skipped"
    pub static ref BACKFILL_PARTITION_COMPLETION_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_partition_completion_total",
        "Total number of backfill partition completions",
        &["status"]
    )
    .expect("servo_backfill_partition_completion_total metric registration");

    /// Total number of backfill job cancellations
    pub static ref BACKFILL_JOB_CANCELLATION_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_job_cancellation_total",
        "Total number of backfill job cancellations",
        &["reason"]
    )
    .expect("servo_backfill_job_cancellation_total metric registration");

    /// Duration of backfill partition execution in seconds
    pub static ref BACKFILL_PARTITION_DURATION: HistogramVec = register_histogram_vec!(
        "servo_backfill_partition_duration_seconds",
        "Duration of backfill partition execution",
        &["status"],
        vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0]
    )
    .expect("servo_backfill_partition_duration_seconds metric registration");

    /// Total number of backfill job pause events
    ///
    /// Labels:
    /// - status: "success", "failed" (e.g., job not in running state)
    pub static ref BACKFILL_JOB_PAUSE_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_job_pause_total",
        "Total number of backfill job pause events",
        &["status"]
    )
    .expect("servo_backfill_job_pause_total metric registration");

    /// Total number of backfill job resume events
    ///
    /// Labels:
    /// - status: "success", "failed" (e.g., job not in paused state)
    pub static ref BACKFILL_JOB_RESUME_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_backfill_job_resume_total",
        "Total number of backfill job resume events",
        &["status"]
    )
    .expect("servo_backfill_job_resume_total metric registration");

    // =========================================================================
    // LOW-CARDINALITY AGGREGATE METRICS (Recommended for production use)
    // =========================================================================

    /// Number of active backfill jobs by state (low cardinality)
    ///
    /// Labels:
    /// - state: "pending", "running", "paused", "resuming"
    ///
    /// Use this metric for monitoring active job counts. Maximum cardinality: 4 series.
    pub static ref BACKFILL_JOBS_ACTIVE: IntGaugeVec = register_int_gauge_vec!(
        "servo_backfill_jobs_active",
        "Number of active backfill jobs by state",
        &["state"]
    )
    .expect("servo_backfill_jobs_active metric registration");

    /// Distribution of ETA estimates for active backfill jobs (low cardinality)
    ///
    /// Buckets: 1min, 5min, 10min, 30min, 1h, 2h, 4h (in seconds)
    ///
    /// Use this histogram to understand the distribution of remaining time across all jobs.
    pub static ref BACKFILL_ETA_DISTRIBUTION: HistogramVec = register_histogram_vec!(
        "servo_backfill_eta_distribution_seconds",
        "Distribution of estimated remaining time for active backfill jobs",
        &[],
        vec![60.0, 300.0, 600.0, 1800.0, 3600.0, 7200.0, 14400.0]
    )
    .expect("servo_backfill_eta_distribution_seconds metric registration");

    /// Distribution of pause durations for backfill jobs (low cardinality)
    ///
    /// Buckets: 1min, 5min, 10min, 30min, 1h, 4h, 24h (in seconds)
    ///
    /// Use this histogram to monitor how long jobs stay in paused state.
    pub static ref BACKFILL_PAUSE_DURATION: HistogramVec = register_histogram_vec!(
        "servo_backfill_pause_duration_seconds",
        "Duration jobs spend in paused state before resumption",
        &[],
        vec![60.0, 300.0, 600.0, 1800.0, 3600.0, 14400.0, 86400.0]
    )
    .expect("servo_backfill_pause_duration_seconds metric registration");

    // =========================================================================
    // DEPRECATED: HIGH-CARDINALITY METRICS (Use aggregates above instead)
    // These metrics use job_id labels which can cause Prometheus memory issues
    // at scale. They are kept for backward compatibility during migration.
    // =========================================================================

    /// DEPRECATED: Use BACKFILL_ETA_DISTRIBUTION instead
    ///
    /// Current ETA for active backfill jobs in seconds
    /// WARNING: High cardinality - job_id label creates unbounded series
    ///
    /// Labels:
    /// - job_id: UUID of the backfill job
    /// - asset_name: Name of the asset being backfilled
    pub static ref BACKFILL_JOB_ETA_SECONDS: GaugeVec = register_gauge_vec!(
        "servo_backfill_job_eta_seconds",
        "Estimated time remaining for backfill jobs in seconds (DEPRECATED: high cardinality)",
        &["job_id", "asset_name"]
    )
    .expect("servo_backfill_job_eta_seconds metric registration");

    /// DEPRECATED: Use BACKFILL_JOBS_ACTIVE for job counts
    ///
    /// Current progress ratio (0-1) for active backfill jobs
    /// WARNING: High cardinality - job_id label creates unbounded series
    ///
    /// Labels:
    /// - job_id: UUID of the backfill job
    /// - asset_name: Name of the asset being backfilled
    pub static ref BACKFILL_JOB_PROGRESS_RATIO: GaugeVec = register_gauge_vec!(
        "servo_backfill_job_progress_ratio",
        "Progress ratio (0-1) for active backfill jobs (DEPRECATED: high cardinality)",
        &["job_id", "asset_name"]
    )
    .expect("servo_backfill_job_progress_ratio metric registration");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Ensure all metrics are registered correctly by accessing them
        // This will panic if they're not registered
        let _ = DB_OPERATION_DURATION.with_label_values(&["test"]);
        let _ = EXECUTION_CREATE_TOTAL.with_label_values(&["success"]);
        let _ =
            EXECUTION_STATE_TRANSITION_TOTAL.with_label_values(&["pending", "running", "success"]);

        // Backfill metrics
        let _ = BACKFILL_JOB_CLAIM_TOTAL.with_label_values(&["new", "success"]);
        let _ = BACKFILL_PARTITION_CLAIM_TOTAL.with_label_values(&["new", "success"]);
        let _ = BACKFILL_PARTITION_COMPLETION_TOTAL.with_label_values(&["completed"]);
        let _ = BACKFILL_JOB_CANCELLATION_TOTAL.with_label_values(&["user"]);
        let _ = BACKFILL_PARTITION_DURATION.with_label_values(&["completed"]);

        // Pause/resume metrics
        let _ = BACKFILL_JOB_PAUSE_TOTAL.with_label_values(&["success"]);
        let _ = BACKFILL_JOB_RESUME_TOTAL.with_label_values(&["success"]);

        // Low-cardinality aggregate metrics (recommended)
        let _ = BACKFILL_JOBS_ACTIVE.with_label_values(&["running"]);
        let _ = BACKFILL_ETA_DISTRIBUTION.with_label_values(&[]);
        let _ = BACKFILL_PAUSE_DURATION.with_label_values(&[]);

        // Deprecated high-cardinality metrics (kept for backward compatibility)
        let _ = BACKFILL_JOB_ETA_SECONDS.with_label_values(&["test-job-id", "test-asset"]);
        let _ = BACKFILL_JOB_PROGRESS_RATIO.with_label_values(&["test-job-id", "test-asset"]);
    }
}
