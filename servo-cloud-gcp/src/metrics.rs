//! Prometheus metrics for Cloud Tasks integration
//!
//! This module defines metrics with consistent naming and appropriate types:
//! - Histograms for latency measurements with defined buckets
//! - Counters for success/failure/rate-limit tracking

use lazy_static::lazy_static;
use prometheus::{register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec};

lazy_static! {
    /// Duration of task enqueue operations in seconds
    ///
    /// Labels:
    /// - operation: "api" (Cloud Tasks API call), "token" (token acquisition), "total" (end-to-end)
    ///
    /// Buckets: 0.01s to 30s (10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s, 30s)
    pub static ref ENQUEUE_DURATION: HistogramVec = register_histogram_vec!(
        "servo_enqueue_duration_seconds",
        "Duration of task enqueue operations",
        &["operation"],
        vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]
    )
    .expect("servo_enqueue_duration_seconds metric registration");

    /// Total number of task enqueue attempts
    ///
    /// Labels:
    /// - status: "success", "failure", "rate_limit"
    ///
    /// Note: tenant_id label is controlled by SERVO_METRICS_INCLUDE_TENANT_ID env var.
    /// Set to "false" in production to avoid high cardinality.
    pub static ref ENQUEUE_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_enqueue_total",
        "Total number of task enqueue attempts",
        &["status"]
    )
    .expect("servo_enqueue_total metric registration");

    /// Total number of task enqueue attempts by tenant (optional, high cardinality)
    ///
    /// Only populated if SERVO_METRICS_INCLUDE_TENANT_ID=true
    ///
    /// Labels:
    /// - status: "success", "failure", "rate_limit"
    /// - tenant_id: UUID of the tenant
    pub static ref ENQUEUE_TOTAL_BY_TENANT: IntCounterVec = register_int_counter_vec!(
        "servo_enqueue_total_by_tenant",
        "Total number of task enqueue attempts by tenant (high cardinality)",
        &["status", "tenant_id"]
    )
    .expect("servo_enqueue_total_by_tenant metric registration");

    /// Number of retry attempts for Cloud Tasks API calls
    ///
    /// Labels:
    /// - reason: "5xx", "429", "timeout", "network"
    pub static ref ENQUEUE_RETRIES: IntCounterVec = register_int_counter_vec!(
        "servo_enqueue_retries_total",
        "Number of retry attempts for Cloud Tasks API calls",
        &["reason"]
    )
    .expect("servo_enqueue_retries_total metric registration");

    /// Duration of token acquisition in seconds
    ///
    /// Buckets: 0.01s to 10s
    pub static ref TOKEN_ACQUISITION_DURATION: HistogramVec = register_histogram_vec!(
        "servo_token_acquisition_duration_seconds",
        "Duration of OAuth2 token acquisition",
        &["cache_hit"],
        vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("servo_token_acquisition_duration_seconds metric registration");

    // =========================================================================
    // Executor Metrics
    // =========================================================================

    /// Duration of workflow execution requests in seconds
    ///
    /// Buckets: 0.01s to 30s (covers DB write + Cloud Tasks enqueue)
    pub static ref EXECUTOR_EXECUTE_DURATION: prometheus::Histogram = prometheus::register_histogram!(
        "servo_executor_execute_duration_seconds",
        "Duration of workflow execution requests (DB write + enqueue)",
        vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0]
    )
    .expect("servo_executor_execute_duration_seconds metric registration");

    /// Total number of workflow execution requests
    ///
    /// Labels:
    /// - status: "success", "error", "idempotent"
    pub static ref EXECUTOR_EXECUTE_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_executor_execute_total",
        "Total number of workflow execution requests",
        &["status"]
    )
    .expect("servo_executor_execute_total metric registration");

    /// Duration of execution cancellation requests in seconds
    pub static ref EXECUTOR_CANCEL_DURATION: prometheus::Histogram = prometheus::register_histogram!(
        "servo_executor_cancel_duration_seconds",
        "Duration of execution cancellation requests",
        vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]
    )
    .expect("servo_executor_cancel_duration_seconds metric registration");

    /// Total number of execution cancellation requests
    ///
    /// Labels:
    /// - status: "success", "error", "skipped"
    pub static ref EXECUTOR_CANCEL_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_executor_cancel_total",
        "Total number of execution cancellation requests",
        &["status"]
    )
    .expect("servo_executor_cancel_total metric registration");

    /// Duration of execution status checks in seconds
    pub static ref EXECUTOR_STATUS_DURATION: prometheus::Histogram = prometheus::register_histogram!(
        "servo_executor_status_duration_seconds",
        "Duration of execution status checks",
        vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
    )
    .expect("servo_executor_status_duration_seconds metric registration");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Ensure all metrics are registered correctly by accessing them
        // This will panic if they're not registered
        let _ = ENQUEUE_DURATION.with_label_values(&["api"]);
        let _ = ENQUEUE_TOTAL.with_label_values(&["success"]);
        let _ = ENQUEUE_TOTAL_BY_TENANT.with_label_values(&["success", "tenant1"]);
        let _ = ENQUEUE_RETRIES.with_label_values(&["5xx"]);
        let _ = TOKEN_ACQUISITION_DURATION.with_label_values(&["true"]);
    }
}
