//! GCP authentication and OIDC token generation

use crate::metrics::TOKEN_ACQUISITION_DURATION;
use crate::{Error, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Service account credentials from GCP
#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountCredentials {
    #[serde(rename = "type")]
    account_type: String,
    #[allow(dead_code)]
    project_id: String,
    private_key_id: String,
    private_key: String,
    client_email: String,
    #[allow(dead_code)]
    client_id: String,
    #[allow(dead_code)]
    auth_uri: String,
    #[allow(dead_code)]
    token_uri: String,
}

/// JWT claims for OIDC token
#[derive(Debug, Serialize)]
struct OidcClaims {
    /// Issuer - the service account email
    iss: String,
    /// Subject - the service account email
    sub: String,
    /// Audience - the target service URL
    aud: String,
    /// Issued at time (Unix timestamp)
    iat: u64,
    /// Expiration time (Unix timestamp)
    exp: u64,
}

/// OAuth2 access token with expiration
#[derive(Debug, Clone)]
struct AccessToken {
    token: String,
    expires_at: Instant,
}

impl AccessToken {
    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }
}

/// Response from OAuth2 token endpoint
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    token_type: String,
}

/// JWT claims for OAuth2 service account assertion
#[derive(Debug, Serialize)]
struct OAuth2Claims {
    iss: String,
    scope: String,
    aud: String,
    exp: u64,
    iat: u64,
}

/// GCP authentication for generating OIDC tokens and OAuth2 access tokens
///
/// This generates:
/// - Self-signed OIDC JWTs for authenticating to Cloud Run services
/// - OAuth2 access tokens for calling GCP APIs (Cloud Tasks, etc.)
pub struct GcpAuth {
    credentials: ServiceAccountCredentials,
    http_client: reqwest::Client,
    access_token_cache: Arc<RwLock<Option<AccessToken>>>,
    circuit_breaker: servo_storage::circuit_breaker::DatabaseCircuitBreaker,
}

impl GcpAuth {
    /// Create a new GCP auth from service account JSON
    ///
    /// # Arguments
    ///
    /// * `service_account_json` - The JSON string from GCP service account key file
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON cannot be parsed or is invalid
    pub fn from_service_account_json(service_account_json: &str) -> Result<Self> {
        let credentials: ServiceAccountCredentials = serde_json::from_str(service_account_json)
            .map_err(|e| Error::Auth(format!("Failed to parse service account JSON: {}", e)))?;

        // Validate it's a service account
        if credentials.account_type != "service_account" {
            return Err(Error::Auth(format!(
                "Invalid account type: expected 'service_account', got '{}'",
                credentials.account_type
            )));
        }

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Auth(format!("Failed to create HTTP client: {}", e)))?;

        // Initialize circuit breaker for OAuth2 token endpoint
        let circuit_breaker_config =
            servo_storage::circuit_breaker::CircuitBreakerConfig::from_env("oauth2");
        let circuit_breaker = servo_storage::circuit_breaker::DatabaseCircuitBreaker::new(
            "oauth2".to_string(),
            circuit_breaker_config,
        );

