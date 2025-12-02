//! Partition resolution for backfill jobs
//!
//! This module integrates partition mapping with the backfill executor,
//! resolving partition ranges when creating upstream child jobs.
//!
//! # Current Limitations
//!
//! - Partition configuration is not yet persisted in the database
//! - Default behavior is identity mapping (upstream partitions = downstream partitions)
//! - Custom mapping expressions are not yet implemented
//!
//! # Future Enhancements
//!
//! When partition configuration is added to the storage layer:
//! 1. Load partition config for each asset from database
//! 2. Infer or use explicit mapping between upstream/downstream
//! 3. Apply mapping to transform partition ranges

use crate::{Error, Result};
#[cfg(test)]
use servo_core::TimeGranularity;
use servo_core::{
    infer_mapping, PartitionConfig, PartitionMapper, PartitionMapping, PartitionType,
    PartitionValidator,
};
use tracing::{debug, warn};

/// Partition resolver for backfill operations
///
/// Resolves partition ranges when creating upstream child jobs,
/// applying appropriate mapping between partition types.
pub struct PartitionResolver {
    /// Default mapping to use when no explicit mapping is configured
    default_mapping: PartitionMapping,
}

impl Default for PartitionResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl PartitionResolver {
    /// Create a new partition resolver with identity mapping as default
    pub fn new() -> Self {
        Self {
            default_mapping: PartitionMapping::Identity,
        }
    }

    /// Create a partition resolver with a specific default mapping
    pub fn with_default_mapping(mapping: PartitionMapping) -> Self {
        // Gate custom mappings - they are not yet implemented
        if let PartitionMapping::Custom { expression } = &mapping {
            warn!(
                expression = %expression,
                "Custom partition mapping expressions are not yet implemented. \
                 Using identity mapping instead."
            );
            return Self::new();
        }
        Self {
            default_mapping: mapping,
        }
    }

    /// Resolve upstream partition range from downstream partition range
    ///
    /// Given a downstream asset's partition range (start, end), returns the
    /// corresponding upstream partition range based on the mapping.
    ///
    /// # Arguments
    /// * `downstream_start` - Start of the downstream partition range
    /// * `downstream_end` - End of the downstream partition range (exclusive)
    /// * `upstream_config` - Optional partition configuration for upstream asset
    /// * `downstream_config` - Optional partition configuration for downstream asset
    ///
    /// # Returns
    /// Tuple of (upstream_start, upstream_end) partition keys
    pub fn resolve_upstream_range(
        &self,
        downstream_start: &str,
        downstream_end: &str,
        upstream_config: Option<&PartitionConfig>,
        downstream_config: Option<&PartitionConfig>,
    ) -> Result<(String, String)> {
        // If configs are available, use them to determine mapping
        let mapping = match (upstream_config, downstream_config) {
            (Some(up), Some(down)) => {
                // Try to infer mapping from partition types
                match infer_mapping(&up.partition_type, &down.partition_type) {
                    Ok(inferred) => {
                        debug!(
                            mapping = ?inferred,
                            "Inferred partition mapping from configs"
                        );
                        inferred
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            "Failed to infer partition mapping, using default"
                        );
                        self.default_mapping.clone()
                    }
                }
            }
            _ => {
                // No config available - use default (identity)
                debug!("No partition config available, using identity mapping");
                self.default_mapping.clone()
            }
        };

        // Apply the mapping to get upstream partition keys
        let mapper = PartitionMapper::new(mapping.clone());

        // For identity and time-based mappings, the start/end directly map
        // For aggregation mappings, we need the full range of upstream partitions
        match &mapping {
            PartitionMapping::Identity => {
                // 1:1 mapping - same partition keys
                Ok((downstream_start.to_string(), downstream_end.to_string()))
            }

            PartitionMapping::TimeAggregation { .. } => {
                // Map start to get all upstream partitions for start date
                let start_partitions = mapper.get_upstream_partitions(downstream_start);
                // Map end to get all upstream partitions for end date
                let end_partitions = mapper.get_upstream_partitions(downstream_end);

                // Upstream range is the first partition from start to last from end
                let upstream_start = start_partitions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| downstream_start.to_string());
                let upstream_end = end_partitions
                    .last()
                    .cloned()
                    .unwrap_or_else(|| downstream_end.to_string());

                Ok((upstream_start, upstream_end))
            }

            PartitionMapping::TimeWindow { .. } => {
                // For window mapping, the upstream range needs to include the lookback
                let start_partitions = mapper.get_upstream_partitions(downstream_start);
                // The earliest partition in the window is the actual start
                let upstream_start = start_partitions
                    .last() // Last in window = earliest
                    .cloned()
                    .unwrap_or_else(|| downstream_start.to_string());

                Ok((upstream_start, downstream_end.to_string()))
            }

