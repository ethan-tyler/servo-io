//! # Servo Cloud GCP
//!
//! Google Cloud Platform adapter for Servo, providing integration with
//! Cloud Tasks, Cloud Run, and Cloud Logging.

pub mod auth;
pub mod circuit_breaker;
pub mod config;
pub mod executor;
pub mod health;
pub mod metrics;
pub mod monitoring;
pub mod queue;
pub mod scheduler;
pub mod secrets;
pub mod signing;
pub mod trace_context;

// Re-export commonly used types
pub use config::GcpConfig;
pub use executor::CloudRunExecutor;
pub use queue::CloudTasksQueue;
pub use scheduler::{CloudSchedulerManager, ScheduleConfig};

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
