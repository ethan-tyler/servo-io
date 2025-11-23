//! Retry logic for failed executions
//!
//! Implements retry strategies with exponential backoff and jitter to avoid
//! thundering herd problems. Uses the "full jitter" algorithm recommended by AWS:
//! delay = random(0, base * 2^attempt)
//!
//! References:
//! - https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/

use rand::Rng;
use std::future::Future;
use std::time::{Duration, Instant};

/// Retry strategy for failed executions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStrategy {
    /// No retries
    None,

    /// Fixed delay between retries
    Fixed,

    /// Exponential backoff
    Exponential,

    /// Exponential backoff with jitter
    ExponentialWithJitter,
}

/// Retry policy configuration
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    pub max_attempts: u32,

    /// Initial delay before first retry (base delay for exponential backoff)
    pub initial_delay: Duration,

    /// Maximum delay between retries (caps exponential growth)
    pub max_delay: Duration,

    /// Maximum total elapsed time for all retries
    /// If retries exceed this duration, stop retrying even if max_attempts not reached
    pub max_elapsed: Duration,

    /// Retry strategy to use
    pub strategy: RetryStrategy,

    /// Multiplier for exponential backoff (typically 2.0)
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            max_elapsed: Duration::from_secs(300), // 5 minutes total
            strategy: RetryStrategy::ExponentialWithJitter,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Calculate the delay for a given attempt
    ///
    /// # Arguments
    ///
    /// * `attempt` - Zero-based attempt number (0 = first retry)
    ///
    /// # Returns
    ///
    /// The duration to wait before the next retry
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        match self.strategy {
            RetryStrategy::None => Duration::from_secs(0),
            RetryStrategy::Fixed => self.initial_delay,
            RetryStrategy::Exponential => {
                let delay =
                    self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
                Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
            }
            RetryStrategy::ExponentialWithJitter => self.calculate_delay_with_jitter(attempt),
        }
    }

    /// Calculate delay with full jitter
    ///
    /// Uses the "full jitter" algorithm: `delay = random(0, min(cap, base * 2^attempt))`
    /// This distributes retry attempts more evenly to avoid thundering herd.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Zero-based attempt number (0 = first retry)
    ///
    /// # Returns
    ///
    /// A randomized duration between 0 and the calculated exponential backoff delay
    pub fn calculate_delay_with_jitter(&self, attempt: u32) -> Duration {
        // Calculate exponential delay: base * multiplier^attempt
        let exponential_delay =
            self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);

        // Cap at max_delay
        let capped_delay = exponential_delay.min(self.max_delay.as_secs_f64());

        // Apply full jitter: random(0, capped_delay)
        let jittered_delay_ms = if capped_delay > 0.0 {
            let max_ms = (capped_delay * 1000.0) as u64;
            rand::thread_rng().gen_range(0..=max_ms)
        } else {
            0
        };

        Duration::from_millis(jittered_delay_ms)
    }

    /// Check if should retry after given attempt number
    ///
    /// # Arguments
    ///
    /// * `attempt` - Zero-based attempt number
    ///
    /// # Returns
    ///
    /// `true` if should retry, `false` otherwise
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }

    /// Check if total elapsed time exceeds max_elapsed
    ///
    /// # Arguments
    ///
    /// * `start` - Time when retries started
    ///
    /// # Returns
    ///
    /// `true` if elapsed time exceeds max_elapsed, `false` otherwise
    pub fn is_elapsed_exceeded(&self, start: Instant) -> bool {
        start.elapsed() >= self.max_elapsed
    }

    /// Execute a function with retry logic
    ///
    /// Automatically retries transient errors with exponential backoff + jitter.
    /// Stops retrying if:
    /// - The function succeeds
    /// - max_attempts is reached
    /// - max_elapsed time is exceeded
    /// - A non-retryable error occurs
    ///
    /// # Arguments
    ///
    /// * `f` - Function to execute (must be repeatable/idempotent)
    ///
    /// # Returns
    ///
    /// Result from the function execution
    ///
    /// # Example
    ///
    /// ```no_run
    /// use servo_runtime::RetryPolicy;
    /// # async fn example() {
    /// let policy = RetryPolicy::default();
    /// let result = policy.execute_with_retry(|| async {
    ///     // Your async operation here
    ///     Ok::<_, servo_runtime::Error>(42)
    /// }).await;
    /// # }
    /// ```
    pub async fn execute_with_retry<F, Fut, T>(&self, mut f: F) -> crate::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = crate::Result<T>>,
    {
        let start = Instant::now();
        let mut attempt = 0;

        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    attempt += 1;

                    // Check if error is retryable - bail out early on permanent errors
                    if !Self::is_retryable(&e) {
                        tracing::debug!(
                            error = %e,
                            "Non-retryable error, failing immediately"
                        );
                        return Err(e);
                    }

                    // Check if we should give up
                    // attempt is now 1-indexed (1 = first attempt, 2 = second attempt, etc.)
                    if attempt >= self.max_attempts {
                        tracing::debug!(
                            attempt,
                            max_attempts = self.max_attempts,
                            "Max attempts reached"
                        );
                        return Err(e);
                    }

                    if self.is_elapsed_exceeded(start) {
                        tracing::debug!(
                            elapsed_ms = start.elapsed().as_millis(),
                            max_elapsed_ms = self.max_elapsed.as_millis(),
                            "Max elapsed time exceeded"
                        );
                        return Err(e);
                    }

                    // Calculate delay and wait (attempt - 1 for 0-indexed delay calculation)
                    let delay = self.calculate_delay(attempt - 1);
                    tracing::debug!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        error = %e,
                        "Retrying after delay"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Check if an error is retryable
    ///
    /// This is a helper method to classify errors. In a real application,
    /// you would check the specific error type to determine if it's transient.
    ///
    /// # Arguments
    ///
    /// * `error` - The error to check
    ///
    /// # Returns
    ///
    /// `true` if the error is retryable (transient), `false` otherwise
    ///
    /// # Examples of retryable errors:
    /// - Database connection timeout
    /// - Connection pool exhausted
    /// - Network timeout
    /// - Rate limit exceeded (with backoff)
    ///
    /// # Examples of non-retryable errors:
    /// - Invalid input / validation errors
    /// - Permission denied / authorization errors
    /// - Resource not found
    /// - Duplicate key / unique constraint violation
    pub fn is_retryable(error: &crate::Error) -> bool {
        use crate::Error;

        match error {
            // Retryable errors (transient)
            Error::Internal(msg) => {
                // Check if the message indicates a transient error
                let msg_lower = msg.to_lowercase();
                msg_lower.contains("connection")
                    || msg_lower.contains("timeout")
                    || msg_lower.contains("pool")
                    || msg_lower.contains("transient")
            }
            Error::Timeout(_) => true,

            // Non-retryable errors (permanent)
            Error::Execution(_) => false,       // Business logic errors
            Error::NotFound(_) => false,        // Resource doesn't exist
            Error::InvalidState(_) => false,    // Data corruption
            Error::TenantViolation(_) => false, // Security/auth error
            Error::RetryExhausted(_) => false,  // Already retried
            Error::ConcurrencyLimitExceeded(_) => false, // Rate limiting
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.initial_delay, Duration::from_secs(1));
        assert_eq!(policy.max_delay, Duration::from_secs(60));
        assert_eq!(policy.max_elapsed, Duration::from_secs(300));
    }

    #[test]
    fn test_should_retry() {
        let policy = RetryPolicy::default();
        assert!(policy.should_retry(0));
        assert!(policy.should_retry(2));
        assert!(!policy.should_retry(3));
    }

    #[test]
    fn test_fixed_delay() {
        let policy = RetryPolicy {
            strategy: RetryStrategy::Fixed,
            initial_delay: Duration::from_secs(5),
            ..Default::default()
        };

        assert_eq!(policy.calculate_delay(0), Duration::from_secs(5));
        assert_eq!(policy.calculate_delay(1), Duration::from_secs(5));
    }

    #[test]
    fn test_exponential_delay() {
        let policy = RetryPolicy {
            strategy: RetryStrategy::Exponential,
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(60),
            ..Default::default()
        };

        assert_eq!(policy.calculate_delay(0), Duration::from_secs(1)); // 1 * 2^0 = 1
        assert_eq!(policy.calculate_delay(1), Duration::from_secs(2)); // 1 * 2^1 = 2
        assert_eq!(policy.calculate_delay(2), Duration::from_secs(4)); // 1 * 2^2 = 4
        assert_eq!(policy.calculate_delay(3), Duration::from_secs(8)); // 1 * 2^3 = 8
    }

    #[test]
    fn test_exponential_delay_capped() {
        let policy = RetryPolicy {
            strategy: RetryStrategy::Exponential,
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(5),
            ..Default::default()
        };

        assert_eq!(policy.calculate_delay(0), Duration::from_secs(1)); // 1
        assert_eq!(policy.calculate_delay(1), Duration::from_secs(2)); // 2
        assert_eq!(policy.calculate_delay(2), Duration::from_secs(4)); // 4
        assert_eq!(policy.calculate_delay(3), Duration::from_secs(5)); // 8 capped to 5
        assert_eq!(policy.calculate_delay(10), Duration::from_secs(5)); // Stays capped
    }

    #[test]
    fn test_jitter_delay_is_random() {
        let policy = RetryPolicy {
            strategy: RetryStrategy::ExponentialWithJitter,
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(60),
            ..Default::default()
        };

        // Run multiple times to ensure jitter produces different values
        let delays: Vec<Duration> = (0..10)
            .map(|_| policy.calculate_delay_with_jitter(2))
            .collect();

        // Check that we got at least some variation (not all the same)
        let unique_delays: std::collections::HashSet<_> = delays.iter().collect();
        assert!(
            unique_delays.len() > 1,
            "Jitter should produce varied delays"
        );

        // Check all delays are within bounds (0 to 4 seconds for attempt 2)
        for delay in delays {
            assert!(delay <= Duration::from_secs(4));
        }
    }

    #[test]
    fn test_jitter_delay_distribution() {
        let policy = RetryPolicy {
            strategy: RetryStrategy::ExponentialWithJitter,
            initial_delay: Duration::from_secs(1),
            backoff_multiplier: 2.0,
            max_delay: Duration::from_secs(60),
            ..Default::default()
        };

        // Generate many samples for attempt 3 (should be 0-8 seconds)
        let samples: Vec<u64> = (0..1000)
            .map(|_| policy.calculate_delay_with_jitter(3).as_millis() as u64)
            .collect();

        // Calculate mean (should be around 4000ms for uniform distribution over 0-8000ms)
        let mean = samples.iter().sum::<u64>() / samples.len() as u64;

        // Mean should be roughly half of max (8000ms / 2 = 4000ms)
        // Allow 20% tolerance due to randomness
        assert!(
            mean > 3200 && mean < 4800,
            "Mean should be ~4000ms, got {}ms",
            mean
        );
    }

    #[test]
    fn test_elapsed_time_check() {
        let policy = RetryPolicy {
            max_elapsed: Duration::from_millis(100),
            ..Default::default()
        };

        let start = Instant::now();
        assert!(!policy.is_elapsed_exceeded(start));

        std::thread::sleep(Duration::from_millis(150));
        assert!(policy.is_elapsed_exceeded(start));
    }

    #[tokio::test]
    async fn test_execute_with_retry_success() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay: Duration::from_millis(10),
            ..Default::default()
        };

        let mut attempt_count = 0;
        let result = policy
            .execute_with_retry(|| {
                attempt_count += 1;
                async move {
                    if attempt_count < 2 {
                        Err(crate::Error::Internal("transient error".to_string()))
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(42));
        assert_eq!(attempt_count, 2); // Failed once, succeeded on retry
    }

    #[tokio::test]
    async fn test_execute_with_retry_max_attempts() {
        let policy = RetryPolicy {
            max_attempts: 2,
            initial_delay: Duration::from_millis(10),
            max_elapsed: Duration::from_secs(60),
            ..Default::default()
        };

        let mut attempt_count = 0;
        let result = policy
            .execute_with_retry(|| {
                attempt_count += 1;
                async move { Err::<i32, _>(crate::Error::Internal("transient error".to_string())) }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempt_count, 2); // Initial + 1 retry (max_attempts = 2)
    }

    #[tokio::test]
    async fn test_execute_with_retry_max_elapsed() {
        let policy = RetryPolicy {
            max_attempts: 100,
            initial_delay: Duration::from_millis(50),
            max_elapsed: Duration::from_millis(120),
            ..Default::default()
        };

        let result = policy
            .execute_with_retry(|| async {
                Err::<i32, _>(crate::Error::Internal("transient error".to_string()))
            })
            .await;

        assert!(result.is_err());
    }

    #[test]
    fn test_is_retryable() {
        use crate::Error;

        // Retryable errors (transient)
        assert!(RetryPolicy::is_retryable(&Error::Internal(
            "connection timeout".to_string()
        )));
        assert!(RetryPolicy::is_retryable(&Error::Internal(
            "connection pool exhausted".to_string()
        )));
        assert!(RetryPolicy::is_retryable(&Error::Timeout(
            "operation timeout".to_string()
        )));
        assert!(RetryPolicy::is_retryable(&Error::Internal(
            "Transient storage error".to_string()
        )));

        // Non-retryable errors (permanent)
        assert!(!RetryPolicy::is_retryable(&Error::Execution(
            "invalid state transition".to_string()
        )));
        assert!(!RetryPolicy::is_retryable(&Error::NotFound(
            "execution not found".to_string()
        )));
        assert!(!RetryPolicy::is_retryable(&Error::InvalidState(
            "unknown state in database".to_string()
        )));
        assert!(!RetryPolicy::is_retryable(&Error::TenantViolation(
            "cross-tenant access denied".to_string()
        )));
        assert!(!RetryPolicy::is_retryable(&Error::RetryExhausted(
            "max retries".to_string()
        )));
        assert!(!RetryPolicy::is_retryable(
            &Error::ConcurrencyLimitExceeded("too many concurrent executions".to_string())
        ));
    }
}
