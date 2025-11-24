//! GCP Secret Manager integration for HMAC secrets
//!
//! This module provides:
//! - Secure secret fetching from GCP Secret Manager
//! - Secret rotation support (multiple valid secrets during rotation)
//! - Automatic refresh without restart (5-minute polling interval)
//! - Startup validation (minimum 32-byte length, entropy check)
//! - Fail-closed behavior on missing/invalid secrets

use crate::auth::GcpAuth;
use crate::{Error, Result};
use base64::Engine as _;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Minimum secret length in bytes (256 bits)
const MIN_SECRET_LENGTH: usize = 32;

/// Secret refresh interval (5 minutes)
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Secret version from GCP Secret Manager
#[derive(Debug, Clone)]
pub struct SecretVersion {
    /// The secret value
    pub value: String,
    /// Version identifier (e.g., "latest", "1", "2")
    pub version: String,
    /// When this secret was fetched
    pub fetched_at: Instant,
}

/// Response from Secret Manager API
#[derive(Debug, Deserialize)]
struct SecretVersionResponse {
    name: String,
    payload: SecretPayload,
}

#[derive(Debug, Deserialize)]
struct SecretPayload {
    data: String, // Base64-encoded secret data
}

/// Secret manager for HMAC secrets with rotation support
///
/// Supports multiple valid secrets simultaneously to enable zero-downtime rotation:
/// 1. Add new secret version in GCP Secret Manager
/// 2. SecretManager fetches both current and previous versions
/// 3. HMAC verification accepts either secret
/// 4. After propagation delay, remove old secret version
pub struct SecretManager {
    /// GCP project ID
    project_id: String,
    /// Secret name (e.g., "servo-hmac-secret")
    secret_name: String,
    /// GCP auth for API calls
    auth: Arc<GcpAuth>,
    /// HTTP client with timeout
    http_client: reqwest::Client,
    /// Currently valid secrets (current + previous for rotation)
    valid_secrets: Arc<RwLock<Vec<SecretVersion>>>,
    /// Last refresh time
    last_refresh: Arc<RwLock<Instant>>,
    /// Circuit breaker for Secret Manager API
    circuit_breaker: servo_storage::circuit_breaker::DatabaseCircuitBreaker,
}

