//! GCP configuration loading and validation.

use crate::{Error, Result};

/// Configuration for interacting with GCP Cloud Tasks and Cloud Run.
#[derive(Debug, Clone)]
pub struct GcpConfig {
    pub project_id: String,
    pub location: String,
    pub queue_name: String,
    pub worker_url: String,
    pub service_account_email: String,
    pub service_account_key_json: String,
    pub hmac_secret: String,
}

impl GcpConfig {
    pub fn from_env() -> Result<Self> {
        let project_id = std::env::var("GCP_PROJECT_ID")
            .map_err(|_| Error::Configuration("GCP_PROJECT_ID not set".into()))?;
        let location = std::env::var("GCP_LOCATION")
            .map_err(|_| Error::Configuration("GCP_LOCATION not set".into()))?;
        let queue_name = std::env::var("GCP_QUEUE_NAME")
            .map_err(|_| Error::Configuration("GCP_QUEUE_NAME not set".into()))?;
        let worker_url = std::env::var("GCP_WORKER_URL")
            .map_err(|_| Error::Configuration("GCP_WORKER_URL not set".into()))?;
        let service_account_email = std::env::var("GCP_SERVICE_ACCOUNT_EMAIL")
            .map_err(|_| Error::Configuration("GCP_SERVICE_ACCOUNT_EMAIL not set".into()))?;
        let service_account_key_json = std::env::var("GCP_SERVICE_ACCOUNT_KEY_JSON")
            .map_err(|_| Error::Configuration("GCP_SERVICE_ACCOUNT_KEY_JSON not set".into()))?;
        let hmac_secret = std::env::var("SERVO_HMAC_SECRET")
            .map_err(|_| Error::Configuration("SERVO_HMAC_SECRET not set".into()))?;

        let cfg = Self {
            project_id,
            location,
            queue_name,
            worker_url,
            service_account_email,
            service_account_key_json,
            hmac_secret,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.project_id.trim().is_empty() {
            return Err(Error::Configuration("project_id cannot be empty".into()));
        }
        if self.location.trim().is_empty() {
            return Err(Error::Configuration("location cannot be empty".into()));
        }
        if self.queue_name.trim().is_empty() {
            return Err(Error::Configuration("queue_name cannot be empty".into()));
        }

        // Validate worker URL format
        if !self.worker_url.starts_with("https://") && !self.worker_url.starts_with("http://") {
            return Err(Error::Configuration(
                "worker_url must be a valid HTTP(S) URL".into(),
            ));
        }

        // Validate service account email format
        if !self.service_account_email.contains('@') {
            return Err(Error::Configuration(
                "service_account_email must be a valid email".into(),
            ));
        }

        // Validate that service account key JSON can be parsed
        let _: serde_json::Value =
            serde_json::from_str(&self.service_account_key_json).map_err(|e| {
                Error::Configuration(format!("service_account_key_json is not valid JSON: {}", e))
            })?;

        if self.hmac_secret.is_empty() {
            return Err(Error::Configuration("hmac_secret cannot be empty".into()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_rejects_empty_project_id() {
        let config = GcpConfig {
            project_id: String::new(),
            location: "us-central1".to_string(),
            queue_name: "servo-tasks".to_string(),
            worker_url: "https://example.com".to_string(),
            service_account_email: "test@example.com".to_string(),
            service_account_key_json: "{}".to_string(),
            hmac_secret: "secret".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_url() {
        let config = GcpConfig {
            project_id: "project-123".to_string(),
            location: "us-central1".to_string(),
            queue_name: "servo-tasks".to_string(),
            worker_url: "not-a-url".to_string(),
            service_account_email: "test@example.com".to_string(),
            service_account_key_json: "{}".to_string(),
            hmac_secret: "secret".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_email() {
        let config = GcpConfig {
            project_id: "project-123".to_string(),
            location: "us-central1".to_string(),
            queue_name: "servo-tasks".to_string(),
            worker_url: "https://example.com".to_string(),
            service_account_email: "not-an-email".to_string(),
            service_account_key_json: "{}".to_string(),
            hmac_secret: "secret".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_invalid_json() {
        let config = GcpConfig {
            project_id: "project-123".to_string(),
            location: "us-central1".to_string(),
            queue_name: "servo-tasks".to_string(),
            worker_url: "https://example.com".to_string(),
            service_account_email: "test@example.com".to_string(),
            service_account_key_json: "not json".to_string(),
            hmac_secret: "secret".to_string(),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_accepts_valid_config() {
        let config = GcpConfig {
            project_id: "project-123".to_string(),
            location: "us-central1".to_string(),
            queue_name: "servo-tasks".to_string(),
            worker_url: "https://example.com".to_string(),
            service_account_email: "test@example.com".to_string(),
            service_account_key_json: "{\"type\":\"service_account\"}".to_string(),
            hmac_secret: "secret123".to_string(),
        };
        assert!(config.validate().is_ok());
    }
}
