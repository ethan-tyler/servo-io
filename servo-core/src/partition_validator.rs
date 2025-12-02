//! Partition type enforcement and validation
//!
//! This module provides validation logic to ensure partition types are compatible
//! between upstream and downstream assets. It enforces rules at:
//!
//! - **Dependency creation time**: Validates when adding asset dependencies
//! - **Backfill execution time**: Validates partition keys before processing
//! - **Runtime validation**: Ensures partition keys match expected formats
//!
//! # Validation Rules
//!
//! 1. **Same Type Required**: By default, identity mapping requires same partition types
//! 2. **Aggregation Rules**: Finer granularity can only aggregate to coarser
//!    (e.g., daily->weekly is valid, weekly->daily is not)
//! 3. **Key Format**: Partition keys must match their type's expected format
//!
//! # Example
//!
//! ```rust
//! use servo_core::partition_validator::{PartitionValidator, ValidationError};
//! use servo_core::{PartitionConfig, PartitionType, TimeGranularity};
//!
//! let upstream = PartitionConfig {
//!     partition_type: PartitionType::Time { granularity: TimeGranularity::Daily },
//!     partition_key: "date".to_string(),
//! };
//! let downstream = PartitionConfig {
//!     partition_type: PartitionType::Time { granularity: TimeGranularity::Daily },
//!     partition_key: "date".to_string(),
//! };
//!
//! // This will pass - same partition types
//! PartitionValidator::validate_dependency(&upstream, &downstream).unwrap();
//! ```

use crate::asset::{PartitionConfig, PartitionType, TimeGranularity};
use crate::partition_mapping::{infer_mapping, PartitionMapping, PartitionMappingError};
use chrono::{NaiveDate, NaiveDateTime};

/// Validation error types
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    /// Partition types are not compatible
    #[error(
        "Incompatible partition types: upstream {upstream:?} cannot feed downstream {downstream:?}"
    )]
    IncompatibleTypes {
        upstream: PartitionType,
        downstream: PartitionType,
    },

    /// Partition key format is invalid for the partition type
    #[error("Invalid partition key format: '{key}' is not valid for {partition_type:?}")]
    InvalidKeyFormat {
        key: String,
        partition_type: PartitionType,
    },

    /// Missing partition configuration when required
    #[error("Missing partition configuration for {asset_name}")]
    MissingPartitionConfig { asset_name: String },

    /// Cannot infer mapping between partition types
    #[error("Cannot infer partition mapping: {0}")]
    MappingInferenceError(#[from] PartitionMappingError),

    /// Partition key does not match expected value from mapping
    #[error("Partition key mismatch: expected {expected}, got {actual}")]
    KeyMismatch { expected: String, actual: String },
}

/// Result type for validation operations
pub type ValidationResult<T> = std::result::Result<T, ValidationError>;

/// Partition validator for enforcing type compatibility
pub struct PartitionValidator;

impl PartitionValidator {
    /// Validate that a dependency relationship has compatible partition types
    ///
    /// # Arguments
    /// * `upstream` - Partition configuration of the upstream (source) asset
    /// * `downstream` - Partition configuration of the downstream (dependent) asset
    ///
    /// # Returns
    /// `Ok(())` if compatible, `Err(ValidationError)` otherwise
    pub fn validate_dependency(
        upstream: &PartitionConfig,
        downstream: &PartitionConfig,
    ) -> ValidationResult<()> {
        // Try to infer a valid mapping between the partition types
        match infer_mapping(&upstream.partition_type, &downstream.partition_type) {
            Ok(_) => Ok(()),
            Err(e) => Err(ValidationError::MappingInferenceError(e)),
        }
    }

    /// Validate that a dependency relationship is valid with a specific mapping
    ///
    /// # Arguments
    /// * `upstream` - Partition configuration of the upstream asset
    /// * `downstream` - Partition configuration of the downstream asset
    /// * `mapping` - The partition mapping to validate against
    pub fn validate_dependency_with_mapping(
        upstream: &PartitionConfig,
        downstream: &PartitionConfig,
        mapping: &PartitionMapping,
    ) -> ValidationResult<()> {
        mapping
            .validate_compatibility(upstream, downstream)
            .map_err(ValidationError::MappingInferenceError)
    }

    /// Validate a partition key format for a given partition type
    ///
    /// # Arguments
    /// * `key` - The partition key to validate
    /// * `partition_type` - The expected partition type
    ///
    /// # Returns
    /// `Ok(())` if the key format is valid, `Err(ValidationError)` otherwise
    pub fn validate_key_format(key: &str, partition_type: &PartitionType) -> ValidationResult<()> {
        match partition_type {
            PartitionType::Time { granularity } => {
                Self::validate_time_key(key, *granularity)?;
            }
            PartitionType::Range {
                lower_bound,
                upper_bound,
            } => {
                Self::validate_range_key(key, lower_bound, upper_bound)?;
            }
            PartitionType::Custom { expression: _ } => {
                // Custom partitions skip format validation
            }
        }
        Ok(())
    }

