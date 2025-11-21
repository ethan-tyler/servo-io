//! Asset definitions and management
//!
//! Assets are persistent data artifacts (tables, files, ML models, reports)
//! that are produced by workflow executions. This module provides core types
//! for defining and tracking assets.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an asset
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub Uuid);

impl AssetId {
    /// Create a new random asset ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AssetId {
    fn default() -> Self {
        Self::new()
    }
}

/// An asset is a persistent data artifact produced by a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    /// Unique identifier
    pub id: AssetId,

    /// Human-readable name
    pub name: String,

    /// Asset metadata
    pub metadata: AssetMetadata,

    /// Dependencies on other assets
    pub dependencies: Vec<AssetDependency>,
}

/// Metadata about an asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetMetadata {
    /// Description of the asset
    pub description: Option<String>,

    /// Asset type (table, file, model, etc.)
    pub asset_type: String,

    /// Owner/creator of the asset
    pub owner: Option<String>,

    /// Tags for categorization
    pub tags: Vec<String>,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last update timestamp
    pub updated_at: DateTime<Utc>,

    /// Tenant ID for multi-tenancy
    pub tenant_id: Option<String>,
}

/// Dependency relationship between assets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDependency {
    /// ID of the upstream asset
    pub upstream_asset_id: AssetId,

    /// Type of dependency (data, metadata, etc.)
    pub dependency_type: DependencyType,
}

/// Type of dependency between assets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    /// Data dependency (output of one asset is input to another)
    Data,

    /// Metadata dependency (schema, statistics, etc.)
    Metadata,

    /// Control dependency (execution ordering only)
    Control,
}

impl Asset {
    /// Create a new asset with the given name
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: AssetId::new(),
            name: name.into(),
            metadata: AssetMetadata {
                description: None,
                asset_type: "generic".to_string(),
                owner: None,
                tags: Vec::new(),
                created_at: now,
                updated_at: now,
                tenant_id: None,
            },
            dependencies: Vec::new(),
        }
    }

    /// Add a dependency on another asset
    pub fn add_dependency(&mut self, upstream: AssetId, dep_type: DependencyType) {
        self.dependencies.push(AssetDependency {
            upstream_asset_id: upstream,
            dependency_type: dep_type,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_creation() {
        let asset = Asset::new("test_table");
        assert_eq!(asset.name, "test_table");
        assert!(asset.dependencies.is_empty());
    }

    #[test]
    fn test_add_dependency() {
        let mut asset = Asset::new("downstream");
        let upstream_id = AssetId::new();

        asset.add_dependency(upstream_id, DependencyType::Data);

        assert_eq!(asset.dependencies.len(), 1);
        assert_eq!(asset.dependencies[0].upstream_asset_id, upstream_id);
        assert_eq!(asset.dependencies[0].dependency_type, DependencyType::Data);
    }
}