impl SecretManager {
    /// Create a new secret manager
    ///
    /// # Arguments
    ///
    /// * `project_id` - GCP project ID
    /// * `secret_name` - Name of the secret in Secret Manager
    /// * `auth` - GCP authentication
    ///
    /// # Returns
    ///
    /// A SecretManager instance that will automatically refresh secrets
    pub fn new(project_id: String, secret_name: String, auth: Arc<GcpAuth>) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("Failed to create HTTP client: {}", e)))?;

        // Initialize circuit breaker for Secret Manager API
        let circuit_breaker_config =
            servo_storage::circuit_breaker::CircuitBreakerConfig::from_env("secret_manager");
        let circuit_breaker = servo_storage::circuit_breaker::DatabaseCircuitBreaker::new(
            "secret_manager".to_string(),
            circuit_breaker_config,
        );

        Ok(Self {
            project_id,
            secret_name,
            auth,
            http_client,
            valid_secrets: Arc::new(RwLock::new(Vec::new())),
            last_refresh: Arc::new(RwLock::new(Instant::now() - REFRESH_INTERVAL)), // Force initial fetch
            circuit_breaker,
        })
    }

    /// Initialize secrets with startup validation
    ///
    /// Fetches secrets and validates them before allowing the service to start.
    /// Fails closed if secrets are missing or invalid.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Secret fetch fails
    /// - Secret is too short (< 32 bytes)
    /// - Secret has insufficient entropy
    pub async fn initialize(&self) -> Result<()> {
        info!(
            project = %self.project_id,
            secret = %self.secret_name,
            "Initializing secrets from GCP Secret Manager"
        );

        self.refresh_secrets().await?;

        let secrets = self.valid_secrets.read().await;
        if secrets.is_empty() {
            return Err(Error::Config(
                "No valid secrets found in Secret Manager".to_string(),
            ));
        }

        info!(count = secrets.len(), "Secrets initialized successfully");

        Ok(())
    }

    /// Refresh secrets from GCP Secret Manager
    ///
    /// Fetches the "latest" version and optionally the previous version
    /// to support rotation. Called automatically every 5 minutes.
    async fn refresh_secrets(&self) -> Result<()> {
        // Fetch latest secret version
        let latest = self.fetch_secret_version("latest").await?;

        // Validate secret quality
        Self::validate_secret(&latest.value)?;

        let mut new_secrets = vec![latest.clone()];

        // Try to fetch previous version for rotation support
        // Don't fail if it doesn't exist (single version is OK)
        if let Ok(previous) = self.fetch_secret_version("1").await {
            if previous.version != latest.version {
                if let Err(e) = Self::validate_secret(&previous.value) {
                    warn!(
                        error = %e,
                        "Previous secret version is invalid, ignoring"
                    );
                } else {
                    new_secrets.push(previous);
                    info!("Loaded previous secret version for rotation support");
                }
            }
        }

        // Update valid secrets
        let mut secrets = self.valid_secrets.write().await;
        *secrets = new_secrets;

        // Update refresh timestamp
        let mut last_refresh = self.last_refresh.write().await;
        *last_refresh = Instant::now();

        info!(count = secrets.len(), "Secrets refreshed successfully");

        Ok(())
    }

    /// Fetch a specific secret version from GCP Secret Manager
    async fn fetch_secret_version(&self, version: &str) -> Result<SecretVersion> {
        let secret_path = format!(
            "projects/{}/secrets/{}/versions/{}",
            self.project_id, self.secret_name, version
        );

        let api_url = format!(
            "https://secretmanager.googleapis.com/v1/{}:access",
            secret_path
        );

        // Get access token
        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        // Wrap HTTP call with circuit breaker
        let http_client = &self.http_client;
        let api_url_clone = api_url.to_string();
        let access_token_clone = access_token.to_string();

        let response_result = self
            .circuit_breaker
            .call(|| async move {
                http_client
                    .get(&api_url_clone)
                    .header("Authorization", format!("Bearer {}", access_token_clone))
                    .send()
                    .await
            })
            .await;

        // Handle circuit breaker result
        let response = match response_result {
            Ok(resp) => resp,
            Err(servo_storage::circuit_breaker::CircuitBreakerError::CircuitOpen) => {
                tracing::warn!("Circuit breaker is open for Secret Manager API");
                return Err(Error::Internal(
                    "Secret Manager API circuit breaker is open".to_string(),
                ));
            }
            Err(servo_storage::circuit_breaker::CircuitBreakerError::Failure(e)) => {
                // Classify error to determine if we should trip breaker
                let status = crate::circuit_breaker::extract_status(&e);
                let is_network = crate::circuit_breaker::is_network_error(&e);
                let should_trip = crate::circuit_breaker::should_trip_breaker(status, is_network);

                tracing::warn!(
                    status = ?status,
                    is_network = is_network,
                    should_trip_breaker = should_trip,
                    error = %e,
                    "Secret Manager API HTTP request failed"
                );

                return Err(Error::Internal(format!(
                    "Secret Manager API call failed: {}",
                    e
                )));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Internal(format!(
                "Secret Manager returned {}: {}",
                status, body
            )));
        }

        let secret_response: SecretVersionResponse = response
            .json()
            .await
            .map_err(|e| Error::Internal(format!("Failed to parse secret response: {}", e)))?;

        // Decode base64 secret data
        let secret_bytes = base64::engine::general_purpose::STANDARD
            .decode(&secret_response.payload.data)
            .map_err(|e| Error::Internal(format!("Failed to decode secret: {}", e)))?;

        let secret_value = String::from_utf8(secret_bytes)
            .map_err(|e| Error::Internal(format!("Secret is not valid UTF-8: {}", e)))?;

        // Extract version from response name (e.g., "projects/*/secrets/*/versions/1" -> "1")
        let version = secret_response
            .name
            .rsplit('/')
            .next()
            .unwrap_or("unknown")
            .to_string();

        Ok(SecretVersion {
            value: secret_value,
            version,
            fetched_at: Instant::now(),
        })
    }

    /// Validate secret quality
    ///
    /// Checks:
    /// - Minimum length (32 bytes / 256 bits)
    /// - Basic entropy (not all same character)
    fn validate_secret(secret: &str) -> Result<()> {
        // Check minimum length
        if secret.len() < MIN_SECRET_LENGTH {
            return Err(Error::Config(format!(
                "Secret is too short: {} bytes (minimum {})",
                secret.len(),
                MIN_SECRET_LENGTH
            )));
        }

        // Basic entropy check: ensure not all same character
        let first_char = secret.chars().next().unwrap();
        if secret.chars().all(|c| c == first_char) {
            return Err(Error::Config(
                "Secret has insufficient entropy (all same character)".to_string(),
            ));
        }

        // Additional entropy check: ensure at least some variety
        let unique_chars: std::collections::HashSet<char> = secret.chars().collect();
        if unique_chars.len() < 10 {
            warn!(
                unique_chars = unique_chars.len(),
                "Secret has low character diversity, consider improving entropy"
            );
        }

        Ok(())
    }

    /// Get all currently valid secrets
    ///
    /// Returns a vector of secret values that are considered valid.
    /// During rotation, this will return both current and previous versions.
    pub async fn get_valid_secrets(&self) -> Vec<String> {
        // Check if refresh is needed
        {
            let last_refresh = self.last_refresh.read().await;
            if last_refresh.elapsed() > REFRESH_INTERVAL {
                drop(last_refresh); // Release read lock before write

                // Attempt refresh (don't fail service if refresh fails)
                if let Err(e) = self.refresh_secrets().await {
                    error!(
                        error = %e,
                        "Failed to refresh secrets, using cached values"
                    );
                }
            }
        }

        let secrets = self.valid_secrets.read().await;
        secrets.iter().map(|s| s.value.clone()).collect()
    }

    /// Get the current (latest) secret
    ///
    /// Returns the most recent secret version.
    /// Use this for signing operations.
    pub async fn get_current_secret(&self) -> Result<String> {
        let secrets = self.get_valid_secrets().await;
        secrets
            .first()
            .cloned()
            .ok_or_else(|| Error::Internal("No secrets available".to_string()))
    }

    /// Verify a signature using any valid secret
    ///
    /// Tries all currently valid secrets (current + previous during rotation).
    /// Returns true if any secret validates the signature.
    ///
    /// # Arguments
    ///
    /// * `payload` - The data that was signed
    /// * `signature` - The HMAC signature to verify
    /// * `verify_fn` - Function to verify signature with a secret
    pub async fn verify_with_any_secret<F>(
        &self,
        payload: &[u8],
        signature: &str,
        verify_fn: F,
    ) -> Result<bool>
    where
        F: Fn(&[u8], &str, &str) -> Result<bool>,
    {
        let secrets = self.get_valid_secrets().await;

        if secrets.is_empty() {
            return Err(Error::Internal(
                "No valid secrets available for verification".to_string(),
            ));
        }

        // Try each valid secret
        for secret in &secrets {
            match verify_fn(payload, signature, secret) {
                Ok(true) => return Ok(true),
                Ok(false) => continue,
                Err(e) => {
                    warn!(error = %e, "Signature verification error");
                    continue;
                }
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_secret_length() {
        // Too short
        let short_secret = "short";
        assert!(SecretManager::validate_secret(short_secret).is_err());

        // Exactly minimum length
        let min_secret = "a".repeat(MIN_SECRET_LENGTH);
        assert!(SecretManager::validate_secret(&min_secret).is_err()); // Fails entropy check

        // Good length with variety
        let good_secret = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH";
        assert!(SecretManager::validate_secret(good_secret).is_ok());
    }

    #[test]
    fn test_validate_secret_entropy() {
        // All same character
        let low_entropy = "a".repeat(MIN_SECRET_LENGTH);
        assert!(SecretManager::validate_secret(&low_entropy).is_err());

        // Good entropy
        let good_entropy = "9f3k2j8h4g7d6s5a1q0w9e8r7t6y5u4i3o2p1m0n9b8v7c6x5z4";
        assert!(SecretManager::validate_secret(good_entropy).is_ok());
    }
}
