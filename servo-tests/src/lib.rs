//! Shared test utilities for Servo crates
//!
//! This crate provides:
//! - **Fixtures**: Pre-built test data factories for workflows, executions, assets
//! - **Builders**: Type-safe builders for constructing test objects
//! - **Mocks**: Mock implementations for external services (JWKS, secrets, etc.)
//! - **Assertions**: Custom assertions for common verification patterns
//!
//! # Example
//!
//! ```ignore
//! use servo_tests::{fixtures, builders, mocks};
//!
//! #[tokio::test]
//! async fn test_workflow_execution() {
//!     // Create a mock JWKS server
//!     let jwks = mocks::MockJwksServer::start().await;
//!
//!     // Build a test request
//!     let request = builders::ExecutionRequestBuilder::new()
//!         .with_workflow_id(fixtures::workflow::simple().id)
//!         .signed("test-secret")
//!         .with_oidc_token(jwks.valid_token("audience", "user@example.com"))
//!         .build();
//!
//!     // Execute and verify
//!     // ...
//! }
//! ```

pub mod assertions;
pub mod builders;
pub mod fixtures;
pub mod mocks;

// Re-export commonly used items
pub use builders::ExecutionRequestBuilder;
pub use fixtures::{execution, tenant, workflow};
pub use mocks::MockJwksServer;
