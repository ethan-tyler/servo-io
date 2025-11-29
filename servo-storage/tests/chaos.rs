//! Chaos Engineering Tests
//!
//! These tests validate system resilience under failure conditions.
//! They simulate various failure scenarios to ensure proper behavior
//! of resilience patterns like circuit breakers, retries, and timeouts.

#[path = "chaos/circuit_breaker_tests.rs"]
mod circuit_breaker_tests;

#[path = "chaos/timeout_tests.rs"]
mod timeout_tests;
