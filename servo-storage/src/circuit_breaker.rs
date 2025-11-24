//! Circuit breaker utilities for database operations
//!
//! This module provides circuit breaker protection for PostgreSQL connection
//! pool acquisition to prevent cascading failures during database outages.
//!
//! ## Configuration
//!
//! Circuit breakers are configured via environment variables:
//! - `SERVO_CB_POSTGRES_FAILURE_THRESHOLD`: Number of consecutive failures to open circuit (default: 5)
//! - `SERVO_CB_POSTGRES_HALF_OPEN_TIMEOUT_SECS`: Half-open timeout in seconds (default: 30)
//!
//! ## Metrics
//!
//! - `servo_circuit_breaker_state{dependency="postgres"}`: Gauge (0=closed, 1=open)
//! - `servo_circuit_breaker_opens_total{dependency="postgres"}`: Counter
//!
//! ## States
//!
//! - **Closed**: Normal operation, requests pass through
//! - **Open**: Fail-fast mode, reject requests immediately
//! - **Half-Open**: Test mode (implicit), allow request after timeout to check recovery

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Circuit breaker configuration for database operations
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit
    pub failure_threshold: u32,
    /// Timeout before transitioning from open to half-open (allow retry)
    pub half_open_timeout: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5, // 5 consecutive failures
            half_open_timeout: Duration::from_secs(30),
        }
    }
}

impl CircuitBreakerConfig {
    /// Load configuration from environment variables with validation
    pub fn from_env(dependency: &str) -> Self {
        let failure_threshold = std::env::var(format!(
            "SERVO_CB_{}_FAILURE_THRESHOLD",
            dependency.to_uppercase()
        ))
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(5);

        let half_open_timeout_secs = std::env::var(format!(
            "SERVO_CB_{}_HALF_OPEN_TIMEOUT_SECS",
            dependency.to_uppercase()
        ))
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30);

        // Validate configuration
        let failure_threshold = if failure_threshold == 0 {
            warn!(
                dependency = %dependency,
                "Invalid failure_threshold=0, using default=5"
            );
            5
        } else {
            failure_threshold
        };

        let half_open_timeout_secs = if half_open_timeout_secs == 0 {
            warn!(
                dependency = %dependency,
                "Invalid half_open_timeout=0, using default=30"
            );
            30
        } else {
            half_open_timeout_secs
        };

        info!(
            dependency = %dependency,
            failure_threshold = %failure_threshold,
            half_open_timeout_secs = %half_open_timeout_secs,
            "Circuit breaker configuration validated and loaded"
        );

        Self {
            failure_threshold,
            half_open_timeout: Duration::from_secs(half_open_timeout_secs),
        }
    }
}

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitState {
    Closed,
    Open,
}

/// Internal state of the circuit breaker
struct CircuitBreakerState {
    state: CircuitState,
    consecutive_failures: u32,
    last_failure_time: Option<Instant>,
    /// Guard to prevent multiple concurrent half-open probes
    half_open_probe_in_progress: bool,
}

/// Database circuit breaker wrapper
///
/// Async-native circuit breaker for protecting database operations.
/// Uses consecutive failures policy to detect cascading failures.
pub struct DatabaseCircuitBreaker {
    config: CircuitBreakerConfig,
    dependency_name: String,
    state: Arc<RwLock<CircuitBreakerState>>,
}