        Ok(Self {
            credentials,
            http_client,
            access_token_cache: Arc::new(RwLock::new(None)),
            circuit_breaker,
        })
    }

    /// Generate an OIDC token for authenticating to a Cloud Run service
    ///
    /// Creates a self-signed JWT with:
    /// - iss/sub: service account email
    /// - aud: target service URL
    /// - exp: 1 hour from now
    ///
    /// # Arguments
    ///
    /// * `audience` - The target service URL (Cloud Run service URL)
    ///
    /// # Returns
    ///
    /// A JWT token string that can be used in the Authorization header
    ///
    /// # Errors
    ///
    /// Returns an error if token generation or signing fails
    pub fn generate_oidc_token(&self, audience: &str) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("System time error: {}", e)))?
            .as_secs();

        let claims = OidcClaims {
            iss: self.credentials.client_email.clone(),
            sub: self.credentials.client_email.clone(),
            aud: audience.to_string(),
            iat: now,
            exp: now + 3600, // 1 hour expiration
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.credentials.private_key_id.clone());

        let encoding_key = EncodingKey::from_rsa_pem(self.credentials.private_key.as_bytes())
            .map_err(|e| Error::Auth(format!("Failed to parse private key: {}", e)))?;

        let token = encode(&header, &claims, &encoding_key)
            .map_err(|e| Error::Auth(format!("Failed to encode JWT: {}", e)))?;

        tracing::debug!(
            service_account = %self.credentials.client_email,
            audience = %audience,
            "Generated OIDC token"
        );

        Ok(token)
    }

    /// Get the service account email
    pub fn service_account_email(&self) -> &str {
        &self.credentials.client_email
    }

    /// Get an OAuth2 access token for calling GCP APIs
    ///
    /// Uses cached token if available and not expired.
    /// Otherwise, exchanges service account JWT for access token.
    ///
    /// # Arguments
    ///
    /// * `scope` - OAuth2 scope (e.g., "https://www.googleapis.com/auth/cloud-platform")
    ///
    /// # Returns
    ///
    /// An access token string valid for ~1 hour
    ///
    /// # Errors
    ///
    /// Returns an error if token exchange fails
    pub async fn get_access_token(&self, scope: &str) -> Result<String> {
        // Check cache first
        {
            let cache = self.access_token_cache.read().await;
            if let Some(token) = cache.as_ref() {
                if !token.is_expired() {
                    // Record cache hit
                    let _timer = TOKEN_ACQUISITION_DURATION
                        .with_label_values(&["true"])
                        .start_timer();
                    tracing::debug!("Using cached OAuth2 access token");
                    return Ok(token.token.clone());
                }
            }
        }

        // Acquire new token (cache miss)
        let _timer = TOKEN_ACQUISITION_DURATION
            .with_label_values(&["false"])
            .start_timer();

        tracing::debug!(scope = %scope, "Fetching new OAuth2 access token");
        let token = self.fetch_access_token(scope).await?;

        // Cache it
        {
            let mut cache = self.access_token_cache.write().await;
            *cache = Some(token.clone());
        }

        Ok(token.token)
    }

    /// Fetch a new OAuth2 access token from Google's token endpoint
    async fn fetch_access_token(&self, scope: &str) -> Result<AccessToken> {
        // Create JWT assertion
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("System time error: {}", e)))?
            .as_secs();

        let claims = OAuth2Claims {
            iss: self.credentials.client_email.clone(),
            scope: scope.to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            iat: now,
            exp: now + 3600,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.credentials.private_key_id.clone());

        let encoding_key = EncodingKey::from_rsa_pem(self.credentials.private_key.as_bytes())
            .map_err(|e| Error::Auth(format!("Failed to parse private key: {}", e)))?;

        let assertion = encode(&header, &claims, &encoding_key)
            .map_err(|e| Error::Auth(format!("Failed to encode JWT assertion: {}", e)))?;

        // Exchange JWT for access token
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &assertion),
        ];

        // Wrap HTTP call with circuit breaker
        let http_client = &self.http_client;
        let params_clone = params.clone();

        let response_result = self
            .circuit_breaker
            .call(|| async move {
                http_client
                    .post("https://oauth2.googleapis.com/token")
                    .form(&params_clone)
                    .send()
                    .await
            })
            .await;

        // Handle circuit breaker result
        let response = match response_result {
            Ok(resp) => resp,
            Err(servo_storage::circuit_breaker::CircuitBreakerError::CircuitOpen) => {
                tracing::warn!("Circuit breaker is open for OAuth2 token endpoint");
                return Err(Error::Auth(
                    "OAuth2 token endpoint circuit breaker is open".to_string(),
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
                    "OAuth2 token endpoint HTTP request failed"
                );

                return Err(Error::Auth(format!("Failed to request access token: {}", e)));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Auth(format!(
                "Token endpoint returned {}: {}",
                status, body
            )));
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(|e| Error::Auth(format!("Failed to parse token response: {}", e)))?;

        // Calculate expiration (subtract 5 minutes for safety margin)
        let expires_in = token_response.expires_in.saturating_sub(300);
        let expires_at = Instant::now() + Duration::from_secs(expires_in);

        tracing::debug!(
            token_type = %token_response.token_type,
            expires_in_seconds = expires_in,
            "OAuth2 access token acquired"
        );

        Ok(AccessToken {
            token: token_response.access_token,
            expires_at,
        })
    }
}