    /// Validate a time-based partition key
    fn validate_time_key(key: &str, granularity: TimeGranularity) -> ValidationResult<()> {
        let valid = match granularity {
            TimeGranularity::Hourly => {
                // Expected formats: "2024-01-15 13:00:00" or "2024-01-15T13:00:00"
                NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M:%S").is_ok()
                    || NaiveDateTime::parse_from_str(key, "%Y-%m-%dT%H:%M:%S").is_ok()
                    || NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:00:00").is_ok()
            }
            TimeGranularity::Daily => {
                // Expected format: "2024-01-15"
                NaiveDate::parse_from_str(key, "%Y-%m-%d").is_ok()
            }
            TimeGranularity::Weekly => {
                // Expected formats: "2024-W03" or "2024-01-15" (Monday of week)
                key.contains("-W") && key.len() >= 7
                    || NaiveDate::parse_from_str(key, "%Y-%m-%d").is_ok()
            }
            TimeGranularity::Monthly => {
                // Expected format: "2024-01"
                let parts: Vec<&str> = key.split('-').collect();
                parts.len() == 2
                    && parts[0].parse::<i32>().is_ok()
                    && parts[1]
                        .parse::<u32>()
                        .map(|m| (1..=12).contains(&m))
                        .unwrap_or(false)
            }
            TimeGranularity::Yearly => {
                // Expected format: "2024"
                key.parse::<i32>().is_ok() && key.len() == 4
            }
        };

        if valid {
            Ok(())
        } else {
            Err(ValidationError::InvalidKeyFormat {
                key: key.to_string(),
                partition_type: PartitionType::Time { granularity },
            })
        }
    }

    /// Validate a range-based partition key
    fn validate_range_key(key: &str, lower_bound: &str, upper_bound: &str) -> ValidationResult<()> {
        // For range partitions, we just check that the key is within bounds
        // This is a simple string comparison - more sophisticated logic
        // would need to understand the key type
        if key >= lower_bound && key < upper_bound {
            Ok(())
        } else {
            Err(ValidationError::InvalidKeyFormat {
                key: key.to_string(),
                partition_type: PartitionType::Range {
                    lower_bound: lower_bound.to_string(),
                    upper_bound: upper_bound.to_string(),
                },
            })
        }
    }

    /// Validate that a backfill request has valid partition range
    ///
    /// # Arguments
    /// * `start_key` - Start of the partition range
    /// * `end_key` - End of the partition range (exclusive)
    /// * `partition_config` - The asset's partition configuration
    pub fn validate_backfill_range(
        start_key: &str,
        end_key: &str,
        partition_config: &PartitionConfig,
    ) -> ValidationResult<()> {
        // Validate both keys have correct format
        Self::validate_key_format(start_key, &partition_config.partition_type)?;
        Self::validate_key_format(end_key, &partition_config.partition_type)?;

        // Validate start < end
        if start_key >= end_key {
            return Err(ValidationError::InvalidKeyFormat {
                key: format!("{} to {}", start_key, end_key),
                partition_type: partition_config.partition_type.clone(),
            });
        }

        Ok(())
    }

    /// Get the expected partition key format for a partition type
    pub fn expected_format(partition_type: &PartitionType) -> &'static str {
        match partition_type {
            PartitionType::Time { granularity } => match granularity {
                TimeGranularity::Hourly => "YYYY-MM-DD HH:00:00 or YYYY-MM-DDTHH:00:00",
                TimeGranularity::Daily => "YYYY-MM-DD",
                TimeGranularity::Weekly => "YYYY-Www or YYYY-MM-DD (Monday)",
                TimeGranularity::Monthly => "YYYY-MM",
                TimeGranularity::Yearly => "YYYY",
            },
            PartitionType::Range { .. } => "value within [lower_bound, upper_bound)",
            PartitionType::Custom { .. } => "custom format",
        }
    }

    /// Check if two partition types can be automatically mapped
    pub fn can_auto_map(upstream: &PartitionType, downstream: &PartitionType) -> bool {
        infer_mapping(upstream, downstream).is_ok()
    }
}

/// Extension trait for PartitionConfig to add validation methods
pub trait PartitionConfigExt {
    /// Validate this partition configuration
    fn validate(&self) -> ValidationResult<()>;

    /// Validate a key against this configuration
    fn validate_key(&self, key: &str) -> ValidationResult<()>;
}

impl PartitionConfigExt for PartitionConfig {
    fn validate(&self) -> ValidationResult<()> {
        // Basic validation - partition key should not be empty
        if self.partition_key.is_empty() {
            return Err(ValidationError::MissingPartitionConfig {
                asset_name: "unknown".to_string(),
            });
        }
        Ok(())
    }

    fn validate_key(&self, key: &str) -> ValidationResult<()> {
        PartitionValidator::validate_key_format(key, &self.partition_type)
    }
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

