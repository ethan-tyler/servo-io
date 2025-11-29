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
// Data Quality Check Metrics
// ============================================================================

/// Total number of data quality checks executed.
///
/// Labels:
/// - `tenant_id`: Tenant identifier
/// - `outcome`: passed | failed | error | skipped
pub static CHECK_EXECUTIONS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "servo_check_executions_total",
        "Total number of data quality checks executed",
        &["tenant_id", "outcome"]
    )
    .expect("Failed to register servo_check_executions_total metric")
});

/// Duration of data quality check batches in seconds.
///
/// Labels:
/// - `tenant_id`: Tenant identifier
///
/// Buckets: 10ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s
pub static CHECK_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_check_duration_seconds",
        "Duration of data quality check batches in seconds",
        &["tenant_id"],
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .expect("Failed to register servo_check_duration_seconds metric")
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
// Database Metrics
// ============================================================================

/// Duration of database operations in seconds.
///
/// Labels:
/// - `operation`: query | insert | update | delete | transaction
///
/// Buckets: 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 5s
pub static DB_OPERATION_DURATION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_db_operation_duration_seconds",
        "Duration of database operations in seconds",
        &["operation"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0]
    )
    .expect("Failed to register servo_db_operation_duration_seconds metric")
});

// ============================================================================
// Circuit Breaker Metrics
// ============================================================================

/// Circuit breaker state gauge.
///
/// Labels:
/// - `service`: Name of the protected service
///
/// Values:
/// - 0 = Closed (normal operation)
/// - 1 = Half-Open (testing recovery)
/// - 2 = Open (failing fast)
pub static CIRCUIT_BREAKER_STATE: LazyLock<prometheus::GaugeVec> = LazyLock::new(|| {
    prometheus::register_gauge_vec!(
        "servo_circuit_breaker_state",
        "Circuit breaker state (0=closed, 1=half-open, 2=open)",
        &["service"]
    )
    .expect("Failed to register servo_circuit_breaker_state metric")
});

/// Total number of circuit breaker transitions.
///
/// Labels:
/// - `service`: Name of the protected service
/// - `from_state`: Previous state (closed, half_open, open)
/// - `to_state`: New state (closed, half_open, open)
pub static CIRCUIT_BREAKER_TRANSITIONS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    register_counter_vec!(
        "servo_circuit_breaker_transitions_total",
        "Total number of circuit breaker state transitions",
        &["service", "from_state", "to_state"]
    )
    .expect("Failed to register servo_circuit_breaker_transitions_total metric")
});

/// Time spent in open state before recovery (histogram for MTTR tracking).
///
/// Labels:
/// - `service`: Name of the protected service
///
/// Buckets: 1s, 5s, 10s, 30s, 60s, 120s, 300s, 600s
pub static CIRCUIT_BREAKER_RECOVERY_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_circuit_breaker_recovery_seconds",
        "Time to recover from circuit breaker open state",
        &["service"],
        vec![1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0]
    )
    .expect("Failed to register servo_circuit_breaker_recovery_seconds metric")
});

// ============================================================================
// Execution Queue Metrics (for backpressure monitoring)
// ============================================================================

/// Current execution queue depth (pending executions).
///
/// Labels:
/// - `state`: pending | running
pub static EXECUTION_QUEUE_DEPTH: LazyLock<prometheus::GaugeVec> = LazyLock::new(|| {
    prometheus::register_gauge_vec!(
        "servo_execution_queue_depth",
        "Current number of executions by state",
        &["state"]
    )
    .expect("Failed to register servo_execution_queue_depth metric")
});

/// Time from execution creation to start (queue wait time).
///
/// Labels:
/// - `tenant_id`: Tenant identifier
///
/// Buckets: 100ms, 500ms, 1s, 5s, 10s, 30s, 60s, 300s
pub static EXECUTION_QUEUE_WAIT_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_execution_queue_wait_seconds",
        "Time spent waiting in queue before execution starts",
        &["tenant_id"],
        vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0]
    )
    .expect("Failed to register servo_execution_queue_wait_seconds metric")
});

// ============================================================================
// Error Detection Metrics (MTTD - Mean Time To Detect)
// ============================================================================

