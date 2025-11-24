//! Health check utilities for GCP services
//!
//! Provides cached health checks to avoid excessive API calls during readiness probes.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cached health check result
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    pub healthy: bool,
    pub checked_at: Instant,
    pub error_message: Option<String>,
}

/// Cached health checker with TTL
///
/// Caches health check results to avoid excessive API calls during readiness probes.
/// Results are cached for a configurable TTL (default: 30 seconds).
pub struct CachedHealthCheck {
    /// Last health check result
    last_result: Arc<RwLock<Option<HealthCheckResult>>>,
    /// Cache TTL duration
    ttl: Duration,
}

impl CachedHealthCheck {
    /// Create a new cached health check with the specified TTL
    pub fn new(ttl: Duration) -> Self {
        Self {
            last_result: Arc::new(RwLock::new(None)),
            ttl,
        }
    }

    /// Create a new cached health check with default 30-second TTL
    pub fn with_default_ttl() -> Self {
        Self::new(Duration::from_secs(30))
    }

    /// Check if the cached result is still valid
    async fn is_cache_valid(&self) -> bool {
        let result = self.last_result.read().await;
        match result.as_ref() {
            Some(r) => r.checked_at.elapsed() < self.ttl,
            None => false,
        }
    }

    /// Get the cached health check result, or run a new check if cache is stale
    ///
    /// # Arguments
    ///
    /// * `check_fn` - Async function that performs the actual health check
    pub async fn check<F, Fut>(&self, check_fn: F) -> HealthCheckResult
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        // Check if cache is still valid
        if self.is_cache_valid().await {
            let result = self.last_result.read().await;
            return result.as_ref().unwrap().clone();
        }

        // Cache is stale or missing, perform new check
        let check_result = match check_fn().await {
            Ok(()) => HealthCheckResult {
                healthy: true,
                checked_at: Instant::now(),
                error_message: None,
            },
            Err(err) => HealthCheckResult {
                healthy: false,
                checked_at: Instant::now(),
                error_message: Some(err),
            },
        };

        // Update cache
        let mut last_result = self.last_result.write().await;
        *last_result = Some(check_result.clone());

        check_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_cache_validity() {
        let health_check = CachedHealthCheck::new(Duration::from_millis(100));

        // First check should not be cached
        assert!(!health_check.is_cache_valid().await);

        // Perform a check
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();
        health_check
            .check(|| async move {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;

        // Should be cached now
        assert!(health_check.is_cache_valid().await);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second check should use cache (counter doesn't increment)
        let counter_clone = counter.clone();
        health_check
            .check(|| async move {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 1); // Still 1, used cache

        // Wait for cache to expire
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!health_check.is_cache_valid().await);

        // Third check should call function again
        let counter_clone = counter.clone();
        health_check
            .check(|| async move {
                counter_clone.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(counter.load(Ordering::SeqCst), 2); // Incremented to 2
    }

    #[tokio::test]
    async fn test_error_caching() {
        let health_check = CachedHealthCheck::new(Duration::from_secs(60));

        // Check that fails
        let result = health_check
            .check(|| async { Err("Service unavailable".to_string()) })
            .await;

        assert!(!result.healthy);
        assert_eq!(result.error_message, Some("Service unavailable".to_string()));

        // Cache should still be valid even for failures
        assert!(health_check.is_cache_valid().await);
    }
}
