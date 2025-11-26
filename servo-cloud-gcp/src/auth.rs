//! GCP authentication and OIDC token generation
//!
//! Supports two authentication modes:
//! - **Workload Identity** (recommended for Cloud Run): Uses GCE metadata server
//! - **Service Account JSON**: Uses a JSON key file (local development)

use crate::metrics::TOKEN_ACQUISITION_DURATION;
use crate::{Error, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

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

/// GCE metadata server URL for Workload Identity
const METADATA_SERVER_URL: &str = "http://metadata.google.internal";

/// Header required for metadata server requests
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
const METADATA_FLAVOR_VALUE: &str = "Google";

/// Authentication mode
#[derive(Debug, Clone)]
enum AuthMode {
    /// Service account JSON key (local development)
    ServiceAccount(ServiceAccountCredentials),
    /// GCE metadata server (Cloud Run Workload Identity)
    MetadataServer {
        /// Service account email (fetched from metadata)
        service_account_email: String,
    },
}

/// GCP authentication for generating OIDC tokens and OAuth2 access tokens
///
/// This generates:
/// - Self-signed OIDC JWTs for authenticating to Cloud Run services (service account mode)
/// - OAuth2 access tokens for calling GCP APIs (Cloud Tasks, etc.)
///
/// # Authentication Modes
///
/// - **Workload Identity** (recommended): On Cloud Run, uses the metadata server
///   to automatically obtain tokens without any JSON keys
/// - **Service Account JSON**: For local development, uses a JSON key file
pub struct GcpAuth {
    auth_mode: AuthMode,
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
            auth_mode: AuthMode::ServiceAccount(credentials),
            http_client,
            access_token_cache: Arc::new(RwLock::new(None)),
            circuit_breaker,
        })
    }

    /// Create a new GCP auth using the GCE metadata server (Workload Identity)
    ///
    /// This is the recommended method for Cloud Run deployments.
    /// No JSON keys are required - authentication is handled automatically
    /// via the metadata server.
    ///
    /// # Errors
    ///
    /// Returns an error if the metadata server is not available
    pub async fn from_metadata_server() -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5)) // Short timeout for metadata server
            .build()
            .map_err(|e| Error::Auth(format!("Failed to create HTTP client: {}", e)))?;

        // Fetch service account email from metadata server
        let email_url = format!(
            "{}/computeMetadata/v1/instance/service-accounts/default/email",
            METADATA_SERVER_URL
        );

        let response = http_client
            .get(&email_url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await
            .map_err(|e| {
                Error::Auth(format!(
                    "Failed to connect to metadata server (not running on GCP?): {}",
                    e
                ))
            })?;

        if !response.status().is_success() {
            return Err(Error::Auth(format!(
                "Metadata server returned status {}",
                response.status()
            )));
        }

        let service_account_email = response.text().await.map_err(|e| {
            Error::Auth(format!(
                "Failed to read service account email from metadata: {}",
                e
            ))
        })?;

        info!(
            service_account = %service_account_email,
            "Initialized GCP auth via metadata server (Workload Identity)"
        );

        // Initialize circuit breaker for metadata server
        let circuit_breaker_config =
            servo_storage::circuit_breaker::CircuitBreakerConfig::from_env("metadata");
        let circuit_breaker = servo_storage::circuit_breaker::DatabaseCircuitBreaker::new(
            "metadata".to_string(),
            circuit_breaker_config,
        );

        Ok(Self {
            auth_mode: AuthMode::MetadataServer {
                service_account_email,
            },
            http_client,
            access_token_cache: Arc::new(RwLock::new(None)),
            circuit_breaker,
        })
    }

    /// Auto-detect authentication method
    ///
    /// Tries the following in order:
    /// 1. GCE metadata server (Workload Identity) - for Cloud Run
    /// 2. Service account JSON from `GCP_SERVICE_ACCOUNT_KEY_JSON` env var
    /// 3. Service account JSON from `GOOGLE_APPLICATION_CREDENTIALS` file
    ///
    /// # Returns
    ///
    /// Returns `Ok(GcpAuth)` using the first successful method,
    /// or an error if no authentication method is available.
    pub async fn auto() -> Result<Self> {
        // Try metadata server first (Cloud Run / GCE)
        debug!("Attempting GCP auth via metadata server");
        match Self::from_metadata_server().await {
            Ok(auth) => {
                info!("Using GCP Workload Identity (metadata server)");
                return Ok(auth);
            }
            Err(e) => {
                debug!(error = %e, "Metadata server not available, trying service account");
            }
        }

        // Try service account JSON from environment
        if let Ok(json) = std::env::var("GCP_SERVICE_ACCOUNT_KEY_JSON") {
            debug!("Attempting GCP auth via GCP_SERVICE_ACCOUNT_KEY_JSON env var");
            match Self::from_service_account_json(&json) {
                Ok(auth) => {
                    info!("Using GCP service account from GCP_SERVICE_ACCOUNT_KEY_JSON");
                    return Ok(auth);
                }
                Err(e) => {
                    warn!(error = %e, "Failed to parse GCP_SERVICE_ACCOUNT_KEY_JSON");
                }
            }
        }

        // Try GOOGLE_APPLICATION_CREDENTIALS file
        if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            debug!(path = %path, "Attempting GCP auth via GOOGLE_APPLICATION_CREDENTIALS file");
            match std::fs::read_to_string(&path) {
                Ok(json) => match Self::from_service_account_json(&json) {
                    Ok(auth) => {
                        info!(path = %path, "Using GCP service account from GOOGLE_APPLICATION_CREDENTIALS");
                        return Ok(auth);
                    }
                    Err(e) => {
                        warn!(error = %e, path = %path, "Failed to parse service account file");
                    }
                },
                Err(e) => {
                    warn!(error = %e, path = %path, "Failed to read GOOGLE_APPLICATION_CREDENTIALS file");
                }
            }
        }

        Err(Error::Auth(
            "No GCP authentication available. On Cloud Run, ensure the service has a service account. \
             For local development, set GCP_SERVICE_ACCOUNT_KEY_JSON or GOOGLE_APPLICATION_CREDENTIALS."
                .to_string(),
        ))
    }

    /// Check if running on GCP (metadata server available)
    pub async fn is_running_on_gcp() -> bool {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        let url = format!("{}/", METADATA_SERVER_URL);
        client
            .get(&url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await
            .is_ok()
    }

    /// Generate an OIDC token for authenticating to a Cloud Run service
    ///
    /// Creates a self-signed JWT with:
    /// - iss/sub: service account email
    /// - aud: target service URL
    /// - exp: 1 hour from now
    ///
    /// Note: This method only works in ServiceAccount mode.
    /// For MetadataServer mode, use `get_identity_token()` instead.
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
        let credentials = match &self.auth_mode {
            AuthMode::ServiceAccount(creds) => creds,
            AuthMode::MetadataServer { .. } => {
                return Err(Error::Auth(
                    "generate_oidc_token is not available in metadata server mode. \
                     Use get_identity_token() instead."
                        .to_string(),
                ));
            }
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("System time error: {}", e)))?
            .as_secs();

        let claims = OidcClaims {
            iss: credentials.client_email.clone(),
            sub: credentials.client_email.clone(),
            aud: audience.to_string(),
            iat: now,
            exp: now + 3600, // 1 hour expiration
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(credentials.private_key_id.clone());

        let encoding_key = EncodingKey::from_rsa_pem(credentials.private_key.as_bytes())
            .map_err(|e| Error::Auth(format!("Failed to parse private key: {}", e)))?;

        let token = encode(&header, &claims, &encoding_key)
            .map_err(|e| Error::Auth(format!("Failed to encode JWT: {}", e)))?;

        debug!(
            service_account = %credentials.client_email,
            audience = %audience,
            "Generated OIDC token"
        );

        Ok(token)
    }

    /// Get an identity token for authenticating to a Cloud Run service
    ///
    /// Works in both authentication modes:
    /// - ServiceAccount: Generates a self-signed JWT
    /// - MetadataServer: Fetches identity token from metadata server
    ///
    /// # Arguments
    ///
    /// * `audience` - The target service URL (Cloud Run service URL)
    pub async fn get_identity_token(&self, audience: &str) -> Result<String> {
        match &self.auth_mode {
            AuthMode::ServiceAccount(_) => self.generate_oidc_token(audience),
            AuthMode::MetadataServer { .. } => {
                self.fetch_identity_token_from_metadata(audience).await
            }
        }
    }

    /// Fetch identity token from metadata server
    async fn fetch_identity_token_from_metadata(&self, audience: &str) -> Result<String> {
        let url = format!(
            "{}/computeMetadata/v1/instance/service-accounts/default/identity?audience={}",
            METADATA_SERVER_URL,
            urlencoding::encode(audience)
        );

        let response = self
            .http_client
            .get(&url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await
            .map_err(|e| Error::Auth(format!("Failed to fetch identity token: {}", e)))?;

        if !response.status().is_success() {
            return Err(Error::Auth(format!(
                "Metadata server returned status {} for identity token",
                response.status()
            )));
        }

        let token = response.text().await.map_err(|e| {
            Error::Auth(format!("Failed to read identity token from metadata: {}", e))
        })?;

        debug!(audience = %audience, "Fetched identity token from metadata server");
        Ok(token)
    }

    /// Get the service account email
    pub fn service_account_email(&self) -> &str {
        match &self.auth_mode {
            AuthMode::ServiceAccount(creds) => &creds.client_email,
            AuthMode::MetadataServer {
                service_account_email,
            } => service_account_email,
        }
    }

    /// Get an OAuth2 access token for calling GCP APIs
    ///
    /// Uses cached token if available and not expired.
    /// Otherwise, fetches a new token (method depends on auth mode).
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
    /// Returns an error if token acquisition fails
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
                    debug!("Using cached OAuth2 access token");
                    return Ok(token.token.clone());
                }
            }
        }

        // Acquire new token (cache miss)
        let _timer = TOKEN_ACQUISITION_DURATION
            .with_label_values(&["false"])
            .start_timer();

        debug!(scope = %scope, "Fetching new OAuth2 access token");
        let token = match &self.auth_mode {
            AuthMode::ServiceAccount(_) => self.fetch_access_token_via_jwt(scope).await?,
            AuthMode::MetadataServer { .. } => self.fetch_access_token_from_metadata(scope).await?,
        };

        // Cache it
        {
            let mut cache = self.access_token_cache.write().await;
            *cache = Some(token.clone());
        }

        Ok(token.token)
    }

    /// Fetch access token from metadata server (Workload Identity)
    async fn fetch_access_token_from_metadata(&self, _scope: &str) -> Result<AccessToken> {
        // Note: Metadata server ignores scope parameter - returns token with all scopes
        // granted to the service account
        let url = format!(
            "{}/computeMetadata/v1/instance/service-accounts/default/token",
            METADATA_SERVER_URL
        );

        let response = self
            .http_client
            .get(&url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await
            .map_err(|e| Error::Auth(format!("Failed to fetch access token from metadata: {}", e)))?;

        if !response.status().is_success() {
            return Err(Error::Auth(format!(
                "Metadata server returned status {} for access token",
                response.status()
            )));
        }

        let token_response: TokenResponse = response.json().await.map_err(|e| {
            Error::Auth(format!(
                "Failed to parse token response from metadata: {}",
                e
            ))
        })?;

        // Calculate expiration (subtract 5 minutes for safety margin)
        let expires_in = token_response.expires_in.saturating_sub(300);
        let expires_at = Instant::now() + Duration::from_secs(expires_in);

        debug!(
            token_type = %token_response.token_type,
            expires_in_seconds = expires_in,
            "OAuth2 access token acquired from metadata server"
        );

        Ok(AccessToken {
            token: token_response.access_token,
            expires_at,
        })
    }

    /// Fetch a new OAuth2 access token via JWT assertion (service account mode)
    async fn fetch_access_token_via_jwt(&self, scope: &str) -> Result<AccessToken> {
        let credentials = match &self.auth_mode {
            AuthMode::ServiceAccount(creds) => creds,
            AuthMode::MetadataServer { .. } => {
                return Err(Error::Internal(
                    "fetch_access_token_via_jwt called in metadata server mode".to_string(),
                ));
            }
        };

        // Create JWT assertion
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Internal(format!("System time error: {}", e)))?
            .as_secs();

        let claims = OAuth2Claims {
            iss: credentials.client_email.clone(),
            scope: scope.to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            iat: now,
            exp: now + 3600,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(credentials.private_key_id.clone());

        let encoding_key = EncodingKey::from_rsa_pem(credentials.private_key.as_bytes())
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
        let params_copy = params;

        let response_result = self
            .circuit_breaker
            .call(|| async move {
                http_client
                    .post("https://oauth2.googleapis.com/token")
                    .form(&params_copy)
                    .send()
                    .await
            })
            .await;

        // Handle circuit breaker result
        let response = match response_result {
            Ok(resp) => resp,
            Err(servo_storage::circuit_breaker::CircuitBreakerError::CircuitOpen) => {
                warn!("Circuit breaker is open for OAuth2 token endpoint");
                return Err(Error::Auth(
                    "OAuth2 token endpoint circuit breaker is open".to_string(),
                ));
            }
            Err(servo_storage::circuit_breaker::CircuitBreakerError::Failure(e)) => {
                // Classify error to determine if we should trip breaker
                let status = crate::circuit_breaker::extract_status(&e);
                let is_network = crate::circuit_breaker::is_network_error(&e);
                let should_trip = crate::circuit_breaker::should_trip_breaker(status, is_network);

                warn!(
                    status = ?status,
                    is_network = is_network,
                    should_trip_breaker = should_trip,
                    error = %e,
                    "OAuth2 token endpoint HTTP request failed"
                );

                return Err(Error::Auth(format!(
                    "Failed to request access token: {}",
                    e
                )));
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

        debug!(
            token_type = %token_response.token_type,
            expires_in_seconds = expires_in,
            "OAuth2 access token acquired via JWT assertion"
        );

        Ok(AccessToken {
            token: token_response.access_token,
            expires_at,
        })
    }
}
