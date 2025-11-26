//! Runtime environment detection
//!
//! Detects whether the worker is running in Production, Staging, or Development
//! based on environment variables. This is used to determine whether to use
//! GCP Secret Manager (production/staging) or environment variables (development).

use std::env;
use std::fmt;

/// Runtime environment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEnvironment {
    /// Production environment (Cloud Run with production config)
    Production,
    /// Staging environment (Cloud Run with staging config)
    Staging,
    /// Development environment (local machine)
    Development,
}

impl RuntimeEnvironment {
    /// Detect the runtime environment from environment variables
    ///
    /// Detection logic:
    /// 1. If `K_SERVICE` is set → Running on Cloud Run
    ///    - Check `SERVO_ENVIRONMENT` for "staging" → Staging
    ///    - Otherwise → Production
    /// 2. If `K_SERVICE` is not set → Development
    ///
    /// `K_SERVICE` is automatically set by Cloud Run to the service name.
    pub fn detect() -> Self {
        // K_SERVICE is set by Cloud Run
        if env::var("K_SERVICE").is_ok() {
            // Check for explicit staging designation
            if let Ok(servo_env) = env::var("SERVO_ENVIRONMENT") {
                if servo_env.eq_ignore_ascii_case("staging") {
                    return Self::Staging;
                }
            }
            Self::Production
        } else {
            Self::Development
        }
    }

    /// Check if Secret Manager should be used for secrets
    ///
    /// Returns `true` for Production and Staging environments.
    /// Returns `false` for Development (use environment variables).
    pub fn should_use_secret_manager(&self) -> bool {
        matches!(self, Self::Production | Self::Staging)
    }

    /// Check if this is a production environment
    pub fn is_production(&self) -> bool {
        matches!(self, Self::Production)
    }

    /// Check if this is a development environment
    pub fn is_development(&self) -> bool {
        matches!(self, Self::Development)
    }

    /// Get the environment name as a string (for logging/metrics)
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
            Self::Development => "development",
        }
    }
}

impl fmt::Display for RuntimeEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn with_env_vars<F, R>(vars: &[(&str, Option<&str>)], f: F) -> R
    where
        F: FnOnce() -> R,
    {
        // Save original values
        let originals: Vec<_> = vars
            .iter()
            .map(|(k, _)| (*k, env::var(*k).ok()))
            .collect();

        // Set new values
        for (key, value) in vars {
            match value {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }

        let result = f();

        // Restore original values
        for (key, original) in originals {
            match original {
                Some(v) => env::set_var(key, v),
                None => env::remove_var(key),
            }
        }

        result
    }

    #[test]
    fn test_detect_development() {
        with_env_vars(&[("K_SERVICE", None), ("SERVO_ENVIRONMENT", None)], || {
            let env = RuntimeEnvironment::detect();
            assert_eq!(env, RuntimeEnvironment::Development);
            assert!(env.is_development());
            assert!(!env.should_use_secret_manager());
        });
    }

    #[test]
    fn test_detect_production() {
        with_env_vars(
            &[
                ("K_SERVICE", Some("servo-worker")),
                ("SERVO_ENVIRONMENT", Some("production")),
            ],
            || {
                let env = RuntimeEnvironment::detect();
                assert_eq!(env, RuntimeEnvironment::Production);
                assert!(env.is_production());
                assert!(env.should_use_secret_manager());
            },
        );
    }

    #[test]
    fn test_detect_staging() {
        with_env_vars(
            &[
                ("K_SERVICE", Some("servo-worker")),
                ("SERVO_ENVIRONMENT", Some("staging")),
            ],
            || {
                let env = RuntimeEnvironment::detect();
                assert_eq!(env, RuntimeEnvironment::Staging);
                assert!(env.should_use_secret_manager());
                assert!(!env.is_production());
            },
        );
    }

    #[test]
    fn test_cloud_run_defaults_to_production() {
        with_env_vars(
            &[("K_SERVICE", Some("servo-worker")), ("SERVO_ENVIRONMENT", None)],
            || {
                let env = RuntimeEnvironment::detect();
                assert_eq!(env, RuntimeEnvironment::Production);
            },
        );
    }

    #[test]
    fn test_display() {
        assert_eq!(RuntimeEnvironment::Production.to_string(), "production");
        assert_eq!(RuntimeEnvironment::Staging.to_string(), "staging");
        assert_eq!(RuntimeEnvironment::Development.to_string(), "development");
    }
}
