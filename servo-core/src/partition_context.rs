//! Partition Execution Context
//!
//! Runtime context for partitioned asset execution, providing partition information
//! to compute functions. This differs from `PartitionConfig` which defines the
//! partition structure - this module provides the actual partition key being processed.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::asset::{PartitionType, TimeGranularity};

/// Runtime context provided to compute functions for partitioned execution.
///
/// This context is created by the backfill executor and flows through the
/// execution pipeline: BackfillExecutor -> TaskPayload -> Worker -> Python.
///
/// # Example
///
/// ```
/// use servo_core::PartitionExecutionContext;
///
/// let ctx = PartitionExecutionContext::new("2024-01-15")
///     .with_partition_type("daily")
///     .with_timezone("UTC");
///
/// assert_eq!(ctx.partition_key, "2024-01-15");
/// assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PartitionExecutionContext {
    /// Current partition key being processed (e.g., "2024-01-15" for daily)
    pub partition_key: String,

    /// Partition type info for the asset ("daily", "hourly", "monthly", "static", "custom")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partition_type: Option<String>,

    /// Timezone (for time-based partitions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    /// Format string for parsing partition key (e.g., "%Y-%m-%d")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// For multi-dimensional partitions: dimension name -> value
    /// Example: {"date": "2024-01-15", "region": "us-west"}
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub dimensions: HashMap<String, String>,

    /// Upstream asset name -> partition keys that this partition depends on
    /// Used for partition-aware dependency resolution
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub upstream_partitions: HashMap<String, Vec<String>>,
}

impl PartitionExecutionContext {
    /// Create a new partition execution context with the given partition key
    pub fn new(partition_key: impl Into<String>) -> Self {
        Self {
            partition_key: partition_key.into(),
            ..Default::default()
        }
    }

    /// Set the partition type (builder pattern)
    pub fn with_partition_type(mut self, partition_type: impl Into<String>) -> Self {
        self.partition_type = Some(partition_type.into());
        self
    }

