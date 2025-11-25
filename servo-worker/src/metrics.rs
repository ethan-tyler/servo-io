//! Prometheus metrics for workflow execution and operational monitoring.
//!
//! This module provides metrics for:
//! - Workflow execution counts and durations
//! - Rate limiter rejections
//! - Health and operational status
//!
//! # Metrics Endpoint
//!
//! Metrics are exposed at `/metrics` in Prometheus text format.
//!
//! # Example Queries
//!
//! ```promql
//! # Execution success rate (last 5 minutes)
//! sum(rate(servo_workflow_executions_total{status="succeeded"}[5m]))
//! / sum(rate(servo_workflow_executions_total[5m]))
//!
//! # P99 execution duration
//! histogram_quantile(0.99, rate(servo_workflow_duration_seconds_bucket[5m]))
//!
//! # Rate limit rejections by tenant
//! sum by (tenant_id) (rate(servo_rate_limit_rejections_total[5m]))
//! ```

use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Gauge, HistogramVec,
};
use std::sync::LazyLock;
use std::time::Instant;

// ============================================================================
// Workflow Execution Metrics
// ============================================================================

/// Total number of workflow executions by status and tenant.
///
/// Labels:
/// - `status`: succeeded | failed | timeout
/// - `tenant_id`: Tenant identifier
pub static WORKFLOW_EXECUTIONS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "servo_workflow_executions_total",
        "Total number of workflow executions by status",
        &["status", "tenant_id"]
    )
    .expect("Failed to register servo_workflow_executions_total metric")
});

/// Duration of workflow executions in seconds.
///
/// Labels:
/// - `status`: succeeded | failed | timeout
/// - `tenant_id`: Tenant identifier
///
/// Buckets: 0.1s, 0.5s, 1s, 5s, 10s, 30s, 60s, 120s, 300s, 600s
pub static WORKFLOW_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_workflow_duration_seconds",
        "Duration of workflow executions in seconds",
        &["status", "tenant_id"],
        vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0]
    )
    .expect("Failed to register servo_workflow_duration_seconds metric")
});

/// Number of assets in workflow executions.
///
/// Labels:
/// - `tenant_id`: Tenant identifier
///
/// Buckets: 1, 5, 10, 25, 50, 100, 250, 500
pub static WORKFLOW_ASSET_COUNT: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_workflow_asset_count",
        "Number of assets in workflow executions",
        &["tenant_id"],
        vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0]
    )
    .expect("Failed to register servo_workflow_asset_count metric")
});

// ============================================================================
// Rate Limiting Metrics
// ============================================================================

/// Total number of rate limit rejections.
///
/// Labels:
/// - `limiter_type`: tenant | ip
/// - `tenant_id`: Tenant identifier (for tenant limiter) or "N/A" (for IP limiter)
pub static RATE_LIMIT_REJECTIONS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "servo_rate_limit_rejections_total",
        "Total number of rate limit rejections",
        &["limiter_type", "tenant_id"]
    )
    .expect("Failed to register servo_rate_limit_rejections_total metric")
});

/// Current number of active tenant rate limiters.
///
/// This indicates the number of unique tenants that have made requests.
pub static ACTIVE_TENANT_LIMITERS: LazyLock<Gauge> = LazyLock::new(|| {
    register_gauge!(
        "servo_active_tenant_limiters",
        "Current number of active tenant rate limiters"
    )
    .expect("Failed to register servo_active_tenant_limiters metric")
});

// ============================================================================
// Request Metrics
// ============================================================================

/// Total number of HTTP requests by endpoint and status.
///
/// Labels:
/// - `endpoint`: execute | health | ready | metrics
/// - `status_code`: HTTP status code (200, 400, 401, 429, 500, etc.)
pub static HTTP_REQUESTS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "servo_http_requests_total",
        "Total number of HTTP requests by endpoint and status",
        &["endpoint", "status_code"]
    )
    .expect("Failed to register servo_http_requests_total metric")
});

/// HTTP request duration in seconds.
///
/// Labels:
/// - `endpoint`: execute | health | ready | metrics
///
/// Buckets: 1ms, 5ms, 10ms, 50ms, 100ms, 250ms, 500ms, 1s, 5s
pub static HTTP_REQUEST_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["endpoint"],
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0]
    )
    .expect("Failed to register servo_http_request_duration_seconds metric")
});

// ============================================================================
// Helper Functions
// ============================================================================

