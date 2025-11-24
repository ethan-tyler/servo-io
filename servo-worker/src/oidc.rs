//! OIDC token validation module
//!
//! Implements JWT token validation for Google Cloud Tasks OIDC tokens with:
//! - JWKS fetching and caching
//! - Automatic cache refresh on key ID (kid) miss
//! - Signature verification
//! - Claims validation (iss, aud, exp, nbf, iat)
//! - Fail-closed security model

use anyhow::{anyhow, Context, Result};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::config::OidcConfig;

/// JWKS cache TTL (1 hour)
const JWKS_TTL: Duration = Duration::from_secs(3600);

/// JWT claims from OIDC token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Issuer
    pub iss: String,
    /// Audience
    pub aud: String,
    /// Expiration time (seconds since epoch)
    pub exp: i64,
    /// Issued at (seconds since epoch)
    pub iat: i64,
    /// Not before (seconds since epoch)
    #[serde(default)]
    pub nbf: Option<i64>,
    /// Service account email
    pub email: String,
}

/// Cached JWKS with expiration tracking
struct JwksCache {
    keys: JwkSet,
    fetched_at: Instant,
}

impl JwksCache {
    fn is_expired(&self) -> bool {
        self.fetched_at.elapsed() > JWKS_TTL
    }

    fn find_key(&self, kid: &str) -> Option<&jsonwebtoken::jwk::Jwk> {
        self.keys
            .keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(kid))
    }
}

/// OIDC token validator with JWKS caching
pub struct OidcValidator {
    config: OidcConfig,
    jwks_cache: Arc<RwLock<Option<JwksCache>>>,
    http_client: reqwest::Client,
}

impl OidcValidator {
    /// Create a new OIDC validator
    pub fn new(config: OidcConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            jwks_cache: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::new(),
        })
    }

    /// Validate an OIDC token
    ///
    /// Returns parsed claims if valid, error otherwise.
    /// Logs validation failures without exposing the token.
    pub async fn validate_token(&self, token: &str) -> Result<Claims> {
        if !self.config.enabled {
            return Err(anyhow!("OIDC validation is disabled"));
        }

        // 1. Decode header to get Key ID (kid)
        let header = decode_header(token).map_err(|e| anyhow!("Invalid token header: {}", e))?;

        let kid = header
            .kid
            .ok_or_else(|| anyhow!("Token missing 'kid' (Key ID)"))?;

        // 2. Get JWKS (from cache or fetch)
        let jwks = self.get_jwks(&kid).await?;

        // 3. Find the matching public key
        let key = jwks
            .keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(&kid))
            .ok_or_else(|| anyhow!("Key ID '{}' not found in JWKS", kid))?;

        // 4. Verify signature and decode claims
        let decoding_key = DecodingKey::from_jwk(key)
            .map_err(|e| anyhow!("Failed to create decoding key: {}", e))?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.config.expected_audience]);
        validation.set_issuer(&[&self.config.issuer]);

        let token_data = decode::<Claims>(token, &decoding_key, &validation).map_err(|e| {
            warn!("Token validation failed: {}", e);
            anyhow!("Token validation failed: {}", e)
        })?;

        // 5. Additional claims validation
        self.verify_claims(&token_data.claims)?;

        debug!("OIDC token validated for {}", token_data.claims.email);
        Ok(token_data.claims)
    }

    /// Get JWKS from cache or fetch if needed
    ///
    /// Refreshes cache if:
    /// - Cache is empty
    /// - Cache is expired (TTL)
    /// - Requested kid not found in cache
    async fn get_jwks(&self, kid: &str) -> Result<JwkSet> {
        // Try cache first
        {
            let cache = self.jwks_cache.read();
            if let Some(cached) = cache.as_ref() {
                if !cached.is_expired() && cached.find_key(kid).is_some() {
                    debug!("JWKS cache hit for kid '{}'", kid);
                    return Ok(cached.keys.clone());
                }
            }
        }

        // Cache miss or expired - fetch and update
        if let Some(cached) = self.jwks_cache.read().as_ref() {
            if cached.find_key(kid).is_none() {
                info!(
                    "Key ID '{}' not in cache - refreshing JWKS (possible key rotation)",
                    kid
                );
            }
        }

        self.fetch_and_cache_jwks().await
    }

    /// Fetch JWKS from Google and update cache
    pub async fn fetch_and_cache_jwks(&self) -> Result<JwkSet> {
        debug!("Fetching JWKS from {}", self.config.jwks_url);

        let jwks: JwkSet = self
            .http_client
            .get(&self.config.jwks_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| {
                error!("Failed to fetch JWKS: {}", e);
                anyhow!("Failed to fetch JWKS: {}", e)
            })?
            .json()
            .await
            .map_err(|e| {
                error!("Failed to parse JWKS JSON: {}", e);
                anyhow!("Failed to parse JWKS JSON: {}", e)
            })?;

        // Update cache
        {
            let mut cache = self.jwks_cache.write();
            *cache = Some(JwksCache {
                keys: jwks.clone(),
                fetched_at: Instant::now(),
            });
        }

        info!("JWKS cache updated ({} keys)", jwks.keys.len());
        Ok(jwks)
    }

    /// Verify additional claims beyond signature
    fn verify_claims(&self, claims: &Claims) -> Result<()> {
        let now = chrono::Utc::now().timestamp();

        // Check expiration
        if claims.exp < now {
            return Err(anyhow!("Token expired (exp: {}, now: {})", claims.exp, now));
        }

        // Check not-before (if present)
        if let Some(nbf) = claims.nbf {
            if nbf > now {
                return Err(anyhow!("Token not yet valid (nbf: {}, now: {})", nbf, now));
            }
        }

        // Check issued-at is reasonable (not too far in past/future)
        let iat_diff = (now - claims.iat).abs();
        if iat_diff > 3600 {
            warn!(
                "Token iat is {} seconds from now (possible clock skew)",
                iat_diff
            );
        }

        // Email must be present (service account identity)
        if claims.email.is_empty() {
            return Err(anyhow!("Token missing 'email' claim"));
        }

        Ok(())
    }

    /// Get the OIDC configuration
    pub fn config(&self) -> &OidcConfig {
        &self.config
    }
}

