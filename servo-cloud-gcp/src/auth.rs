//! GCP authentication

use crate::Result;

/// GCP authentication configuration
pub struct GcpAuth {
    credentials_path: Option<String>,
}

impl GcpAuth {
    /// Create a new GCP auth configuration
    pub fn new() -> Self {
        Self {
            credentials_path: None,
        }
    }

    /// Set the credentials file path
    pub fn with_credentials_file(mut self, path: String) -> Self {
        self.credentials_path = Some(path);
        self
    }

    /// Get an access token
    pub async fn get_token(&self) -> Result<String> {
        // TODO: Implement token acquisition
        // 1. Load credentials from file or ADC
        // 2. Exchange for access token
        // 3. Cache token until expiry

        Err(crate::Error::Internal("Not yet implemented".to_string()))
    }
}

impl Default for GcpAuth {
    fn default() -> Self {
        Self::new()
    }
}
