//! Workflow definitions and management
//!
//! Workflows are collections of assets with defined execution order.
//! This module provides core types for defining and managing workflows.

use crate::asset::AssetId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    /// Create a new random workflow ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

/// A workflow is a collection of assets with defined execution order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier
    pub id: WorkflowId,

    /// Human-readable name
    pub name: String,

    /// Workflow metadata
    pub metadata: WorkflowMetadata,

    /// Assets that are part of this workflow
    pub assets: Vec<AssetId>,
}

/// Metadata about a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowMetadata {
    /// Description of the workflow
    pub description: Option<String>,

    /// Owner/creator of the workflow
    pub owner: Option<String>,

    /// Tags for categorization
    pub tags: Vec<String>,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,

    /// Tenant ID for multi-tenancy
    pub tenant_id: Option<String>,

    /// Version of the workflow
    pub version: u32,
}

impl Workflow {
    /// Create a new workflow with the given name
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: WorkflowId::new(),
            name: name.into(),
            metadata: WorkflowMetadata {
                description: None,
                owner: None,
                tags: Vec::new(),
                created_at: now,
                updated_at: now,
                tenant_id: None,
                version: 1,
            },
            assets: Vec::new(),
        }
    }

    /// Add an asset to the workflow
    pub fn add_asset(&mut self, asset_id: AssetId) {
        self.assets.push(asset_id);
        self.touch();
    }

    /// Bump workflow version (for updated definitions)
    pub fn bump_version(&mut self) {
        self.metadata.version += 1;
        self.touch();
    }

    /// Set a specific version (e.g., during imports)
    pub fn with_version(mut self, version: u32) -> Self {
        self.metadata.version = version;
        self.touch();
        self
    }

    /// Update the updated_at timestamp
    pub fn touch(&mut self) {
        self.metadata.updated_at = Utc::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_creation() {
        let workflow = Workflow::new("daily_etl");
        assert_eq!(workflow.name, "daily_etl");
        assert!(workflow.assets.is_empty());
        assert_eq!(workflow.metadata.version, 1);
    }

    #[test]
    fn test_add_asset() {
        let mut workflow = Workflow::new("test_workflow");
        let asset_id = AssetId::new();

        workflow.add_asset(asset_id);

        assert_eq!(workflow.assets.len(), 1);
        assert_eq!(workflow.assets[0], asset_id);
    }

    #[test]
    fn test_bump_version_updates_metadata() {
        let mut workflow = Workflow::new("versioned");
        let original_updated_at = workflow.metadata.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(5));
        workflow.bump_version();

        assert_eq!(workflow.metadata.version, 2);
        assert!(workflow.metadata.updated_at > original_updated_at);
    }

    #[test]
    fn test_with_version_sets_version() {
        let workflow = Workflow::new("imported").with_version(42);
        assert_eq!(workflow.metadata.version, 42);
    }
}
