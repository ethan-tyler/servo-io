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

    /// Partition configuration for time-based or custom partitioning
    pub partition: Option<PartitionConfig>,
}

/// Partition configuration for an asset
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartitionConfig {
    /// Type of partitioning
    pub partition_type: PartitionType,

    /// Partition key or expression
    pub partition_key: String,
}

/// Types of partitioning supported
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartitionType {
    /// Time-based partitioning (daily, hourly, etc.)
    Time {
        /// Granularity of time partitions
        granularity: TimeGranularity,
    },

    /// Custom partitioning expression (e.g., SQL expression)
    Custom {
        /// Expression or pattern used for partitioning
        expression: String,
    },

    /// Range-based partitioning
    Range {
        /// Lower bound (inclusive) as string representation
        lower_bound: String,
        /// Upper bound (exclusive) as string representation
        upper_bound: String,
    },
}

/// Time granularity for time-based partitions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeGranularity {
    /// Hourly partitions
    Hourly,

    /// Daily partitions
    Daily,

    /// Weekly partitions
    Weekly,

    /// Monthly partitions
    Monthly,

    /// Yearly partitions
    Yearly,
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
                partition: None,
            },
            dependencies: Vec::new(),
        }
    }

    /// Create a builder for constructing an asset with fluent API
    pub fn builder(name: impl Into<String>) -> AssetBuilder {
        AssetBuilder::new(name)
    }

    /// Add a dependency on another asset
    pub fn add_dependency(&mut self, upstream: AssetId, dep_type: DependencyType) {
        // Prevent duplicates to keep topology and lineage clean.
        let exists = self
            .dependencies
            .iter()
            .any(|d| d.upstream_asset_id == upstream && d.dependency_type == dep_type);
        if !exists {
            self.dependencies.push(AssetDependency {
                upstream_asset_id: upstream,
                dependency_type: dep_type,
            });
        }
    }

    /// Update the asset's updated_at timestamp
    pub fn touch(&mut self) {
        self.metadata.updated_at = Utc::now();
    }
}

/// Builder for constructing assets with a fluent API
pub struct AssetBuilder {
    name: String,
    description: Option<String>,
    asset_type: String,
    owner: Option<String>,
    tags: Vec<String>,
    tenant_id: Option<String>,
    partition: Option<PartitionConfig>,
    dependencies: Vec<AssetDependency>,
}

