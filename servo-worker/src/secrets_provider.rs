//! Unified secrets provider
//!
//! Provides secrets abstraction that:
//! - Uses GCP Secret Manager in production/staging (via Workload Identity)
//! - Falls back to environment variables in development
//! - Supports secret rotation (multiple valid secrets)

use crate::environment::RuntimeEnvironment;
use servo_cloud_gcp::auth::GcpAuth;
use servo_cloud_gcp::secrets::SecretManager;
use std::env;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Error type for secrets operations
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    #[error("Secret Manager initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Secret not found: {0}")]
    SecretNotFound(String),

    #[error("Secret validation failed: {0}")]
    ValidationFailed(String),

    #[error("GCP authentication failed: {0}")]
    AuthError(String),
}

/// Secrets provider that abstracts secret source
///
/// In production/staging, uses GCP Secret Manager with automatic refresh
/// and rotation support. In development, uses environment variables.
pub struct SecretsProvider {
    /// Secret Manager instance (production/staging only)
    secret_manager: Option<Arc<SecretManager>>,

    /// HMAC secret fallback for development
    hmac_fallback: Option<String>,

    /// Runtime environment
    environment: RuntimeEnvironment,
}

impl SecretsProvider {
    /// Create a new secrets provider for the given environment
    ///
    /// # Arguments
    ///
    /// * `environment` - The runtime environment (Production, Staging, Development)
    ///
    /// # Production/Staging Mode
    ///
    /// Requires:
    /// - `GCP_PROJECT_ID` - GCP project containing the secrets
    /// - `GCP_SECRET_NAME` - Name of the HMAC secret in Secret Manager
    ///
    /// Uses Workload Identity (metadata server) for authentication on Cloud Run.
    ///
    /// # Development Mode
    ///
    /// Requires:
    /// - `SERVO_HMAC_SECRET` - HMAC secret (min 32 bytes)
    ///
    /// # Errors
    ///
    /// Returns an error if secrets cannot be loaded or validated.
    pub async fn new(environment: RuntimeEnvironment) -> Result<Self, SecretsError> {
        if environment.should_use_secret_manager() {
            Self::new_with_secret_manager(environment).await
        } else {
            Self::new_with_env_vars(environment)
        }
    }

    /// Create provider using GCP Secret Manager
    async fn new_with_secret_manager(
        environment: RuntimeEnvironment,
    ) -> Result<Self, SecretsError> {
        let project_id = env::var("GCP_PROJECT_ID").map_err(|_| {
            SecretsError::InitializationFailed(
                "GCP_PROJECT_ID environment variable required for Secret Manager".to_string(),
            )
        })?;

        let secret_name = env::var("GCP_SECRET_NAME").map_err(|_| {
            SecretsError::InitializationFailed(
                "GCP_SECRET_NAME environment variable required for Secret Manager".to_string(),
            )
        })?;

        info!(
            environment = %environment,
            project_id = %project_id,
            secret_name = %secret_name,
            "Initializing secrets from GCP Secret Manager"
        );

        // Use auto-detection for GCP auth (Workload Identity on Cloud Run)
        let auth = GcpAuth::auto()
            .await
            .map_err(|e| SecretsError::AuthError(e.to_string()))?;

        let auth = Arc::new(auth);

        // Create Secret Manager client
        let secret_manager = SecretManager::new(project_id.clone(), secret_name.clone(), auth)
            .map_err(|e| SecretsError::InitializationFailed(e.to_string()))?;

        // Initialize and validate secrets
        secret_manager
            .initialize()
            .await
            .map_err(|e| SecretsError::InitializationFailed(e.to_string()))?;

        info!(
            environment = %environment,
            secret_source = "secret_manager",
            "Secrets initialized successfully"
        );

        Ok(Self {
            secret_manager: Some(Arc::new(secret_manager)),
            hmac_fallback: None,
            environment,
        })
    }

    /// Create provider using environment variables (development)
    fn new_with_env_vars(environment: RuntimeEnvironment) -> Result<Self, SecretsError> {
        warn!(
            environment = %environment,
            "Using environment variables for secrets (NOT recommended for production)"
        );

        let hmac_secret = env::var("SERVO_HMAC_SECRET").map_err(|_| {
            SecretsError::SecretNotFound(
                "SERVO_HMAC_SECRET environment variable not set".to_string(),
            )
        })?;

        // Validate minimum length
        if hmac_secret.len() < 32 {
            return Err(SecretsError::ValidationFailed(format!(
                "SERVO_HMAC_SECRET must be at least 32 bytes, got {}",
                hmac_secret.len()
            )));
        }

        info!(
            environment = %environment,
            secret_source = "environment_variable",
            secret_length = hmac_secret.len(),
            "Secrets initialized from environment variables"
        );

        Ok(Self {
            secret_manager: None,
            hmac_fallback: Some(hmac_secret),
            environment,
        })
    }

    /// Get the current HMAC secret for signing
    ///
    /// Returns the most recent secret version.
    pub async fn get_hmac_secret(&self) -> Result<String, SecretsError> {
        if let Some(ref sm) = self.secret_manager {
            sm.get_current_secret()
                .await
                .map_err(|e| SecretsError::SecretNotFound(e.to_string()))
        } else if let Some(ref secret) = self.hmac_fallback {
            Ok(secret.clone())
        } else {
            Err(SecretsError::SecretNotFound(
                "No HMAC secret available".to_string(),
            ))
        }
    }

    /// Get all valid HMAC secrets for verification
    ///
    /// During secret rotation, this returns both the current and previous
    /// secret versions, allowing verification with either.
    pub async fn get_all_valid_secrets(&self) -> Vec<String> {
        if let Some(ref sm) = self.secret_manager {
            sm.get_valid_secrets().await
        } else if let Some(ref secret) = self.hmac_fallback {
            vec![secret.clone()]
        } else {
            vec![]
        }
    }

    /// Get the runtime environment
    pub fn environment(&self) -> RuntimeEnvironment {
        self.environment
    }

    /// Get the secret source for logging
    pub fn secret_source(&self) -> &'static str {
        if self.secret_manager.is_some() {
            "secret_manager"
        } else {
            "environment_variable"
        }
    }

    /// Check if using Secret Manager (vs env vars)
    pub fn is_using_secret_manager(&self) -> bool {
        self.secret_manager.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secrets_error_display() {
        let err = SecretsError::SecretNotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = SecretsError::ValidationFailed("too short".to_string());
        assert!(err.to_string().contains("too short"));
    }

    // Note: Full integration tests require a running Secret Manager
    // and are covered in the integration test suite
}
