//! Circuit breaker utilities for GCP service calls
//!
//! This module provides error classification for circuit breaker protection
//! of external GCP API calls (Cloud Tasks, Secret Manager, OAuth2).
//!
//! ## Error Classification
//!
//! - **Breaker-tripping errors**: Network/timeout, 5xx, 429 (rate limits)
//! - **Non-breaker errors**: 4xx configuration/auth errors (fail fast, don't trip breaker)

use reqwest::StatusCode;
use tracing::warn;

/// Classifies whether an error should trip the circuit breaker
///
/// # Circuit Breaker Policy
///
/// **Trip breaker** (external/transient failures):
/// - Network errors (connection refused, timeout, DNS)
/// - 5xx server errors (service unavailable, internal error)
/// - 429 rate limit exceeded (quota/throughput limits)
///
/// **Don't trip** (configuration/permanent failures):
/// - 4xx client errors except 429 (bad request, unauthorized, not found)
/// - These indicate misconfiguration and should fail fast
///
/// # Arguments
///
/// * `status` - Optional HTTP status code from response
/// * `is_network_error` - Whether this is a network-level error (connection, timeout, DNS)
///
/// # Returns
///
/// `true` if the error should trip the circuit breaker (external/transient failure)
pub fn should_trip_breaker(status: Option<StatusCode>, is_network_error: bool) -> bool {
    // Network errors always trip the breaker
    if is_network_error {
        return true;
    }

    // Classify based on HTTP status code
    match status {
        // 5xx server errors - trip breaker (service issues)
        Some(status) if status.is_server_error() => {
            warn!(
                status = %status,
                "Server error detected, will trip circuit breaker"
            );
            true
        }
        // 429 rate limit - trip breaker (quota exhausted, transient)
        Some(StatusCode::TOO_MANY_REQUESTS) => {
            warn!("Rate limit (429) detected, will trip circuit breaker");
            true
        }
        // 4xx client errors (except 429) - don't trip (config issues, fail fast)
        Some(status) if status.is_client_error() => {
            warn!(
                status = %status,
                "Client error detected, will NOT trip circuit breaker (config issue)"
            );
            false
        }
        // Other status codes or no status - don't trip
        _ => false,
    }
}

/// Helper to extract status code from reqwest::Error
pub fn extract_status(error: &reqwest::Error) -> Option<StatusCode> {
    error.status()
}

/// Helper to check if error is network-related
pub fn is_network_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_errors_trip_breaker() {
        // Network errors always trip
        assert!(should_trip_breaker(None, true));
        assert!(should_trip_breaker(Some(StatusCode::OK), true));
    }

    #[test]
    fn test_5xx_errors_trip_breaker() {
        // Server errors trip breaker
        assert!(should_trip_breaker(
            Some(StatusCode::INTERNAL_SERVER_ERROR),
            false
        ));
        assert!(should_trip_breaker(
            Some(StatusCode::SERVICE_UNAVAILABLE),
            false
        ));
        assert!(should_trip_breaker(Some(StatusCode::BAD_GATEWAY), false));
    }

    #[test]
    fn test_429_trips_breaker() {
        // Rate limit trips breaker (transient quota issue)
        assert!(should_trip_breaker(
            Some(StatusCode::TOO_MANY_REQUESTS),
            false
        ));
    }

    #[test]
    fn test_4xx_errors_dont_trip_breaker() {
        // Client errors (except 429) don't trip breaker
        assert!(!should_trip_breaker(Some(StatusCode::BAD_REQUEST), false));
        assert!(!should_trip_breaker(Some(StatusCode::UNAUTHORIZED), false));
        assert!(!should_trip_breaker(Some(StatusCode::FORBIDDEN), false));
        assert!(!should_trip_breaker(Some(StatusCode::NOT_FOUND), false));
    }

    #[test]
    fn test_2xx_success_dont_trip() {
        // Success codes don't trip
        assert!(!should_trip_breaker(Some(StatusCode::OK), false));
        assert!(!should_trip_breaker(Some(StatusCode::CREATED), false));
    }

    #[test]
    fn test_no_status_no_network_dont_trip() {
        // Unknown errors without status don't trip
        assert!(!should_trip_breaker(None, false));
    }
}
