//! # Servo Cloud GCP
//!
//! Google Cloud Platform adapter for Servo, providing integration with
//! Cloud Tasks, Cloud Run, and Cloud Logging.

pub mod auth;
pub mod executor;
pub mod monitoring;
pub mod queue;

// Re-export commonly used types
pub use executor::CloudRunExecutor;
pub use queue::CloudTasksQueue;

/// Result type for GCP operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for GCP operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("GCP API error: {0}")]
    Api(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}
