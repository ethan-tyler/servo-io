//! Prometheus metrics for Servo runtime operations
//!
//! This module defines metrics for tracking execution orchestration and database operations.

use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec};

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
    }
}
