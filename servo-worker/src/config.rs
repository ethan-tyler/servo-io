//! Worker configuration module
//!
//! Handles loading and validation of worker configuration from environment variables,
//! with special focus on OIDC security settings.

use anyhow::{anyhow, Context, Result};
use std::env;
use tracing::warn;

/// OIDC configuration for token validation
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Whether OIDC validation is enabled
    pub enabled: bool,
    /// Expected token issuer (e.g., "https://accounts.google.com")
    pub issuer: String,
    /// JWKS endpoint URL for fetching public keys
    pub jwks_url: String,
    /// Expected audience (worker URL)
    pub expected_audience: String,
}

impl OidcConfig {
    /// Load OIDC configuration from environment variables
    ///
    /// Environment variables:
    /// - `OIDC_ENABLE_VALIDATION`: Enable OIDC (default: true)
    /// - `OIDC_ISSUER`: Token issuer (default: https://accounts.google.com)
    /// - `OIDC_EXPECTED_AUDIENCE`: Expected aud claim (required if enabled)
    ///
    /// JWKS URL is automatically derived from issuer.
    pub fn from_env() -> Result<Self> {
        let enabled: bool = env::var("OIDC_ENABLE_VALIDATION")
            .unwrap_or_else(|_| "true".to_string())
            .parse()
            .context("OIDC_ENABLE_VALIDATION must be 'true' or 'false'")?;

        if !enabled {
            warn!("⚠️  OIDC validation is DISABLED. Do not use in production!");
            return Ok(Self::disabled());
        }

        let issuer = env::var("OIDC_ISSUER")
            .unwrap_or_else(|_| "https://accounts.google.com".to_string());

        let jwks_url = Self::derive_jwks_url(&issuer)?;

        let expected_audience = env::var("OIDC_EXPECTED_AUDIENCE")
            .context("OIDC_EXPECTED_AUDIENCE required when validation enabled")?;

        let config = Self {
            enabled,
            issuer,
            jwks_url,
            expected_audience,
        };

        config.validate()?;
        Ok(config)
    }

    /// Derive JWKS URL from issuer
    ///
    /// For Google (accounts.google.com), uses the well-known JWKS endpoint.
    /// For other issuers, constructs /.well-known/jwks.json endpoint.
    fn derive_jwks_url(issuer: &str) -> Result<String> {
        if issuer == "https://accounts.google.com" {
            Ok("https://www.googleapis.com/oauth2/v3/certs".to_string())
        } else {
            Ok(format!(
                "{}/.well-known/jwks.json",
                issuer.trim_end_matches('/')
            ))
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.enabled {
            if self.expected_audience.is_empty() {
                return Err(anyhow!("expected_audience cannot be empty"));
            }
            if !self.jwks_url.starts_with("https://") {
                return Err(anyhow!("jwks_url must use HTTPS"));
            }
            if !self.issuer.starts_with("https://") {
                return Err(anyhow!("issuer must use HTTPS"));
            }
        }
        Ok(())
    }

    /// Create a disabled OIDC config (for local development)
    fn disabled() -> Self {
        Self {
            enabled: false,
            issuer: String::new(),
            jwks_url: String::new(),
            expected_audience: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_jwks_url_for_google() {
        let url = OidcConfig::derive_jwks_url("https://accounts.google.com").unwrap();
        assert_eq!(url, "https://www.googleapis.com/oauth2/v3/certs");
    }

    #[test]
    fn test_derive_jwks_url_for_custom_issuer() {
        let url = OidcConfig::derive_jwks_url("https://custom.issuer.com").unwrap();
        assert_eq!(url, "https://custom.issuer.com/.well-known/jwks.json");
    }

    #[test]
    fn test_validation_requires_https() {
        let config = OidcConfig {
            enabled: true,
            issuer: "http://insecure.com".to_string(),
            jwks_url: "http://insecure.com/jwks".to_string(),
            expected_audience: "https://worker.example.com".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validation_requires_audience() {
        let config = OidcConfig {
            enabled: true,
            issuer: "https://accounts.google.com".to_string(),
            jwks_url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
            expected_audience: String::new(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_disabled_config_passes_validation() {
        let config = OidcConfig::disabled();
        assert!(config.validate().is_ok());
    }
}
