//! Rate limiting for worker endpoints
//!
//! Implements two types of rate limiting:
//! - **Per-tenant rate limiting** for /execute endpoint
//! - **IP-based rate limiting** for /metrics endpoint
//!
//! ## Configuration
//!
//! ```bash
//! # Per-tenant rate limiting (requests per second per tenant)
//! export SERVO_RATE_LIMIT_TENANT_RPS=10  # Default: 10
//!
//! # IP-based rate limiting for metrics (requests per minute per IP)
//! export SERVO_RATE_LIMIT_METRICS_RPM=60  # Default: 60
//! ```

use dashmap::DashMap;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, direct::NotKeyed},
    Quota, RateLimiter as GovernorRateLimiter,
};
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use tracing::{info, warn};

/// Per-tenant rate limiter configuration
#[derive(Debug, Clone)]
pub struct TenantRateLimiterConfig {
    /// Requests per second per tenant
    pub requests_per_second: u32,
}

impl Default for TenantRateLimiterConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 10,
        }
    }
}

impl TenantRateLimiterConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let requests_per_second = std::env::var("SERVO_RATE_LIMIT_TENANT_RPS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(10);

        info!(
            requests_per_second = requests_per_second,
            "Tenant rate limiter configuration loaded"
        );

        Self {
            requests_per_second,
        }
    }
}

/// IP-based rate limiter configuration
#[derive(Debug, Clone)]
pub struct IpRateLimiterConfig {
    /// Requests per minute per IP
    pub requests_per_minute: u32,
}

impl Default for IpRateLimiterConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
        }
    }
}

impl IpRateLimiterConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let requests_per_minute = std::env::var("SERVO_RATE_LIMIT_METRICS_RPM")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(60);

        info!(
            requests_per_minute = requests_per_minute,
            "IP rate limiter configuration loaded"
        );

        Self {
            requests_per_minute,
        }
    }
}

/// Per-tenant rate limiter
///
/// Enforces separate rate limits for each tenant using in-memory storage.
/// Rate limiters are created lazily on first request from a tenant.
pub struct TenantRateLimiter {
    config: TenantRateLimiterConfig,
    limiters: Arc<DashMap<String, GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
}

impl TenantRateLimiter {
    /// Create a new tenant rate limiter
    pub fn new(config: TenantRateLimiterConfig) -> Self {
        info!(
            requests_per_second = config.requests_per_second,
            "Initialized tenant rate limiter"
        );

        Self {
            config,
            limiters: Arc::new(DashMap::new()),
        }
    }

    /// Check if a request from a tenant should be allowed
    ///
    /// Returns `Ok(())` if the request is allowed, `Err(())` if rate limited.
    pub fn check_tenant(&self, tenant_id: &str) -> Result<(), RateLimitError> {
        // Get or create rate limiter for this tenant
        let limiter = self.limiters.entry(tenant_id.to_string()).or_insert_with(|| {
            let quota = Quota::per_second(
                NonZeroU32::new(self.config.requests_per_second)
                    .expect("requests_per_second must be non-zero"),
            );
            GovernorRateLimiter::direct(quota)
        });

        // Check rate limit
        match limiter.check() {
            Ok(_) => Ok(()),
            Err(_not_until) => {
                warn!(
                    tenant_id = %tenant_id,
                    limit = self.config.requests_per_second,
                    "Tenant rate limit exceeded"
                );
                Err(RateLimitError::TenantLimitExceeded {
                    tenant_id: tenant_id.to_string(),
                    limit_per_second: self.config.requests_per_second,
                })
            }
        }
    }

    /// Get the number of active tenant rate limiters
    ///
    /// Useful for monitoring memory usage.
    pub fn active_limiters_count(&self) -> usize {
        self.limiters.len()
    }
}

/// IP-based rate limiter
///
/// Enforces rate limits per IP address for the metrics endpoint.
/// Uses DashMap for per-IP state storage.
pub struct IpRateLimiter {
    config: IpRateLimiterConfig,
    limiters: Arc<DashMap<IpAddr, GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock>>>,
}

impl IpRateLimiter {
    /// Create a new IP rate limiter
    pub fn new(config: IpRateLimiterConfig) -> Self {
        info!(
            requests_per_minute = config.requests_per_minute,
            "Initialized IP rate limiter"
        );

        Self {
            config,
            limiters: Arc::new(DashMap::new()),
        }
    }