/// Record a workflow execution with its outcome and duration.
///
/// # Arguments
///
/// * `status` - Execution status: "succeeded", "failed", or "timeout"
/// * `tenant_id` - Tenant identifier
/// * `duration_secs` - Execution duration in seconds
/// * `asset_count` - Number of assets in the workflow
pub fn record_execution(status: &str, tenant_id: &str, duration_secs: f64, asset_count: usize) {
    WORKFLOW_EXECUTIONS_TOTAL
        .with_label_values(&[status, tenant_id])
        .inc();

    WORKFLOW_DURATION_SECONDS
        .with_label_values(&[status, tenant_id])
        .observe(duration_secs);

    WORKFLOW_ASSET_COUNT
        .with_label_values(&[tenant_id])
        .observe(asset_count as f64);
}

/// Record a rate limit rejection.
///
/// # Arguments
///
/// * `limiter_type` - Type of rate limiter: "tenant" or "ip"
/// * `tenant_id` - Tenant identifier (use "N/A" for IP limiter)
pub fn record_rate_limit_rejection(limiter_type: &str, tenant_id: &str) {
    RATE_LIMIT_REJECTIONS_TOTAL
        .with_label_values(&[limiter_type, tenant_id])
        .inc();
}

/// Update the active tenant limiters gauge.
///
/// # Arguments
///
/// * `count` - Current number of active tenant limiters
pub fn set_active_tenant_limiters(count: usize) {
    ACTIVE_TENANT_LIMITERS.set(count as f64);
}

/// Record an HTTP request.
///
/// # Arguments
///
/// * `endpoint` - Endpoint name: "execute", "health", "ready", "metrics"
/// * `status_code` - HTTP status code as string
/// * `duration_secs` - Request duration in seconds
pub fn record_http_request(endpoint: &str, status_code: &str, duration_secs: f64) {
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[endpoint, status_code])
        .inc();

    HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[endpoint])
        .observe(duration_secs);
}

/// Timer for measuring operation duration.
///
/// Automatically computes elapsed time when `elapsed_secs()` is called.
pub struct Timer {
    start: Instant,
}

impl Timer {
    /// Create a new timer starting from now.
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Get the elapsed time in seconds.
    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

/// Initialize all metrics by accessing them once.
///
/// This ensures metrics are registered with the global registry before
/// the first request, avoiding potential race conditions.
pub fn init_metrics() {
    // Force initialization of all LazyLock metrics
    let _ = &*WORKFLOW_EXECUTIONS_TOTAL;
    let _ = &*WORKFLOW_DURATION_SECONDS;
    let _ = &*WORKFLOW_ASSET_COUNT;
    let _ = &*RATE_LIMIT_REJECTIONS_TOTAL;
    let _ = &*ACTIVE_TENANT_LIMITERS;
    let _ = &*HTTP_REQUESTS_TOTAL;
    let _ = &*HTTP_REQUEST_DURATION_SECONDS;

    tracing::info!("Prometheus metrics initialized");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer() {
        let timer = Timer::start();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let elapsed = timer.elapsed_secs();
        assert!(elapsed >= 0.01, "Timer should measure at least 10ms");
        assert!(elapsed < 1.0, "Timer should not measure more than 1s");
    }

    #[test]
    fn test_record_execution() {
        // Initialize metrics first
        init_metrics();

        // Record an execution
        record_execution("succeeded", "test-tenant", 1.5, 5);

        // Verify counter was incremented (we can't easily check the value,
        // but if it panics the test fails)
        let _ = WORKFLOW_EXECUTIONS_TOTAL
            .with_label_values(&["succeeded", "test-tenant"])
            .get();
    }

    #[test]
    fn test_record_rate_limit_rejection() {
        init_metrics();

        record_rate_limit_rejection("tenant", "test-tenant");

        let _ = RATE_LIMIT_REJECTIONS_TOTAL
            .with_label_values(&["tenant", "test-tenant"])
            .get();
    }

    #[test]
    fn test_record_http_request() {
        init_metrics();

        record_http_request("execute", "200", 0.05);

        let _ = HTTP_REQUESTS_TOTAL
            .with_label_values(&["execute", "200"])
            .get();
    }

    #[test]
    fn test_set_active_tenant_limiters() {
        init_metrics();

        set_active_tenant_limiters(42);

        let value = ACTIVE_TENANT_LIMITERS.get();
        assert_eq!(value, 42.0);
    }
}