    fn hourly_config() -> PartitionConfig {
        PartitionConfig {
            partition_type: PartitionType::Time {
                granularity: TimeGranularity::Hourly,
            },
            partition_key: "hour".to_string(),
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
    fn test_validate_same_type_dependency() {
        let upstream = daily_config();
        let downstream = daily_config();

        assert!(PartitionValidator::validate_dependency(&upstream, &downstream).is_ok());
    }

    #[test]
    fn test_validate_daily_to_weekly_dependency() {
        let upstream = daily_config();
        let downstream = weekly_config();

        assert!(PartitionValidator::validate_dependency(&upstream, &downstream).is_ok());
    }

    #[test]
    fn test_validate_weekly_to_daily_fails() {
        let upstream = weekly_config();
        let downstream = daily_config();

        assert!(PartitionValidator::validate_dependency(&upstream, &downstream).is_err());
    }

    #[test]
    fn test_validate_hourly_to_daily_dependency() {
        let upstream = hourly_config();
        let downstream = daily_config();

        assert!(PartitionValidator::validate_dependency(&upstream, &downstream).is_ok());
    }

    #[test]
    fn test_validate_daily_key_format() {
        let pt = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };

        assert!(PartitionValidator::validate_key_format("2024-01-15", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("invalid", &pt).is_err());
        assert!(PartitionValidator::validate_key_format("2024-13-01", &pt).is_err());
    }

    #[test]
    fn test_validate_hourly_key_format() {
        let pt = PartitionType::Time {
            granularity: TimeGranularity::Hourly,
        };

        assert!(PartitionValidator::validate_key_format("2024-01-15 13:00:00", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("2024-01-15T13:00:00", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("2024-01-15", &pt).is_err());
    }

    #[test]
    fn test_validate_monthly_key_format() {
        let pt = PartitionType::Time {
            granularity: TimeGranularity::Monthly,
        };

        assert!(PartitionValidator::validate_key_format("2024-01", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("2024-12", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("2024-13", &pt).is_err());
        assert!(PartitionValidator::validate_key_format("2024-1-15", &pt).is_err());
    }

    #[test]
    fn test_validate_yearly_key_format() {
        let pt = PartitionType::Time {
            granularity: TimeGranularity::Yearly,
        };

        assert!(PartitionValidator::validate_key_format("2024", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("24", &pt).is_err());
        assert!(PartitionValidator::validate_key_format("2024-01", &pt).is_err());
    }

    #[test]
    fn test_validate_weekly_key_format() {
        let pt = PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        };

        assert!(PartitionValidator::validate_key_format("2024-W03", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("2024-01-15", &pt).is_ok());
        // Monday
    }

    #[test]
    fn test_validate_range_key() {
        let pt = PartitionType::Range {
            lower_bound: "a".to_string(),
            upper_bound: "z".to_string(),
        };

        assert!(PartitionValidator::validate_key_format("m", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("a", &pt).is_ok());
        assert!(PartitionValidator::validate_key_format("z", &pt).is_err()); // upper exclusive
    }

    #[test]
    fn test_validate_backfill_range() {
        let config = daily_config();

        assert!(
            PartitionValidator::validate_backfill_range("2024-01-01", "2024-01-31", &config)
                .is_ok()
        );

        // End before start
        assert!(
            PartitionValidator::validate_backfill_range("2024-01-31", "2024-01-01", &config)
                .is_err()
        );

        // Invalid format
        assert!(
            PartitionValidator::validate_backfill_range("invalid", "2024-01-31", &config).is_err()
        );
    }

    #[test]
    fn test_can_auto_map() {
        let daily = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };
        let weekly = PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        };

        assert!(PartitionValidator::can_auto_map(&daily, &daily));
        assert!(PartitionValidator::can_auto_map(&daily, &weekly));
        assert!(!PartitionValidator::can_auto_map(&weekly, &daily));
    }

    #[test]
    fn test_expected_format() {
        let daily = PartitionType::Time {
            granularity: TimeGranularity::Daily,
        };
        assert_eq!(PartitionValidator::expected_format(&daily), "YYYY-MM-DD");

        let hourly = PartitionType::Time {
            granularity: TimeGranularity::Hourly,
        };
        assert!(PartitionValidator::expected_format(&hourly).contains("HH:00:00"));
    }

    #[test]
    fn test_partition_config_ext() {
        let config = daily_config();
        assert!(config.validate().is_ok());
        assert!(config.validate_key("2024-01-15").is_ok());
        assert!(config.validate_key("invalid").is_err());
    }

    #[test]
    fn test_validate_with_identity_mapping() {
        let upstream = daily_config();
        let downstream = daily_config();
        let mapping = PartitionMapping::Identity;

        assert!(PartitionValidator::validate_dependency_with_mapping(
            &upstream,
            &downstream,
            &mapping
        )
        .is_ok());
    }

    #[test]
    fn test_validate_identity_with_different_types_fails() {
        let upstream = daily_config();
        let downstream = hourly_config();
        let mapping = PartitionMapping::Identity;

        assert!(PartitionValidator::validate_dependency_with_mapping(
            &upstream,
            &downstream,
            &mapping
        )
        .is_err());
    }
}
