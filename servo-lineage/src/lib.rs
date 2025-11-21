//! # Servo Lineage
//!
//! Lineage graph operations for tracking data dependencies and impact analysis.

pub mod graph;
pub mod impact;
pub mod queries;

// Re-export commonly used types
pub use graph::{LineageEdge, LineageGraph, LineageNode};
pub use impact::ImpactAnalysis;
pub use queries::LineageQuery;

/// Result type for lineage operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for lineage operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    #[error("Circular dependency detected")]
    CircularDependency,

    #[error("Invalid lineage: {0}")]
    InvalidLineage(String),

    #[error("Query error: {0}")]
    Query(String),
}