impl DatabaseCircuitBreaker {
    /// Create a new database circuit breaker
    ///
    /// # Arguments
    ///
    /// * `dependency_name` - Name for metrics/logging (e.g., "postgres")
    /// * `config` - Circuit breaker configuration
    pub fn new(dependency_name: String, config: CircuitBreakerConfig) -> Self {
        info!(
            dependency = %dependency_name,
            "Initialized circuit breaker"
        );

        // Initialize metrics
        crate::metrics::CIRCUIT_BREAKER_STATE
            .with_label_values(&[&dependency_name])
            .set(0.0); // 0 = closed

        Self {
            config,
            dependency_name,
            state: Arc::new(RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                last_failure_time: None,
                half_open_probe_in_progress: false,
            })),
        }
    }

    /// Execute a function with circuit breaker protection
    ///
    /// If the circuit is open, returns immediately with CircuitOpen error.
    /// Otherwise, executes the function and records success/failure.
    ///
    /// # Arguments
    ///
    /// * `f` - Async function to execute
    ///
    /// # Returns
    ///
    /// Result of the function execution, or CircuitOpen error if breaker is open
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        // Check if circuit is open and handle half-open state
        let is_half_open_probe = {
            let mut state = self.state.write().await;
            if state.state == CircuitState::Open {
                // Check if enough time has passed to try again (half-open)
                if let Some(last_failure) = state.last_failure_time {
                    if last_failure.elapsed() < self.config.half_open_timeout {
                        // Still in open state, reject immediately
                        warn!(
                            dependency = %self.dependency_name,
                            time_since_open = ?last_failure.elapsed(),
                            "Circuit breaker is open, rejecting request"
                        );
                        return Err(CircuitBreakerError::CircuitOpen);
                    }

                    // Timeout elapsed, check if we can attempt a half-open probe
                    if state.half_open_probe_in_progress {
                        // Another probe is already in progress, reject
                        warn!(
                            dependency = %self.dependency_name,
                            "Half-open probe already in progress, rejecting request"
                        );
                        return Err(CircuitBreakerError::CircuitOpen);
                    }

                    // Set guard to prevent concurrent probes
                    state.half_open_probe_in_progress = true;
                    info!(
                        dependency = %self.dependency_name,
                        "Attempting half-open probe after timeout"
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        // Execute the function
        let result = f().await;

        // Handle result and update state
        match result {
            Ok(value) => {
                let mut state = self.state.write().await;
                let was_open = state.state == CircuitState::Open;

                // Success - reset failure count and close circuit
                state.state = CircuitState::Closed;
                state.consecutive_failures = 0;
                state.last_failure_time = None;
                state.half_open_probe_in_progress = false;

                if was_open && is_half_open_probe {
                    info!(
                        dependency = %self.dependency_name,
                        "Half-open probe succeeded, circuit closed"
                    );
                    // Track successful half-open probe
                    crate::metrics::CIRCUIT_BREAKER_HALF_OPEN_ATTEMPTS
                        .with_label_values(&[&self.dependency_name, "success"])
                        .inc();
                } else if was_open {
                    info!(
                        dependency = %self.dependency_name,
                        "Circuit breaker closed after successful request"
                    );
                }

                // Update state gauge
                crate::metrics::CIRCUIT_BREAKER_STATE
                    .with_label_values(&[&self.dependency_name])
                    .set(0.0); // 0 = closed

                Ok(value)
            }
            Err(e) => {
                let mut state = self.state.write().await;

                // Clear half-open probe guard if it was set
                if is_half_open_probe {
                    state.half_open_probe_in_progress = false;
                    warn!(
                        dependency = %self.dependency_name,
                        error = %e,
                        "Half-open probe failed, circuit remains open"
                    );
                    // Track failed half-open probe
                    crate::metrics::CIRCUIT_BREAKER_HALF_OPEN_ATTEMPTS
                        .with_label_values(&[&self.dependency_name, "failure"])
                        .inc();
                }

                // Increment failure counter
                state.consecutive_failures += 1;
                state.last_failure_time = Some(Instant::now());

                // Check if we should open the circuit
                if state.consecutive_failures >= self.config.failure_threshold
                    && state.state == CircuitState::Closed
                {
                    // Transition: Closed → Open
                    state.state = CircuitState::Open;
                    error!(
                        dependency = %self.dependency_name,
                        consecutive_failures = state.consecutive_failures,
                        threshold = self.config.failure_threshold,
                        error = %e,
                        "Circuit breaker transition: Closed → Open"
                    );

                    // Update metrics
                    crate::metrics::CIRCUIT_BREAKER_OPENS_TOTAL
                        .with_label_values(&[&self.dependency_name])
                        .inc();
                    crate::metrics::CIRCUIT_BREAKER_STATE
                        .with_label_values(&[&self.dependency_name])
                        .set(1.0); // 1 = open
                } else if state.state == CircuitState::Closed {
                    warn!(
                        dependency = %self.dependency_name,
                        consecutive_failures = state.consecutive_failures,
                        threshold = self.config.failure_threshold,
                        error = %e,
                        "Request failed, circuit breaker remains closed"
                    );
                }

                Err(CircuitBreakerError::Failure(e))
            }
        }
    }

    /// Check if the circuit breaker is currently open
    pub async fn is_open(&self) -> bool {
        let state = self.state.read().await;
        state.state == CircuitState::Open
            && state
                .last_failure_time
                .map(|t| t.elapsed() < self.config.half_open_timeout)
                .unwrap_or(false)
    }
}

/// Circuit breaker error type
#[derive(Debug, thiserror::Error)]
pub enum CircuitBreakerError<E> {
    /// Circuit breaker is open, request rejected
    #[error("Circuit breaker is open")]
    CircuitOpen,
    /// Underlying operation failed
    #[error("Operation failed: {0}")]
    Failure(E),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_closes_on_success() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            half_open_timeout: Duration::from_secs(1),
        };

        let breaker = DatabaseCircuitBreaker::new("test".to_string(), config);

        // Successful operation
        let result = breaker.call(|| async { Ok::<_, String>("success") }).await;
        assert!(result.is_ok());
        assert!(!breaker.is_open().await);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_on_failure_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3, // 3 consecutive failures
            half_open_timeout: Duration::from_secs(1),
        };

        let breaker = DatabaseCircuitBreaker::new("test".to_string(), config);

        // Trigger consecutive failures
        for i in 0..3 {
            let result = breaker
                .call(|| async { Err::<(), _>(format!("failure {}", i)) })
                .await;
            assert!(matches!(result, Err(CircuitBreakerError::Failure(_))));
        }

        // Circuit should be open now
        assert!(breaker.is_open().await);

        // Next request should be rejected immediately
        let result = breaker
            .call(|| async { Ok::<_, String>("should not execute") })
            .await;
        assert!(matches!(result, Err(CircuitBreakerError::CircuitOpen)));
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            half_open_timeout: Duration::from_millis(100), // Very short for test
        };

        let breaker = DatabaseCircuitBreaker::new("test".to_string(), config);

        // Open the circuit
        for _ in 0..2 {
            let _ = breaker.call(|| async { Err::<(), _>("failure") }).await;
        }

        assert!(breaker.is_open().await);

        // Wait for half-open timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Circuit should allow a test request (half-open state)
        // Successful request should close the circuit
        let result = breaker.call(|| async { Ok::<_, String>("recovery") }).await;
        assert!(result.is_ok());

        // Circuit should be closed now
        assert!(!breaker.is_open().await);

        // Next request should succeed
        let result = breaker.call(|| async { Ok::<_, String>("success") }).await;
        assert!(result.is_ok());
    }
}