impl AssetBuilder {
    /// Create a new asset builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            asset_type: "generic".to_string(),
            owner: None,
            tags: Vec::new(),
            tenant_id: None,
            partition: None,
            dependencies: Vec::new(),
        }
    }

    /// Set the asset description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the asset type
    pub fn asset_type(mut self, asset_type: impl Into<String>) -> Self {
        self.asset_type = asset_type.into();
        self
    }

    /// Set the asset owner
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// Add a tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add multiple tags
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Set time-based daily partitioning
    pub fn partitioned_daily(mut self, partition_key: impl Into<String>) -> Self {
        self.partition = Some(PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: partition_key.into(),
        });
        self
    }

    /// Set time-based hourly partitioning
    pub fn partitioned_hourly(mut self, partition_key: impl Into<String>) -> Self {
        self.partition = Some(PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Hourly,
            },
            partition_key: partition_key.into(),
        });
        self
    }

    /// Set custom partition configuration
    pub fn partition(mut self, partition: PartitionConfig) -> Self {
        self.partition = Some(partition);
        self
    }

    /// Set custom partitioning with an expression
    pub fn partitioned_custom(mut self, expression: impl Into<String>) -> Self {
        self.partition = Some(PartitionConfig {
            partition_type: PartitionType::Custom {
                expression: expression.into(),
            },
            partition_key: "custom".to_string(),
        });
        self
    }

    /// Set range-based partitioning
    pub fn partitioned_range(
        mut self,
        partition_key: impl Into<String>,
        lower_bound: impl Into<String>,
        upper_bound: impl Into<String>,
    ) -> Self {
        self.partition = Some(PartitionConfig {
            partition_type: PartitionType::Range {
                lower_bound: lower_bound.into(),
                upper_bound: upper_bound.into(),
            },
            partition_key: partition_key.into(),
        });
        self
    }

    /// Add a dependency on another asset
    pub fn depends_on(mut self, upstream: AssetId, dep_type: DependencyType) -> Self {
        self.dependencies.push(AssetDependency {
            upstream_asset_id: upstream,
            dependency_type: dep_type,
        });
        self
    }

    /// Build the asset
    pub fn build(self) -> Asset {
        let now = Utc::now();
        Asset {
            id: AssetId::new(),
            name: self.name,
            metadata: AssetMetadata {
                description: self.description,
                asset_type: self.asset_type,
                owner: self.owner,
                tags: self.tags,
                created_at: now,
                updated_at: now,
                tenant_id: self.tenant_id,
                partition: self.partition,
            },
            dependencies: self.dependencies,
        }
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
        assert!(asset.metadata.partition.is_none());
    }

    #[test]
    fn test_add_dependency() {
        let mut asset = Asset::new("downstream");
        let upstream_id = AssetId::new();

        asset.add_dependency(upstream_id, DependencyType::Data);
        asset.add_dependency(upstream_id, DependencyType::Data); // duplicate should be ignored

        assert_eq!(asset.dependencies.len(), 1);
        assert_eq!(asset.dependencies[0].upstream_asset_id, upstream_id);
        assert_eq!(asset.dependencies[0].dependency_type, DependencyType::Data);
    }

    #[test]
    fn test_asset_builder_basic() {
        let asset = Asset::builder("customer_data")
            .description("Customer records")
            .asset_type("table")
            .owner("data_team")
            .build();

        assert_eq!(asset.name, "customer_data");
        assert_eq!(
            asset.metadata.description,
            Some("Customer records".to_string())
        );
        assert_eq!(asset.metadata.asset_type, "table");
        assert_eq!(asset.metadata.owner, Some("data_team".to_string()));
    }

    #[test]
    fn test_asset_builder_with_tags() {
        let asset = Asset::builder("sales_data")
            .tag("sales")
            .tag("analytics")
            .tags(vec!["pii".to_string(), "sensitive".to_string()])
            .build();

        assert_eq!(asset.metadata.tags.len(), 4);
        assert!(asset.metadata.tags.contains(&"sales".to_string()));
        assert!(asset.metadata.tags.contains(&"pii".to_string()));
    }

    #[test]
    fn test_asset_builder_with_tenant() {
        let asset = Asset::builder("tenant_data")
            .tenant_id("tenant_123")
            .build();

        assert_eq!(asset.metadata.tenant_id, Some("tenant_123".to_string()));
    }

    #[test]
    fn test_asset_builder_with_daily_partition() {
        let asset = Asset::builder("events")
            .asset_type("table")
            .partitioned_daily("event_date")
            .build();

        assert!(asset.metadata.partition.is_some());
        let partition = asset.metadata.partition.unwrap();
        assert_eq!(partition.partition_key, "event_date");
        assert_eq!(
            partition.partition_type,
            PartitionType::Time {
                granularity: TimeGranularity::Daily
            }
        );
    }

    #[test]
    fn test_asset_builder_with_custom_partition() {
        let asset = Asset::builder("custom_asset")
            .partitioned_custom("hash(user_id)")
            .build();

        let partition = asset.metadata.partition.unwrap();
        match partition.partition_type {
            PartitionType::Custom { expression } => {
                assert_eq!(expression, "hash(user_id)");
            }
            _ => panic!("Expected custom partition"),
        }
    }

    #[test]
    fn test_asset_builder_with_range_partition() {
        let asset = Asset::builder("range_asset")
            .partitioned_range("event_date", "2024-01-01", "2025-01-01")
            .build();

        let partition = asset.metadata.partition.unwrap();
        match partition.partition_type {
            PartitionType::Range {
                lower_bound,
                upper_bound,
            } => {
                assert_eq!(lower_bound, "2024-01-01");
                assert_eq!(upper_bound, "2025-01-01");
            }
            _ => panic!("Expected range partition"),
        }
        assert_eq!(partition.partition_key, "event_date");
    }

    #[test]
    fn test_asset_builder_with_hourly_partition() {
        let asset = Asset::builder("logs")
            .partitioned_hourly("timestamp")
            .build();

        assert!(asset.metadata.partition.is_some());
        let partition = asset.metadata.partition.unwrap();
        assert_eq!(partition.partition_key, "timestamp");
        assert_eq!(
            partition.partition_type,
            PartitionType::Time {
                granularity: TimeGranularity::Hourly
            }
        );
    }

    #[test]
    fn test_asset_builder_with_dependencies() {
        let upstream_id = AssetId::new();
        let asset = Asset::builder("downstream")
            .depends_on(upstream_id, DependencyType::Data)
            .build();

        assert_eq!(asset.dependencies.len(), 1);
        assert_eq!(asset.dependencies[0].upstream_asset_id, upstream_id);
        assert_eq!(asset.dependencies[0].dependency_type, DependencyType::Data);
    }

    #[test]
    fn test_asset_builder_fluent_api() {
        let upstream_id = AssetId::new();
        let asset = Asset::builder("complex_asset")
            .description("A complex asset with many properties")
            .asset_type("view")
            .owner("analytics_team")
            .tag("core")
            .tag("daily")
            .tenant_id("tenant_456")
            .partitioned_daily("partition_date")
            .depends_on(upstream_id, DependencyType::Data)
            .build();

        assert_eq!(asset.name, "complex_asset");
        assert_eq!(asset.metadata.asset_type, "view");
        assert_eq!(asset.metadata.tags.len(), 2);
        assert_eq!(asset.dependencies.len(), 1);
        assert!(asset.metadata.partition.is_some());
        assert_eq!(asset.metadata.tenant_id, Some("tenant_456".to_string()));
    }

    #[test]
    fn test_asset_touch_updates_timestamp() {
        let mut asset = Asset::new("test");
        let original_updated_at = asset.metadata.updated_at;

        // Sleep a tiny bit to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(10));

        asset.touch();

        assert!(asset.metadata.updated_at > original_updated_at);
    }

    #[test]
    fn test_partition_config_serialization() {
        let partition = PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: "date".to_string(),
        };

        // Test JSON serialization
        let json = serde_json::to_string(&partition).unwrap();
        let deserialized: PartitionConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(partition, deserialized);
    }

    #[test]
    fn test_asset_serialization() {
        let asset = Asset::builder("test_asset")
            .description("Test")
            .partitioned_daily("date")
            .build();

        // Test JSON serialization
        let json = serde_json::to_string(&asset).unwrap();
        let deserialized: Asset = serde_json::from_str(&json).unwrap();

        assert_eq!(asset.name, deserialized.name);
        assert_eq!(
            asset.metadata.description,
            deserialized.metadata.description
        );
        assert_eq!(asset.metadata.partition, deserialized.metadata.partition);
    }
}
