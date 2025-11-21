//! Concurrency control for workflow executions

use std::sync::Arc;
use tokio::sync::Semaphore;

/// Concurrency limiter for controlling parallel executions
pub struct ConcurrencyLimiter {
    semaphore: Arc<Semaphore>,
    max_concurrent: usize,
}

impl ConcurrencyLimiter {
    /// Create a new concurrency limiter
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
        }
    }

    /// Acquire a permit for execution
    pub async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.semaphore
            .acquire()
            .await
            .expect("Semaphore closed unexpectedly")
    }

    /// Get the maximum concurrent executions
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Get the current available permits
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_concurrency_limiter() {
        let limiter = ConcurrencyLimiter::new(2);
        assert_eq!(limiter.available(), 2);

        let _permit1 = limiter.acquire().await;
        assert_eq!(limiter.available(), 1);

        let _permit2 = limiter.acquire().await;
        assert_eq!(limiter.available(), 0);
    }
}
