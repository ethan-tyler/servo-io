//! Partition mapping for asset dependencies
//!
//! This module provides functionality to map partitions between upstream and downstream
//! assets in a data pipeline. It supports:
//!
//! - **Identity mapping**: Same partition key maps 1:1 (e.g., daily -> daily with same date)
//! - **Time aggregation**: Multiple upstream partitions map to one downstream
//!   (e.g., 7 daily -> 1 weekly)
//! - **Time window**: A sliding window of upstream partitions
//! - **Custom expression**: User-defined mapping logic
//!
//! # Example
//!
//! ```rust
//! use servo_core::partition_mapping::{PartitionMapping, PartitionMapper};
//! use servo_core::{PartitionType, TimeGranularity};
//!
//! // Identity mapping: daily partitions map 1:1
//! let mapping = PartitionMapping::Identity;
//! let upstream_keys = mapping.map_partition("2024-01-15");
//! assert_eq!(upstream_keys, vec!["2024-01-15"]);
//! ```

use crate::asset::{PartitionConfig, PartitionType, TimeGranularity};
use chrono::{Duration, NaiveDate, NaiveDateTime, Weekday};
use serde::{Deserialize, Serialize};

/// Error types for partition mapping operations
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PartitionMappingError {
    /// Incompatible partition types between assets
    #[error("Incompatible partition types: {downstream} depends on {upstream}")]
    IncompatibleTypes {
        upstream: String,
        downstream: String,
    },

    /// Invalid partition key format
    #[error("Invalid partition key format: {key} (expected {expected_format})")]
    InvalidKeyFormat {
        key: String,
        expected_format: String,
    },

    /// Missing partition configuration
    #[error("Asset has no partition configuration")]
    MissingPartitionConfig,

    /// Invalid time aggregation (e.g., daily can't aggregate to hourly)
    #[error("Invalid time aggregation: {from} cannot aggregate to {to}")]
    InvalidAggregation { from: String, to: String },

    /// Custom mapping expression error
    #[error("Custom mapping error: {0}")]
    CustomMappingError(String),
}

/// Partition mapping strategies
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PartitionMapping {
    /// Identity mapping - downstream partition key equals upstream partition key
    /// Use when both assets have the same partition granularity
    #[default]
    Identity,

    /// Time-based aggregation - downstream partition covers multiple upstream partitions
    /// E.g., weekly downstream aggregates 7 daily upstream partitions
    TimeAggregation {
        /// Number of upstream partitions per downstream partition
        factor: u32,
        /// Granularity of upstream partitions
        upstream_granularity: TimeGranularity,
        /// Granularity of downstream partitions
        downstream_granularity: TimeGranularity,
    },

    /// Time window - downstream depends on a window of upstream partitions
    /// E.g., rolling 7-day average
    TimeWindow {
        /// Number of upstream partitions to include
        window_size: u32,
        /// Granularity of the window
        granularity: TimeGranularity,
    },

    /// Offset mapping - downstream partition depends on upstream partition offset by N periods
    /// E.g., downstream 2024-01-15 depends on upstream 2024-01-14 (offset -1)
    TimeOffset {
        /// Number of periods to offset (negative = past, positive = future)
        offset: i32,
        /// Granularity of the offset
        granularity: TimeGranularity,
    },

    /// Custom expression-based mapping
    /// Allows arbitrary mapping logic via a string expression
    Custom {
        /// Expression that transforms partition keys
        expression: String,
    },
}

impl PartitionMapping {
    /// Map a downstream partition key to upstream partition key(s)
    ///
    /// # Arguments
    /// * `downstream_key` - The partition key of the downstream asset
    ///
    /// # Returns
    /// A vector of upstream partition keys that this downstream partition depends on
    pub fn map_partition(&self, downstream_key: &str) -> Vec<String> {
        match self {
            PartitionMapping::Identity => vec![downstream_key.to_string()],

            PartitionMapping::TimeAggregation {
                factor,
                upstream_granularity,
                downstream_granularity,
            } => self.map_time_aggregation(
                downstream_key,
                *factor,
                *upstream_granularity,
                *downstream_granularity,
            ),

            PartitionMapping::TimeWindow {
                window_size,
                granularity,
            } => self.map_time_window(downstream_key, *window_size, *granularity),

            PartitionMapping::TimeOffset {
                offset,
                granularity,
            } => self.map_time_offset(downstream_key, *offset, *granularity),

            PartitionMapping::Custom { expression: _ } => {
                // Custom expressions are evaluated externally
                // For now, return the key as-is
                vec![downstream_key.to_string()]
            }
        }
    }

