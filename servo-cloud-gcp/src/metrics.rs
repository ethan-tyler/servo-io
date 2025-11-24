//! Prometheus metrics for Cloud Tasks integration
//!
//! This module defines metrics with consistent naming and appropriate types:
//! - Histograms for latency measurements with defined buckets
//! - Counters for success/failure/rate-limit tracking

use lazy_static::lazy_static;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, HistogramVec, IntCounterVec,
};

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
    /// - tenant_id: UUID of the tenant (for debugging, can be disabled in production if cardinality is an issue)
    pub static ref ENQUEUE_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_enqueue_total",
        "Total number of task enqueue attempts",
        &["status", "tenant_id"]
    )
    .expect("servo_enqueue_total metric registration");

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Ensure all metrics are registered correctly by accessing them
        // This will panic if they're not registered
        let _ = ENQUEUE_DURATION.with_label_values(&["api"]);
        let _ = ENQUEUE_TOTAL.with_label_values(&["success", "tenant1"]);
        let _ = ENQUEUE_RETRIES.with_label_values(&["5xx"]);
        let _ = TOKEN_ACQUISITION_DURATION.with_label_values(&["true"]);
    }
}
