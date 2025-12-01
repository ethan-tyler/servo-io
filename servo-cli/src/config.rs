//! CLI configuration management
//!
//! Supports layered configuration with precedence:
//! 1. Defaults (lowest priority)
//! 2. Config file (servo.toml)
//! 3. Environment variables (highest priority)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// CLI configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Database connection URL
    pub database_url: String,
    /// Cloud provider (gcp, aws, local)
    pub cloud_provider: Option<String>,
    /// GCP project ID
    pub project_id: Option<String>,
    /// Cloud region
    pub region: Option<String>,
    /// Tenant ID for multi-tenant operations
    pub tenant_id: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "postgresql://localhost/servo".to_string(),
            cloud_provider: None,
            project_id: None,
            region: None,
            tenant_id: None,
        }
    }
}

impl Config {
    /// Load configuration with precedence: defaults → file → env
    pub fn load() -> anyhow::Result<Self> {
        let mut config = Config::default();

        // Load from config file if exists
        if let Some(file_config) = Self::load_from_file()? {
            config = config.merge(file_config);
        }

        // Apply environment variable overrides (highest priority)
        config = config.apply_env_overrides();

        config.validate()?;
        Ok(config)
    }

    /// Config file search order:
    /// 1. SERVO_CONFIG env var (explicit path)
    /// 2. ./servo.toml (current directory)
    /// 3. ~/.config/servo/config.toml (user config)
    fn load_from_file() -> anyhow::Result<Option<Self>> {
        for path in Self::config_search_paths() {
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let config: Config = toml::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;
                tracing::debug!(path = %path.display(), "Loaded config from file");
                return Ok(Some(config));
            }
        }
        Ok(None)
    }

    fn config_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Explicit path via SERVO_CONFIG env var
        if let Ok(explicit) = std::env::var("SERVO_CONFIG") {
            paths.push(PathBuf::from(explicit));
        }

        // 2. Current directory
        paths.push(PathBuf::from("servo.toml"));

        // 3. User config directory (~/.config/servo/config.toml)
        if let Some(config_dir) = dirs::config_dir() {
            paths.push(config_dir.join("servo").join("config.toml"));
        }

        paths
    }

    fn merge(self, other: Self) -> Self {
        Self {
            // Only override if the file value differs from default
            database_url: if other.database_url != Config::default().database_url {
                other.database_url
            } else {
                self.database_url
            },
            cloud_provider: other.cloud_provider.or(self.cloud_provider),
            project_id: other.project_id.or(self.project_id),
            region: other.region.or(self.region),
            tenant_id: other.tenant_id.or(self.tenant_id),
        }
    }

    fn apply_env_overrides(mut self) -> Self {
        if let Ok(url) = std::env::var("DATABASE_URL") {
            self.database_url = url;
        }
        if let Ok(provider) = std::env::var("CLOUD_PROVIDER") {
            self.cloud_provider = Some(provider);
        }
        if let Ok(project) = std::env::var("PROJECT_ID") {
            self.project_id = Some(project);
        }
        if let Ok(region) = std::env::var("REGION") {
            self.region = Some(region);
        }
        if let Ok(tenant) = std::env::var("TENANT_ID") {
            self.tenant_id = Some(tenant);
        }
        self
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.database_url.is_empty() {
            anyhow::bail!("database_url cannot be empty");
        }

        // Validate cloud_provider if set
        if let Some(ref provider) = self.cloud_provider {
            const VALID_PROVIDERS: &[&str] = &["gcp", "aws", "local"];
            if !VALID_PROVIDERS.contains(&provider.as_str()) {
                anyhow::bail!(
                    "Invalid cloud_provider '{}'. Must be one of: {}",
                    provider,
                    VALID_PROVIDERS.join(", ")
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.database_url, "postgresql://localhost/servo");
        assert!(config.cloud_provider.is_none());
        assert!(config.project_id.is_none());
        assert!(config.region.is_none());
        assert!(config.tenant_id.is_none());
    }

    #[test]
    fn test_load_from_toml() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"database_url = "postgresql://test@localhost/test""#).unwrap();

        // Set SERVO_CONFIG to point to our temp file
        std::env::set_var("SERVO_CONFIG", file.path().to_str().unwrap());
        // Clear DATABASE_URL to ensure file value is used
        std::env::remove_var("DATABASE_URL");

        let config = Config::load().unwrap();
        assert!(config.database_url.contains("test"));

        std::env::remove_var("SERVO_CONFIG");
    }

    #[test]
    fn test_env_overrides_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"database_url = "postgresql://file@localhost/file""#).unwrap();

        std::env::set_var("SERVO_CONFIG", file.path().to_str().unwrap());
        std::env::set_var("DATABASE_URL", "postgresql://env@localhost/env");

        let config = Config::load().unwrap();
        // Env var should override file
        assert!(config.database_url.contains("env"));

        std::env::remove_var("SERVO_CONFIG");
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn test_partial_config_file() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
cloud_provider = "gcp"
project_id = "my-project"
"#
        )
        .unwrap();

        std::env::set_var("SERVO_CONFIG", file.path().to_str().unwrap());
        std::env::remove_var("DATABASE_URL");

        let config = Config::load().unwrap();
        // Should use default database_url
        assert_eq!(config.database_url, "postgresql://localhost/servo");
        // But have file values for other fields
        assert_eq!(config.cloud_provider, Some("gcp".to_string()));
        assert_eq!(config.project_id, Some("my-project".to_string()));

        std::env::remove_var("SERVO_CONFIG");
    }

    #[test]
    fn test_validation_rejects_empty_url() {
        let config = Config {
            database_url: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_search_paths() {
        let paths = Config::config_search_paths();
        // Should always include ./servo.toml
        assert!(paths.contains(&PathBuf::from("servo.toml")));
    }

    #[test]
    fn test_merge_prefers_other_values() {
        let base = Config::default();
        let other = Config {
            database_url: "postgresql://other@localhost/other".to_string(),
            cloud_provider: Some("aws".to_string()),
            ..Default::default()
        };

        let merged = base.merge(other);
        assert!(merged.database_url.contains("other"));
        assert_eq!(merged.cloud_provider, Some("aws".to_string()));
    }

    #[test]
    fn test_validation_accepts_valid_cloud_providers() {
        for provider in ["gcp", "aws", "local"] {
            let config = Config {
                cloud_provider: Some(provider.to_string()),
                ..Default::default()
            };
            assert!(config.validate().is_ok(), "Should accept '{}'", provider);
        }
    }

    #[test]
    fn test_validation_rejects_invalid_cloud_provider() {
        let config = Config {
            cloud_provider: Some("azure".to_string()),
            ..Default::default()
        };
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("Invalid cloud_provider"));
        assert!(err.to_string().contains("azure"));
    }
}