    /// Check if a request from an IP should be allowed
    ///
    /// Returns `Ok(())` if the request is allowed, `Err(())` if rate limited.
    pub fn check_ip(&self, ip: IpAddr) -> Result<(), RateLimitError> {
        // Get or create rate limiter for this IP
        let limiter = self.limiters.entry(ip).or_insert_with(|| {
            let quota = Quota::per_minute(
                NonZeroU32::new(self.config.requests_per_minute)
                    .expect("requests_per_minute must be non-zero"),
            )
            .allow_burst(
                NonZeroU32::new(self.config.requests_per_minute / 2)
                    .unwrap_or(NonZeroU32::new(5).unwrap()),
            );
            GovernorRateLimiter::direct(quota)
        });

        // Check rate limit
        match limiter.check() {
            Ok(_) => Ok(()),
            Err(_not_until) => {
                warn!(
                    ip = %ip,
                    limit = self.config.requests_per_minute,
                    "IP rate limit exceeded"
                );
                Err(RateLimitError::IpLimitExceeded {
                    ip,
                    limit_per_minute: self.config.requests_per_minute,
                })
            }
        }
    }
}

/// Rate limiting error
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("Tenant rate limit exceeded: {tenant_id} ({limit_per_second} req/s)")]
    TenantLimitExceeded {
        tenant_id: String,
        limit_per_second: u32,
    },

    #[error("IP rate limit exceeded: {ip} ({limit_per_minute} req/min)")]
    IpLimitExceeded {
        ip: IpAddr,
        limit_per_minute: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_tenant_rate_limiter_allows_within_limit() {
        let config = TenantRateLimiterConfig {
            requests_per_second: 10,
        };
        let limiter = TenantRateLimiter::new(config);

        // First request should be allowed
        assert!(limiter.check_tenant("tenant-1").is_ok());
    }

    #[test]
    fn test_tenant_rate_limiter_blocks_over_limit() {
        let config = TenantRateLimiterConfig {
            requests_per_second: 2,
        };
        let limiter = TenantRateLimiter::new(config);

        // First 2 requests should be allowed
        assert!(limiter.check_tenant("tenant-1").is_ok());
        assert!(limiter.check_tenant("tenant-1").is_ok());

        // 3rd request should be rate limited
        assert!(limiter.check_tenant("tenant-1").is_err());
    }

    #[test]
    fn test_tenant_rate_limiter_isolates_tenants() {
        let config = TenantRateLimiterConfig {
            requests_per_second: 2,
        };
        let limiter = TenantRateLimiter::new(config);

        // Exhaust limit for tenant-1
        assert!(limiter.check_tenant("tenant-1").is_ok());
        assert!(limiter.check_tenant("tenant-1").is_ok());
        assert!(limiter.check_tenant("tenant-1").is_err());

        // tenant-2 should still have capacity
        assert!(limiter.check_tenant("tenant-2").is_ok());
        assert!(limiter.check_tenant("tenant-2").is_ok());
    }

    #[test]
    fn test_ip_rate_limiter_allows_within_limit() {
        let config = IpRateLimiterConfig {
            requests_per_minute: 60,
        };
        let limiter = IpRateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        // First request should be allowed
        assert!(limiter.check_ip(ip).is_ok());
    }

    #[test]
    fn test_ip_rate_limiter_blocks_over_limit() {
        let config = IpRateLimiterConfig {
            requests_per_minute: 10,
        };
        let limiter = IpRateLimiter::new(config);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        // Allow initial burst of requests (quota + burst capacity)
        let mut allowed_count = 0;
        for _ in 0..20 {
            if limiter.check_ip(ip).is_ok() {
                allowed_count += 1;
            }
        }

        // At least the first request should be allowed
        assert!(allowed_count >= 1, "At least one request should be allowed");

        // Keep requesting until we hit rate limit
        let mut hit_rate_limit = false;
        for _ in 0..100 {
            if limiter.check_ip(ip).is_err() {
                hit_rate_limit = true;
                break;
            }
        }

        assert!(hit_rate_limit, "Should eventually hit rate limit after many requests");
    }

    #[test]
    fn test_active_limiters_count() {
        let config = TenantRateLimiterConfig {
            requests_per_second: 10,
        };
        let limiter = TenantRateLimiter::new(config);

        assert_eq!(limiter.active_limiters_count(), 0);

        limiter.check_tenant("tenant-1").ok();
        assert_eq!(limiter.active_limiters_count(), 1);

        limiter.check_tenant("tenant-2").ok();
        assert_eq!(limiter.active_limiters_count(), 2);

        // Same tenant doesn't increase count
        limiter.check_tenant("tenant-1").ok();
        assert_eq!(limiter.active_limiters_count(), 2);
    }
}