/// Initialize OIDC validator with fail-closed semantics
///
/// If OIDC is enabled, prefetches JWKS on startup to ensure keys are available.
/// Returns error if OIDC enabled but JWKS cannot be fetched (fail-closed).
pub async fn initialize_validator(config: OidcConfig) -> Result<OidcValidator> {
    let validator = OidcValidator::new(config)?;

    if validator.config.enabled {
        // Prefetch JWKS to fail fast if unavailable
        validator
            .fetch_and_cache_jwks()
            .await
            .context("Failed to fetch JWKS on startup - cannot start with OIDC enabled")?;

        let key_count = validator
            .jwks_cache
            .read()
            .as_ref()
            .unwrap()
            .keys
            .keys
            .len();
        info!("✅ OIDC validator initialized ({} keys)", key_count);
    } else {
        warn!("⚠️  OIDC validation DISABLED - do not use in production!");
    }

    Ok(validator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwks_cache_expiration() {
        let cache = JwksCache {
            keys: JwkSet { keys: vec![] },
            fetched_at: Instant::now() - Duration::from_secs(3700),
        };
        assert!(cache.is_expired());

        let fresh = JwksCache {
            keys: JwkSet { keys: vec![] },
            fetched_at: Instant::now(),
        };
        assert!(!fresh.is_expired());
    }

    #[test]
    fn test_claims_validation_expired() {
        let validator = OidcValidator {
            config: OidcConfig {
                enabled: true,
                issuer: "https://accounts.google.com".to_string(),
                jwks_url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                expected_audience: "https://worker.example.com".to_string(),
            },
            jwks_cache: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::new(),
        };

        let claims = Claims {
            iss: "https://accounts.google.com".to_string(),
            aud: "https://worker.example.com".to_string(),
            exp: chrono::Utc::now().timestamp() - 3600, // Expired 1 hour ago
            iat: chrono::Utc::now().timestamp() - 7200,
            nbf: None,
            email: "test@example.com".to_string(),
        };

        assert!(validator.verify_claims(&claims).is_err());
    }

    #[test]
    fn test_claims_validation_missing_email() {
        let validator = OidcValidator {
            config: OidcConfig {
                enabled: true,
                issuer: "https://accounts.google.com".to_string(),
                jwks_url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                expected_audience: "https://worker.example.com".to_string(),
            },
            jwks_cache: Arc::new(RwLock::new(None)),
            http_client: reqwest::Client::new(),
        };

        let claims = Claims {
            iss: "https://accounts.google.com".to_string(),
            aud: "https://worker.example.com".to_string(),
            exp: chrono::Utc::now().timestamp() + 3600,
            iat: chrono::Utc::now().timestamp(),
            nbf: None,
            email: String::new(), // Missing email
        };

        assert!(validator.verify_claims(&claims).is_err());
    }
}
