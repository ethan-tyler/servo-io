//! Prometheus metrics for storage operations
//!
//! This module defines metrics for:
//! - Circuit breaker state and transitions
//! - Database operation latency
//! - Query errors by type

use lazy_static::lazy_static;
use prometheus::{register_gauge_vec, register_int_counter_vec, GaugeVec, IntCounterVec};

lazy_static! {
    /// Circuit breaker state gauge
    ///
    /// Values:
    /// - 0 = closed (normal operation)
    /// - 1 = open (fail-fast mode)
    ///
    /// Labels:
    /// - dependency: Name of the protected dependency (e.g., "postgres")
    pub static ref CIRCUIT_BREAKER_STATE: GaugeVec = register_gauge_vec!(
        "servo_circuit_breaker_state",
        "Circuit breaker state (0=closed, 1=open)",
        &["dependency"]
    )
    .expect("Failed to register circuit_breaker_state metric");

    /// Circuit breaker open events counter
    ///
    /// Incremented each time a circuit breaker transitions from closed to open.
    ///
    /// Labels:
    /// - dependency: Name of the protected dependency (e.g., "postgres")
    pub static ref CIRCUIT_BREAKER_OPENS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_circuit_breaker_opens_total",
        "Total number of circuit breaker open events",
        &["dependency"]
    )
    .expect("Failed to register circuit_breaker_opens_total metric");

    /// Half-open probe attempts counter
    ///
    /// Tracks half-open state probe attempts and their outcomes.
    ///
    /// Labels:
    /// - dependency: Name of the protected dependency (e.g., "postgres")
    /// - result: "success" or "failure"
    pub static ref CIRCUIT_BREAKER_HALF_OPEN_ATTEMPTS: IntCounterVec = register_int_counter_vec!(
        "servo_circuit_breaker_half_open_attempts_total",
        "Total number of half-open probe attempts",
        &["dependency", "result"]
    )
    .expect("Failed to register circuit_breaker_half_open_attempts_total metric");
}