/// Time from error occurrence to detection in execution.
///
/// This helps measure how quickly we detect failures.
///
/// Labels:
/// - `error_type`: timeout | check_failure | internal | dependency
///
/// Buckets: 100ms, 500ms, 1s, 5s, 10s, 30s, 60s
pub static ERROR_DETECTION_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_error_detection_seconds",
        "Time to detect errors in workflow execution",
        &["error_type"],
        vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]
    )
    .expect("Failed to register servo_error_detection_seconds metric")
});

// ============================================================================
// Dependency Health Metrics
// ============================================================================

/// Latency to external dependencies.
///
/// Labels:
/// - `dependency`: database | cloud_tasks | external_api
///
/// Buckets: 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s
pub static DEPENDENCY_LATENCY_SECONDS: LazyLock<HistogramVec> = LazyLock::new(|| {
    register_histogram_vec!(
        "servo_dependency_latency_seconds",
        "Latency to external dependencies",
        &["dependency"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
    )
    .expect("Failed to register servo_dependency_latency_seconds metric")
});

/// Health status of dependencies.
///
/// Labels:
/// - `dependency`: database | cloud_tasks | external_api
///
/// Values: 0 = unhealthy, 1 = healthy
pub static DEPENDENCY_HEALTH: LazyLock<prometheus::GaugeVec> = LazyLock::new(|| {
    prometheus::register_gauge_vec!(
        "servo_dependency_health",
        "Health status of dependencies (0=unhealthy, 1=healthy)",
        &["dependency"]
    )
    .expect("Failed to register servo_dependency_health metric")
});

// ============================================================================
// Helper Functions
// ============================================================================

/// Record a database operation duration.
///
/// # Arguments
///
/// * `operation` - Operation type: "query", "insert", "update", "delete", "transaction"
/// * `duration_secs` - Operation duration in seconds
pub fn record_db_operation(operation: &str, duration_secs: f64) {
    DB_OPERATION_DURATION_SECONDS
        .with_label_values(&[operation])
        .observe(duration_secs);
}

/// Record a circuit breaker state transition.
///
/// # Arguments
///
/// * `service` - Name of the protected service
/// * `from_state` - Previous state
/// * `to_state` - New state
/// * `new_state_value` - Numeric value for new state (0=closed, 1=half-open, 2=open)
pub fn record_circuit_breaker_transition(
    service: &str,
    from_state: &str,
    to_state: &str,
    new_state_value: f64,
) {
    CIRCUIT_BREAKER_STATE
        .with_label_values(&[service])
        .set(new_state_value);

    CIRCUIT_BREAKER_TRANSITIONS_TOTAL
        .with_label_values(&[service, from_state, to_state])
        .inc();
}

/// Record circuit breaker recovery time.
///
/// # Arguments
///
/// * `service` - Name of the protected service
/// * `recovery_secs` - Time spent in open state before recovery
pub fn record_circuit_breaker_recovery(service: &str, recovery_secs: f64) {
    CIRCUIT_BREAKER_RECOVERY_SECONDS
        .with_label_values(&[service])
        .observe(recovery_secs);
}

/// Update execution queue depth.
///
/// # Arguments
///
/// * `pending` - Number of pending executions
/// * `running` - Number of running executions
pub fn set_execution_queue_depth(pending: i64, running: i64) {
    EXECUTION_QUEUE_DEPTH
        .with_label_values(&["pending"])
        .set(pending as f64);
    EXECUTION_QUEUE_DEPTH
        .with_label_values(&["running"])
        .set(running as f64);
}

/// Record execution queue wait time.
///
/// # Arguments
///
/// * `tenant_id` - Tenant identifier
/// * `wait_secs` - Time spent waiting in queue
pub fn record_execution_queue_wait(tenant_id: &str, wait_secs: f64) {
    let label = get_tenant_label(tenant_id);
    EXECUTION_QUEUE_WAIT_SECONDS
        .with_label_values(&[label])
        .observe(wait_secs);
}

/// Record time to detect an error.
///
/// # Arguments
///
/// * `error_type` - Type of error detected
/// * `detection_secs` - Time from error occurrence to detection
pub fn record_error_detection(error_type: &str, detection_secs: f64) {
    ERROR_DETECTION_SECONDS
        .with_label_values(&[error_type])
        .observe(detection_secs);
}

/// Record dependency latency.
///
/// # Arguments
///
/// * `dependency` - Name of the dependency
/// * `latency_secs` - Latency in seconds
pub fn record_dependency_latency(dependency: &str, latency_secs: f64) {
    DEPENDENCY_LATENCY_SECONDS
        .with_label_values(&[dependency])
        .observe(latency_secs);
}

/// Set dependency health status.
///
/// # Arguments
///
/// * `dependency` - Name of the dependency
/// * `healthy` - Whether the dependency is healthy
pub fn set_dependency_health(dependency: &str, healthy: bool) {
    DEPENDENCY_HEALTH
        .with_label_values(&[dependency])
        .set(if healthy { 1.0 } else { 0.0 });
}

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

/// Maximum number of unique tenant IDs to track in metrics.
/// Beyond this limit, tenant_id is aggregated as "other" to prevent cardinality explosion.
const MAX_TENANT_CARDINALITY: usize = 100;

/// Track unique tenants seen for cardinality protection
static SEEN_TENANTS: LazyLock<std::sync::RwLock<std::collections::HashSet<String>>> =
    LazyLock::new(|| std::sync::RwLock::new(std::collections::HashSet::new()));

/// Get tenant label with cardinality protection.
/// Returns "other" if too many unique tenants have been seen.
fn get_tenant_label(tenant_id: &str) -> &str {
    let seen = SEEN_TENANTS.read().unwrap();
    if seen.contains(tenant_id) {
        return tenant_id;
    }
    drop(seen);

    let mut seen = SEEN_TENANTS.write().unwrap();
    if seen.len() >= MAX_TENANT_CARDINALITY {
        "other"
    } else {
        seen.insert(tenant_id.to_string());
        tenant_id
    }
}

/// Record data quality check execution metrics.
///
/// # Arguments
///
/// * `tenant_id` - Tenant identifier (cardinality-protected)
/// * `passed` - Number of checks that passed
/// * `failed` - Number of checks that failed
/// * `skipped` - Number of checks that were skipped
/// * `duration_secs` - Total duration of check batch in seconds
pub fn record_check_execution(
    tenant_id: &str,
    passed: usize,
    failed: usize,
    skipped: usize,
    duration_secs: f64,
) {
    let label = get_tenant_label(tenant_id);

    if passed > 0 {
        CHECK_EXECUTIONS_TOTAL
            .with_label_values(&[label, "passed"])
            .inc_by(passed as f64);
    }

    if failed > 0 {
        CHECK_EXECUTIONS_TOTAL
            .with_label_values(&[label, "failed"])
            .inc_by(failed as f64);
    }

    if skipped > 0 {
        CHECK_EXECUTIONS_TOTAL
            .with_label_values(&[label, "skipped"])
            .inc_by(skipped as f64);
    }

    CHECK_DURATION_SECONDS
        .with_label_values(&[label])
        .observe(duration_secs);
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
    // Workflow metrics
    let _ = &*WORKFLOW_EXECUTIONS_TOTAL;
    let _ = &*WORKFLOW_DURATION_SECONDS;
    let _ = &*WORKFLOW_ASSET_COUNT;

    // Data quality check metrics
    let _ = &*CHECK_EXECUTIONS_TOTAL;
    let _ = &*CHECK_DURATION_SECONDS;

    // Rate limiting metrics
    let _ = &*RATE_LIMIT_REJECTIONS_TOTAL;
    let _ = &*ACTIVE_TENANT_LIMITERS;

    // HTTP metrics
    let _ = &*HTTP_REQUESTS_TOTAL;
    let _ = &*HTTP_REQUEST_DURATION_SECONDS;

    // Database metrics
    let _ = &*DB_OPERATION_DURATION_SECONDS;

    // Circuit breaker metrics
    let _ = &*CIRCUIT_BREAKER_STATE;
    let _ = &*CIRCUIT_BREAKER_TRANSITIONS_TOTAL;
    let _ = &*CIRCUIT_BREAKER_RECOVERY_SECONDS;

    // Execution queue metrics
    let _ = &*EXECUTION_QUEUE_DEPTH;
    let _ = &*EXECUTION_QUEUE_WAIT_SECONDS;

    // Error detection metrics
    let _ = &*ERROR_DETECTION_SECONDS;

    // Dependency health metrics
    let _ = &*DEPENDENCY_LATENCY_SECONDS;
    let _ = &*DEPENDENCY_HEALTH;

    tracing::info!("Prometheus metrics initialized (including MTTR-focused metrics)");
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
