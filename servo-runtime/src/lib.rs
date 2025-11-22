//! # Servo Runtime
//!
//! Execution runtime for Servo workflows, providing state management,
//! retry logic, and concurrency control.

pub mod concurrency;
pub mod converters;
pub mod executor;
pub mod orchestrator;
pub mod retry;
pub mod state_machine;

// Re-export commonly used types
pub use executor::{ExecutionResult, Executor};
pub use orchestrator::ExecutionOrchestrator;
pub use retry::{RetryPolicy, RetryStrategy};
pub use state_machine::{ExecutionState, StateMachine};

/// Result type for runtime operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for runtime operations
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Tenant violation: {0}")]
    TenantViolation(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Retry exhausted: {0}")]
    RetryExhausted(String),

    #[error("Concurrency limit exceeded: {0}")]
    ConcurrencyLimitExceeded(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
