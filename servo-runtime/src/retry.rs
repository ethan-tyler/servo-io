//! Retry logic for failed executions

use std::time::Duration;

/// Retry strategy for failed executions
#[derive(Debug, Clone, Copy)]
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

    /// Initial delay before first retry
    pub initial_delay: Duration,

    /// Maximum delay between retries
    pub max_delay: Duration,

    /// Retry strategy to use
    pub strategy: RetryStrategy,

    /// Multiplier for exponential backoff
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            strategy: RetryStrategy::ExponentialWithJitter,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Calculate the delay for a given attempt
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        match self.strategy {
            RetryStrategy::None => Duration::from_secs(0),
            RetryStrategy::Fixed => self.initial_delay,
            RetryStrategy::Exponential => {
                let delay =
                    self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
                Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
            }
            RetryStrategy::ExponentialWithJitter => {
                // TODO: Add jitter to exponential backoff
                let delay =
                    self.initial_delay.as_secs_f64() * self.backoff_multiplier.powi(attempt as i32);
                Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
            }
        }
    }

    /// Check if should retry after given attempt number
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 3);
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
}