    /// Map time aggregation (e.g., weekly -> 7 daily partitions)
    fn map_time_aggregation(
        &self,
        downstream_key: &str,
        _factor: u32, // Used for custom aggregation, not time-based
        upstream_granularity: TimeGranularity,
        downstream_granularity: TimeGranularity,
    ) -> Vec<String> {
        // Parse the downstream key based on granularity
        match (upstream_granularity, downstream_granularity) {
            (TimeGranularity::Daily, TimeGranularity::Weekly) => {
                // Weekly key format: "2024-W03" or "2024-01-15" (Monday of week)
                if let Some(monday) = parse_weekly_key(downstream_key) {
                    (0..7)
                        .map(|i| (monday + Duration::days(i)).format("%Y-%m-%d").to_string())
                        .collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            (TimeGranularity::Daily, TimeGranularity::Monthly) => {
                // Monthly key format: "2024-01"
                if let Some((year, month)) = parse_monthly_key(downstream_key) {
                    let start = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
                    let days_in_month = days_in_month(year, month);
                    (0..days_in_month)
                        .map(|i| {
                            (start + Duration::days(i as i64))
                                .format("%Y-%m-%d")
                                .to_string()
                        })
                        .collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            (TimeGranularity::Hourly, TimeGranularity::Daily) => {
                // Daily key format: "2024-01-15"
                if let Some(date) = parse_daily_key(downstream_key) {
                    (0..24)
                        .map(|h| format!("{} {:02}:00:00", date.format("%Y-%m-%d"), h))
                        .collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            (TimeGranularity::Monthly, TimeGranularity::Yearly) => {
                // Yearly key format: "2024"
                if let Ok(year) = downstream_key.parse::<i32>() {
                    (1..=12).map(|m| format!("{}-{:02}", year, m)).collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            _ => {
                // Fallback to identity for unsupported combinations
                vec![downstream_key.to_string()]
            }
        }
    }

    /// Map time window (e.g., rolling 7-day window)
    fn map_time_window(
        &self,
        downstream_key: &str,
        window_size: u32,
        granularity: TimeGranularity,
    ) -> Vec<String> {
        match granularity {
            TimeGranularity::Daily => {
                if let Some(date) = parse_daily_key(downstream_key) {
                    (0..window_size)
                        .map(|i| {
                            (date - Duration::days(i as i64))
                                .format("%Y-%m-%d")
                                .to_string()
                        })
                        .collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            TimeGranularity::Hourly => {
                if let Some(dt) = parse_hourly_key(downstream_key) {
                    (0..window_size)
                        .map(|i| {
                            (dt - Duration::hours(i as i64))
                                .format("%Y-%m-%d %H:00:00")
                                .to_string()
                        })
                        .collect()
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            _ => vec![downstream_key.to_string()],
        }
    }

    /// Map time offset (e.g., yesterday's data)
    fn map_time_offset(
        &self,
        downstream_key: &str,
        offset: i32,
        granularity: TimeGranularity,
    ) -> Vec<String> {
        match granularity {
            TimeGranularity::Daily => {
                if let Some(date) = parse_daily_key(downstream_key) {
                    let offset_date = date + Duration::days(offset as i64);
                    vec![offset_date.format("%Y-%m-%d").to_string()]
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            TimeGranularity::Hourly => {
                if let Some(dt) = parse_hourly_key(downstream_key) {
                    let offset_dt = dt + Duration::hours(offset as i64);
                    vec![offset_dt.format("%Y-%m-%d %H:00:00").to_string()]
                } else {
                    vec![downstream_key.to_string()]
                }
            }
            _ => vec![downstream_key.to_string()],
        }
    }

    /// Check if this mapping is compatible with the given partition types
    pub fn validate_compatibility(
        &self,
        upstream: &PartitionConfig,
        downstream: &PartitionConfig,
    ) -> Result<(), PartitionMappingError> {
        match self {
            PartitionMapping::Identity => {
                // Identity requires same partition type
                if upstream.partition_type != downstream.partition_type {
                    return Err(PartitionMappingError::IncompatibleTypes {
                        upstream: format!("{:?}", upstream.partition_type),
                        downstream: format!("{:?}", downstream.partition_type),
                    });
                }
                Ok(())
            }

            PartitionMapping::TimeAggregation {
                upstream_granularity,
                downstream_granularity,
                ..
            } => {
                // Validate upstream granularity is finer than downstream
                if !is_finer_granularity(*upstream_granularity, *downstream_granularity) {
                    return Err(PartitionMappingError::InvalidAggregation {
                        from: format!("{:?}", upstream_granularity),
                        to: format!("{:?}", downstream_granularity),
                    });
                }

                // Validate partition types match expected granularities
                match (&upstream.partition_type, &downstream.partition_type) {
                    (
                        PartitionType::Time { granularity: up },
                        PartitionType::Time { granularity: down },
                    ) => {
                        if up != upstream_granularity || down != downstream_granularity {
                            return Err(PartitionMappingError::IncompatibleTypes {
                                upstream: format!("{:?}", upstream.partition_type),
                                downstream: format!("{:?}", downstream.partition_type),
                            });
                        }
                        Ok(())
                    }
                    _ => Err(PartitionMappingError::IncompatibleTypes {
                        upstream: format!("{:?}", upstream.partition_type),
                        downstream: format!("{:?}", downstream.partition_type),
                    }),
                }
            }

            PartitionMapping::TimeWindow { granularity, .. }
            | PartitionMapping::TimeOffset { granularity, .. } => {
                // Both assets should have the same time granularity
                match (&upstream.partition_type, &downstream.partition_type) {
                    (
                        PartitionType::Time { granularity: up },
                        PartitionType::Time { granularity: down },
                    ) => {
                        if up != granularity || down != granularity {
                            return Err(PartitionMappingError::IncompatibleTypes {
                                upstream: format!("{:?}", upstream.partition_type),
                                downstream: format!("{:?}", downstream.partition_type),
                            });
                        }
                        Ok(())
                    }
                    _ => Err(PartitionMappingError::IncompatibleTypes {
                        upstream: format!("{:?}", upstream.partition_type),
                        downstream: format!("{:?}", downstream.partition_type),
                    }),
                }
            }

            PartitionMapping::Custom { .. } => {
                // Custom mappings skip validation - user is responsible
                Ok(())
            }
        }
    }
}

/// Partition mapper for resolving upstream partition dependencies
pub struct PartitionMapper {
    /// Mapping strategy to use
    mapping: PartitionMapping,
}

impl PartitionMapper {
    /// Create a new partition mapper with the given strategy
    pub fn new(mapping: PartitionMapping) -> Self {
        Self { mapping }
    }

    /// Create an identity mapper (most common case)
    pub fn identity() -> Self {
        Self::new(PartitionMapping::Identity)
    }

    /// Get the upstream partition key(s) for a given downstream partition
    pub fn get_upstream_partitions(&self, downstream_key: &str) -> Vec<String> {
        self.mapping.map_partition(downstream_key)
    }

    /// Validate that this mapper is compatible with the asset partition configurations
    pub fn validate(
        &self,
        upstream: &PartitionConfig,
        downstream: &PartitionConfig,
    ) -> Result<(), PartitionMappingError> {
        self.mapping.validate_compatibility(upstream, downstream)
    }
}

/// Infer the appropriate partition mapping between two partition types
pub fn infer_mapping(
    upstream: &PartitionType,
    downstream: &PartitionType,
) -> Result<PartitionMapping, PartitionMappingError> {
    match (upstream, downstream) {
        // Same granularity = identity mapping
        (PartitionType::Time { granularity: up }, PartitionType::Time { granularity: down })
            if up == down =>
        {
            Ok(PartitionMapping::Identity)
        }

        // Daily -> Weekly aggregation
        (
            PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            PartitionType::Time {
                granularity: TimeGranularity::Weekly,
            },
        ) => Ok(PartitionMapping::TimeAggregation {
            factor: 7,
            upstream_granularity: TimeGranularity::Daily,
            downstream_granularity: TimeGranularity::Weekly,
        }),

        // Daily -> Monthly aggregation
        (
            PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            PartitionType::Time {
                granularity: TimeGranularity::Monthly,
            },
        ) => Ok(PartitionMapping::TimeAggregation {
            factor: 30, // Approximate
            upstream_granularity: TimeGranularity::Daily,
            downstream_granularity: TimeGranularity::Monthly,
        }),

        // Hourly -> Daily aggregation
        (
            PartitionType::Time {
                granularity: TimeGranularity::Hourly,
            },
            PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
        ) => Ok(PartitionMapping::TimeAggregation {
            factor: 24,
            upstream_granularity: TimeGranularity::Hourly,
            downstream_granularity: TimeGranularity::Daily,
        }),

        // Monthly -> Yearly aggregation
        (
            PartitionType::Time {
                granularity: TimeGranularity::Monthly,
            },
            PartitionType::Time {
                granularity: TimeGranularity::Yearly,
            },
        ) => Ok(PartitionMapping::TimeAggregation {
            factor: 12,
            upstream_granularity: TimeGranularity::Monthly,
            downstream_granularity: TimeGranularity::Yearly,
        }),

        // Custom and Range types default to identity
        (PartitionType::Custom { .. }, PartitionType::Custom { .. })
        | (PartitionType::Range { .. }, PartitionType::Range { .. }) => {
            Ok(PartitionMapping::Identity)
        }

        // Incompatible types
        _ => Err(PartitionMappingError::IncompatibleTypes {
            upstream: format!("{:?}", upstream),
            downstream: format!("{:?}", downstream),
        }),
    }
}

// Helper functions for parsing partition keys

fn parse_daily_key(key: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()
}

fn parse_hourly_key(key: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:00:00").ok())
        .or_else(|| NaiveDateTime::parse_from_str(key, "%Y-%m-%dT%H:%M:%S").ok())
}

fn parse_weekly_key(key: &str) -> Option<NaiveDate> {
    // Try ISO week format "2024-W03"
    if key.contains("-W") {
        let parts: Vec<&str> = key.split("-W").collect();
        if parts.len() == 2 {
            if let (Ok(year), Ok(week)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
                return NaiveDate::from_isoywd_opt(year, week, Weekday::Mon);
            }
        }
    }
    // Try date format (assume it's the Monday of the week)
    parse_daily_key(key)
}

fn parse_monthly_key(key: &str) -> Option<(i32, u32)> {
    // Format: "2024-01"
    let parts: Vec<&str> = key.split('-').collect();
    if parts.len() >= 2 {
        if let (Ok(year), Ok(month)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
            if (1..=12).contains(&month) {
                return Some((year, month));
            }
        }
    }
    None
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn is_finer_granularity(finer: TimeGranularity, coarser: TimeGranularity) -> bool {
    let granularity_order = |g: TimeGranularity| -> u8 {
        match g {
            TimeGranularity::Hourly => 0,
            TimeGranularity::Daily => 1,
            TimeGranularity::Weekly => 2,
            TimeGranularity::Monthly => 3,
            TimeGranularity::Yearly => 4,
        }
    };
    granularity_order(finer) < granularity_order(coarser)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_mapping() {
        let mapping = PartitionMapping::Identity;
        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result, vec!["2024-01-15"]);
    }

    #[test]
    fn test_daily_to_weekly_aggregation() {
        let mapping = PartitionMapping::TimeAggregation {
            factor: 7,
            upstream_granularity: TimeGranularity::Daily,
            downstream_granularity: TimeGranularity::Weekly,
        };

        // Week starting 2024-01-15 (Monday)
        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result.len(), 7);
        assert_eq!(result[0], "2024-01-15");
        assert_eq!(result[6], "2024-01-21");
    }

    #[test]
    fn test_daily_to_monthly_aggregation() {
        let mapping = PartitionMapping::TimeAggregation {
            factor: 31,
            upstream_granularity: TimeGranularity::Daily,
            downstream_granularity: TimeGranularity::Monthly,
        };

        let result = mapping.map_partition("2024-01");
        assert_eq!(result.len(), 31); // January has 31 days
        assert_eq!(result[0], "2024-01-01");
        assert_eq!(result[30], "2024-01-31");
    }

    #[test]
    fn test_hourly_to_daily_aggregation() {
        let mapping = PartitionMapping::TimeAggregation {
            factor: 24,
            upstream_granularity: TimeGranularity::Hourly,
            downstream_granularity: TimeGranularity::Daily,
        };

        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result.len(), 24);
        assert_eq!(result[0], "2024-01-15 00:00:00");
        assert_eq!(result[23], "2024-01-15 23:00:00");
    }

    #[test]
    fn test_time_window() {
        let mapping = PartitionMapping::TimeWindow {
            window_size: 7,
            granularity: TimeGranularity::Daily,
        };

        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result.len(), 7);
        assert_eq!(result[0], "2024-01-15"); // Current day
        assert_eq!(result[6], "2024-01-09"); // 6 days ago
    }

    #[test]
    fn test_time_offset_negative() {
        let mapping = PartitionMapping::TimeOffset {
            offset: -1,
            granularity: TimeGranularity::Daily,
        };

        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result, vec!["2024-01-14"]); // Yesterday
    }

    #[test]
    fn test_time_offset_positive() {
        let mapping = PartitionMapping::TimeOffset {
            offset: 1,
            granularity: TimeGranularity::Daily,
        };

        let result = mapping.map_partition("2024-01-15");
        assert_eq!(result, vec!["2024-01-16"]); // Tomorrow
    }

    #[test]
    fn test_infer_identity_mapping() {
        let upstream = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };
        let downstream = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };

        let mapping = infer_mapping(&upstream, &downstream).unwrap();
        assert_eq!(mapping, PartitionMapping::Identity);
    }

    #[test]
    fn test_infer_daily_to_weekly() {
        let upstream = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };
        let downstream = PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        };

        let mapping = infer_mapping(&upstream, &downstream).unwrap();
        match mapping {
            PartitionMapping::TimeAggregation {
                factor,
                upstream_granularity,
                downstream_granularity,
            } => {
                assert_eq!(factor, 7);
                assert_eq!(upstream_granularity, TimeGranularity::Daily);
                assert_eq!(downstream_granularity, TimeGranularity::Weekly);
            }
            _ => panic!("Expected TimeAggregation"),
        }
    }

    #[test]
    fn test_infer_incompatible_types() {
        let upstream = PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        };
        let downstream = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };

        let result = infer_mapping(&upstream, &downstream);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_identity_compatible() {
        let mapping = PartitionMapping::Identity;
        let upstream = PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: "date".to_string(),
        };
        let downstream = PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: "date".to_string(),
        };

        assert!(mapping
            .validate_compatibility(&upstream, &downstream)
            .is_ok());
    }

    #[test]
    fn test_validate_identity_incompatible() {
        let mapping = PartitionMapping::Identity;
        let upstream = PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Daily,
            },
            partition_key: "date".to_string(),
        };
        let downstream = PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Hourly,
            },
            partition_key: "hour".to_string(),
        };

        assert!(mapping
            .validate_compatibility(&upstream, &downstream)
            .is_err());
    }

    #[test]
    fn test_partition_mapper() {
        let mapper = PartitionMapper::identity();
        let upstream = mapper.get_upstream_partitions("2024-01-15");
        assert_eq!(upstream, vec!["2024-01-15"]);
    }

    #[test]
    fn test_february_days() {
        // Regular year
        assert_eq!(days_in_month(2023, 2), 28);
        // Leap year
        assert_eq!(days_in_month(2024, 2), 29);
    }

    #[test]
    fn test_is_finer_granularity() {
        assert!(is_finer_granularity(
            TimeGranularity::Hourly,
            TimeGranularity::Daily
        ));
        assert!(is_finer_granularity(
            TimeGranularity::Daily,
            TimeGranularity::Weekly
        ));
        assert!(is_finer_granularity(
            TimeGranularity::Daily,
            TimeGranularity::Monthly
        ));
        assert!(!is_finer_granularity(
            TimeGranularity::Monthly,
            TimeGranularity::Daily
        ));
    }
}
