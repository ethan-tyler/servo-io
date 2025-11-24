//! # Servo Storage
//!
//! Metadata storage for Servo using PostgreSQL with row-level security
//! for multi-tenancy.

pub mod circuit_breaker;
pub mod metrics;
pub mod migrations;
pub mod models;
pub mod postgres;
pub mod tenant;

// Re-export commonly used types
pub use circuit_breaker::CircuitBreakerConfig;
pub use models::{AssetModel, ExecutionModel, WorkflowModel};
pub use postgres::PostgresStorage;
pub use tenant::TenantId;

/// Result type for storage operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for storage operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Tenant isolation violation: {0}")]
    TenantViolation(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Connection pool exhausted: {0}")]
    PoolExhausted(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Circuit breaker is open, rejecting database operation")]
    CircuitOpen,
}