    /// Set the timezone (builder pattern)
    pub fn with_timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }

    /// Set the format string (builder pattern)
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Add a dimension value (builder pattern)
    pub fn with_dimension(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.dimensions.insert(name.into(), value.into());
        self
    }

    /// Add upstream partition dependencies (builder pattern)
    pub fn with_upstream_partition(
        mut self,
        asset_name: impl Into<String>,
        partition_keys: Vec<String>,
    ) -> Self {
        self.upstream_partitions
            .insert(asset_name.into(), partition_keys);
        self
    }

    /// Check if this is a multi-dimensional partition
    pub fn is_multi_dimensional(&self) -> bool {
        !self.dimensions.is_empty()
    }

    /// Get a dimension value by name
    pub fn get_dimension(&self, name: &str) -> Option<&str> {
        self.dimensions.get(name).map(|s| s.as_str())
    }

    /// Get upstream partition keys for an asset
    pub fn get_upstream_partitions(&self, asset_name: &str) -> Option<&Vec<String>> {
        self.upstream_partitions.get(asset_name)
    }

    /// Create context from PartitionType definition
    pub fn from_partition_type(partition_key: String, partition_type: &PartitionType) -> Self {
        let type_name = match partition_type {
            PartitionType::Time { granularity } => match granularity {
                TimeGranularity::Hourly => "hourly",
                TimeGranularity::Daily => "daily",
                TimeGranularity::Weekly => "weekly",
                TimeGranularity::Monthly => "monthly",
                TimeGranularity::Yearly => "yearly",
            },
            PartitionType::Custom { .. } => "custom",
            PartitionType::Range { .. } => "range",
        };

        Self::new(partition_key).with_partition_type(type_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_partition_context() {
        let ctx = PartitionExecutionContext::new("2024-01-15");
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert!(ctx.partition_type.is_none());
        assert!(ctx.timezone.is_none());
        assert!(ctx.dimensions.is_empty());
        assert!(ctx.upstream_partitions.is_empty());
    }

    #[test]
    fn test_builder_pattern() {
        let ctx = PartitionExecutionContext::new("2024-01-15")
            .with_partition_type("daily")
            .with_timezone("UTC")
            .with_format("%Y-%m-%d")
            .with_dimension("date", "2024-01-15")
            .with_dimension("region", "us-west")
            .with_upstream_partition(
                "source_table",
                vec!["2024-01-14".into(), "2024-01-15".into()],
            );

        assert_eq!(ctx.partition_key, "2024-01-15");
        assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
        assert_eq!(ctx.timezone.as_deref(), Some("UTC"));
        assert_eq!(ctx.format.as_deref(), Some("%Y-%m-%d"));
        assert_eq!(ctx.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(ctx.get_dimension("region"), Some("us-west"));
        assert_eq!(
            ctx.get_upstream_partitions("source_table"),
            Some(&vec!["2024-01-14".into(), "2024-01-15".into()])
        );
    }

    #[test]
    fn test_is_multi_dimensional() {
        let simple = PartitionExecutionContext::new("2024-01-15");
        assert!(!simple.is_multi_dimensional());

        let multi = PartitionExecutionContext::new("2024-01-15|us-west")
            .with_dimension("date", "2024-01-15")
            .with_dimension("region", "us-west");
        assert!(multi.is_multi_dimensional());
    }

    #[test]
    fn test_serialization() {
        let ctx = PartitionExecutionContext::new("2024-01-15")
            .with_partition_type("daily")
            .with_timezone("UTC");

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("2024-01-15"));
        assert!(json.contains("daily"));
        assert!(json.contains("UTC"));

        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx, deserialized);
    }

    #[test]
    fn test_skip_serializing_empty() {
        let ctx = PartitionExecutionContext::new("2024-01-15");
        let json = serde_json::to_string(&ctx).unwrap();

        // Empty optional fields should be skipped
        assert!(!json.contains("partition_type"));
        assert!(!json.contains("timezone"));
        assert!(!json.contains("dimensions"));
        assert!(!json.contains("upstream_partitions"));
    }

    #[test]
    fn test_deserialize_minimal() {
        let json = r#"{"partition_key": "2024-01-15"}"#;
        let ctx: PartitionExecutionContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert!(ctx.partition_type.is_none());
        assert!(ctx.dimensions.is_empty());
    }

    #[test]
    fn test_deserialize_full() {
        let json = r#"{
            "partition_key": "2024-01-15",
            "partition_type": "daily",
            "timezone": "America/New_York",
            "format": "%Y-%m-%d",
            "dimensions": {"date": "2024-01-15"},
            "upstream_partitions": {"source": ["2024-01-14", "2024-01-15"]}
        }"#;
        let ctx: PartitionExecutionContext = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
        assert_eq!(ctx.timezone.as_deref(), Some("America/New_York"));
        assert_eq!(ctx.format.as_deref(), Some("%Y-%m-%d"));
        assert_eq!(ctx.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(
            ctx.get_upstream_partitions("source"),
            Some(&vec!["2024-01-14".into(), "2024-01-15".into()])
        );
    }

    #[test]
    fn test_from_partition_type_daily() {
        let partition_type = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };
        let ctx =
            PartitionExecutionContext::from_partition_type("2024-01-15".into(), &partition_type);
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
    }

    #[test]
    fn test_from_partition_type_hourly() {
        let partition_type = PartitionType::Time {
            granularity: TimeGranularity::Hourly,
        };
        let ctx = PartitionExecutionContext::from_partition_type(
            "2024-01-15T10:00:00".into(),
            &partition_type,
        );
        assert_eq!(ctx.partition_key, "2024-01-15T10:00:00");
        assert_eq!(ctx.partition_type.as_deref(), Some("hourly"));
    }

    #[test]
    fn test_from_partition_type_custom() {
        let partition_type = PartitionType::Custom {
            expression: "hash(user_id)".into(),
        };
        let ctx =
            PartitionExecutionContext::from_partition_type("bucket_42".into(), &partition_type);
        assert_eq!(ctx.partition_key, "bucket_42");
        assert_eq!(ctx.partition_type.as_deref(), Some("custom"));
    }

    #[test]
    fn test_default() {
        let ctx = PartitionExecutionContext::default();
        assert_eq!(ctx.partition_key, "");
        assert!(ctx.partition_type.is_none());
        assert!(!ctx.is_multi_dimensional());
    }

    #[test]
    fn test_multi_dimensional_json_parsing() {
        // Test parsing a JSON multi-dimensional partition key
        let json_key = r#"{"date": "2024-01-15", "region": "us-west"}"#;

        // Simulate what build_partition_context does
        let mut ctx = PartitionExecutionContext::new(json_key);
        if json_key.starts_with('{') {
            if let Ok(parsed) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(json_key)
            {
                for (dim_name, dim_value) in parsed {
                    ctx = ctx.with_dimension(dim_name, dim_value);
                }
            }
        }

        assert_eq!(ctx.partition_key, json_key);
        assert!(ctx.is_multi_dimensional());
        assert_eq!(ctx.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(ctx.get_dimension("region"), Some("us-west"));
    }

    #[test]
    fn test_multi_dimensional_three_dimensions() {
        let json_key = r#"{"date": "2024-01-15", "region": "eu", "product": "widgets"}"#;

        let mut ctx = PartitionExecutionContext::new(json_key);
        if let Ok(parsed) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(json_key)
        {
            for (dim_name, dim_value) in parsed {
                ctx = ctx.with_dimension(dim_name, dim_value);
            }
        }

        assert!(ctx.is_multi_dimensional());
        assert_eq!(ctx.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(ctx.get_dimension("region"), Some("eu"));
        assert_eq!(ctx.get_dimension("product"), Some("widgets"));
    }

    #[test]
    fn test_simple_partition_key_not_parsed_as_json() {
        // A simple partition key like "2024-01-15" should NOT be parsed as JSON
        let simple_key = "2024-01-15";
        let ctx = PartitionExecutionContext::new(simple_key);

        // Without the JSON parsing logic, dimensions should be empty
        assert!(!ctx.is_multi_dimensional());
        assert_eq!(ctx.get_dimension("date"), None);
    }

    #[test]
    fn test_invalid_json_gracefully_handled() {
        // Invalid JSON should not cause errors, just skip dimension parsing
        let invalid_json = "{not valid json}";

        let mut ctx = PartitionExecutionContext::new(invalid_json);
        if invalid_json.starts_with('{') {
            if let Ok(parsed) =
                serde_json::from_str::<std::collections::HashMap<String, String>>(invalid_json)
            {
                for (dim_name, dim_value) in parsed {
                    ctx = ctx.with_dimension(dim_name, dim_value);
                }
            }
            // If parsing fails, dimensions remain empty
        }

        assert_eq!(ctx.partition_key, invalid_json);
        assert!(!ctx.is_multi_dimensional());
    }

    #[test]
    fn test_json_serialization_with_dimensions() {
        let ctx = PartitionExecutionContext::new(r#"{"date": "2024-01-15", "region": "us"}"#)
            .with_dimension("date", "2024-01-15")
            .with_dimension("region", "us");

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("dimensions"));
        assert!(json.contains("date"));
        assert!(json.contains("region"));

        // Deserialize and verify
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_multi_dimensional());
        assert_eq!(deserialized.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(deserialized.get_dimension("region"), Some("us"));
    }
}