            PartitionMapping::TimeOffset { .. } => {
                // Apply offset to both start and end
                let start_partitions = mapper.get_upstream_partitions(downstream_start);
                let end_partitions = mapper.get_upstream_partitions(downstream_end);

                let upstream_start = start_partitions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| downstream_start.to_string());
                let upstream_end = end_partitions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| downstream_end.to_string());

                Ok((upstream_start, upstream_end))
            }

            PartitionMapping::Custom { expression } => {
                // Custom mappings are gated at creation time
                // If we somehow get here, fall back to identity
                warn!(
                    expression = %expression,
                    "Custom mapping reached resolve - using identity fallback"
                );
                Ok((downstream_start.to_string(), downstream_end.to_string()))
            }
        }
    }

    /// Validate that upstream and downstream partition configs are compatible
    ///
    /// Returns an error if the partition types cannot be mapped.
    pub fn validate_compatibility(
        &self,
        upstream_config: &PartitionConfig,
        downstream_config: &PartitionConfig,
    ) -> Result<()> {
        PartitionValidator::validate_dependency(upstream_config, downstream_config)
            .map_err(|e| Error::InvalidState(format!("Incompatible partition types: {}", e)))
    }

    /// Validate a partition key format against a partition type
    pub fn validate_partition_key(&self, key: &str, partition_type: &PartitionType) -> Result<()> {
        PartitionValidator::validate_key_format(key, partition_type)
            .map_err(|e| Error::InvalidState(format!("Invalid partition key: {}", e)))
    }

    /// Get the expected format description for a partition type
    pub fn expected_format(partition_type: &PartitionType) -> &'static str {
        PartitionValidator::expected_format(partition_type)
    }
}

/// Extension trait for BackfillJobModel to support partition resolution
pub trait BackfillPartitionExt {
    /// Get partition start key
    fn partition_start(&self) -> &str;

    /// Get partition end key
    fn partition_end(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daily_config() -> PartitionConfig {
        PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: "date".to_string(),
        }
    }

    fn weekly_config() -> PartitionConfig {
        PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Weekly,
            },
            partition_key: "week".to_string(),
        }
    }

    #[test]
    fn test_identity_resolution() {
        let resolver = PartitionResolver::new();
        let (start, end) = resolver
            .resolve_upstream_range("2024-01-01", "2024-01-31", None, None)
            .unwrap();

        assert_eq!(start, "2024-01-01");
        assert_eq!(end, "2024-01-31");
    }

    #[test]
    fn test_identity_resolution_with_same_configs() {
        let resolver = PartitionResolver::new();
        let upstream = daily_config();
        let downstream = daily_config();

        let (start, end) = resolver
            .resolve_upstream_range(
                "2024-01-01",
                "2024-01-31",
                Some(&upstream),
                Some(&downstream),
            )
            .unwrap();

        assert_eq!(start, "2024-01-01");
        assert_eq!(end, "2024-01-31");
    }

    #[test]
    fn test_daily_to_weekly_resolution() {
        let resolver = PartitionResolver::new();
        let upstream = daily_config();
        let downstream = weekly_config();

        // When downstream is weekly, upstream daily should expand the range
        let (start, end) = resolver
            .resolve_upstream_range(
                "2024-01-01", // Monday
                "2024-01-08", // Following Monday
                Some(&upstream),
                Some(&downstream),
            )
            .unwrap();

        // Daily upstream should cover the full week
        assert_eq!(start, "2024-01-01");
        assert_eq!(end, "2024-01-14"); // End includes all days of second week
    }

    #[test]
    fn test_custom_mapping_gated() {
        // Custom mapping should be gated and fall back to identity
        let resolver = PartitionResolver::with_default_mapping(PartitionMapping::Custom {
            expression: "custom_expr".to_string(),
        });

        let (start, end) = resolver
            .resolve_upstream_range("2024-01-01", "2024-01-31", None, None)
            .unwrap();

        // Should use identity mapping (custom is gated)
        assert_eq!(start, "2024-01-01");
        assert_eq!(end, "2024-01-31");
    }

    #[test]
    fn test_validation_compatible() {
        let resolver = PartitionResolver::new();
        let upstream = daily_config();
        let downstream = daily_config();

        assert!(resolver
            .validate_compatibility(&upstream, &downstream)
            .is_ok());
    }

    #[test]
    fn test_validation_incompatible() {
        let resolver = PartitionResolver::new();
        let upstream = weekly_config();
        let downstream = daily_config();

        // Weekly can't feed daily (reverse aggregation)
        assert!(resolver
            .validate_compatibility(&upstream, &downstream)
            .is_err());
    }

    #[test]
    fn test_validate_partition_key() {
        let resolver = PartitionResolver::new();
        let daily = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };

        assert!(resolver
            .validate_partition_key("2024-01-15", &daily)
            .is_ok());
        assert!(resolver.validate_partition_key("invalid", &daily).is_err());
    }

    #[test]
    fn test_time_offset_resolution() {
        let resolver = PartitionResolver::with_default_mapping(PartitionMapping::TimeOffset {
            offset: -1,
            granularity: TimeGranularity::Daily,
        });

        let (start, end) = resolver
            .resolve_upstream_range("2024-01-15", "2024-01-20", None, None)
            .unwrap();

        // Should offset both by -1 day
        assert_eq!(start, "2024-01-14");
        assert_eq!(end, "2024-01-19");
    }

    #[test]
    fn test_time_window_resolution() {
        let resolver = PartitionResolver::with_default_mapping(PartitionMapping::TimeWindow {
            window_size: 7,
            granularity: TimeGranularity::Daily,
        });

        let (start, end) = resolver
            .resolve_upstream_range("2024-01-15", "2024-01-20", None, None)
            .unwrap();

        // Start should include lookback window (7 days before)
        assert_eq!(start, "2024-01-09");
        assert_eq!(end, "2024-01-20");
    }
}
