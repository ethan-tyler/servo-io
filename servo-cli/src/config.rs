//! CLI configuration management

use serde::{Deserialize, Serialize};

/// CLI configuration
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database_url: String,
    pub cloud_provider: Option<String>,
    pub project_id: Option<String>,
    pub region: Option<String>,
}

impl Config {
    /// Load configuration from environment or config file
    #[allow(dead_code)]
    pub fn load() -> anyhow::Result<Self> {
        // TODO: Implement config loading from file
        // For now, just use environment variables

        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://localhost/servo".to_string());

        Ok(Self {
            database_url,
            cloud_provider: std::env::var("CLOUD_PROVIDER").ok(),
            project_id: std::env::var("PROJECT_ID").ok(),
            region: std::env::var("REGION").ok(),
        })
    }
}
