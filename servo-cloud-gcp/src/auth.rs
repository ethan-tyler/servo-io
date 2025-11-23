//! GCP authentication and OIDC token generation

use crate::{Error, Result};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Service account credentials from GCP
#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountCredentials {
    #[serde(rename = "type")]
    account_type: String,
    project_id: String,
    private_key_id: String,
    private_key: String,
    client_email: String,
    client_id: String,
    auth_uri: String,
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

/// GCP authentication for generating OIDC tokens
///
/// This generates self-signed JWT tokens using service account credentials
/// for authenticating Cloud Tasks requests to Cloud Run services.
pub struct GcpAuth {
    credentials: ServiceAccountCredentials,
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
        let credentials: ServiceAccountCredentials =
            serde_json::from_str(service_account_json).map_err(|e| {
                Error::Auth(format!("Failed to parse service account JSON: {}", e))
            })?;

        // Validate it's a service account
        if credentials.account_type != "service_account" {
            return Err(Error::Auth(format!(
                "Invalid account type: expected 'service_account', got '{}'",
                credentials.account_type
            )));
        }

        Ok(Self { credentials })
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
}
