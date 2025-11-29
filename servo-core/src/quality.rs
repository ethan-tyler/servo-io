//! Data quality check definitions and execution
//!
//! This module provides core types for defining and executing data quality
//! checks on assets. Checks can be blocking (halt downstream execution on failure)
//! or non-blocking (log and continue).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a data quality check
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckId(pub Uuid);

impl CheckId {
    /// Create a new random check ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a CheckId from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for CheckId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CheckId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for CheckId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

/// Types of data quality checks that can be applied to assets
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckType {
    /// Column(s) cannot contain null values
    NotNull {
        /// Columns to check for nulls
        columns: Vec<String>,
    },

    /// Column values must be unique (no duplicates)
    Unique {
        /// Columns that together must be unique
        columns: Vec<String>,
    },

    /// Numeric column values must be within a specified range
    InRange {
        /// Column to check
        column: String,
        /// Minimum value (inclusive), if any
        min: Option<f64>,
        /// Maximum value (inclusive), if any
        max: Option<f64>,
    },

    /// String column values must match a regular expression
    Regex {
        /// Column to check
        column: String,
        /// Regular expression pattern
        pattern: String,
    },

    /// Table row count must be within specified bounds
    RowCount {
        /// Minimum row count (inclusive), if any
        min: Option<i64>,
        /// Maximum row count (inclusive), if any
        max: Option<i64>,
    },

    /// Data must have been updated within a specified time window
    Freshness {
        /// Column containing timestamp to check
        timestamp_column: String,
        /// Maximum age in seconds
        max_age_seconds: u64,
    },

    /// Column values must be one of a predefined set
    AcceptedValues {
        /// Column to check
        column: String,
        /// Set of allowed values
        values: Vec<serde_json::Value>,
    },

    /// Custom check with user-defined logic
    Custom {
        /// Name of the custom check
        name: String,
        /// Description of what the check validates
        description: String,
    },

    /// All values in column must exist in a reference table
    ReferentialIntegrity {
        /// Column to check
        column: String,
        /// Reference table name
        reference_table: String,
        /// Column in reference table
        reference_column: String,
    },

    /// Schema must match expected structure
    SchemaMatch {
        /// Expected column names
        expected_columns: Vec<String>,
        /// Whether to allow additional columns not in the expected list
        allow_extra_columns: bool,
    },
}

impl CheckType {
    /// Get the check type name for metrics and logging
    pub fn type_name(&self) -> &'static str {
        match self {
            CheckType::NotNull { .. } => "not_null",
            CheckType::Unique { .. } => "unique",
            CheckType::InRange { .. } => "in_range",
            CheckType::Regex { .. } => "regex",
            CheckType::RowCount { .. } => "row_count",
            CheckType::Freshness { .. } => "freshness",
            CheckType::AcceptedValues { .. } => "accepted_values",
            CheckType::Custom { .. } => "custom",
            CheckType::ReferentialIntegrity { .. } => "referential_integrity",
            CheckType::SchemaMatch { .. } => "schema_match",
        }
    }
}

/// Severity level for check failures
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckSeverity {
    /// Informational - logged but doesn't affect workflow
    Info,

    /// Warning - logged prominently, may trigger alerts, but workflow continues
    Warning,

    /// Error - blocks downstream execution if blocking=true
    #[default]
    Error,
}

impl CheckSeverity {
    /// Returns true if this severity level can block workflow execution
    pub fn can_block(&self) -> bool {
        matches!(self, CheckSeverity::Error)
    }
}

impl std::fmt::Display for CheckSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckSeverity::Info => write!(f, "info"),
            CheckSeverity::Warning => write!(f, "warning"),
            CheckSeverity::Error => write!(f, "error"),
        }
    }
}

/// Outcome of a check execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckOutcome {
    /// Check passed - data meets quality criteria
    Passed,

    /// Check failed - data does not meet quality criteria
    Failed,

    /// Check was skipped (e.g., check disabled or upstream failed)
    Skipped,

    /// Check execution itself errored (not a data quality failure)
    Error,
}

impl CheckOutcome {
    /// Returns true if the check passed
    pub fn is_success(&self) -> bool {
        matches!(self, CheckOutcome::Passed)
    }

    /// Returns true if the check failed or errored
    pub fn is_failure(&self) -> bool {
        matches!(self, CheckOutcome::Failed | CheckOutcome::Error)
    }
}

impl std::fmt::Display for CheckOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckOutcome::Passed => write!(f, "passed"),
            CheckOutcome::Failed => write!(f, "failed"),
            CheckOutcome::Skipped => write!(f, "skipped"),
            CheckOutcome::Error => write!(f, "error"),
        }
    }
}

/// Definition of a data quality check attached to an asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetCheck {
    /// Unique identifier for this check
    pub id: CheckId,

    /// Human-readable name for the check
    pub name: String,

    /// Description of what this check validates
    pub description: Option<String>,

    /// ID of the asset this check is attached to
    pub asset_id: Uuid,

    /// Type and configuration of the check
    pub check_type: CheckType,

    /// Severity level for failures
    pub severity: CheckSeverity,

    /// Whether a failure should block downstream execution
    pub blocking: bool,

    /// Whether this check is currently enabled
    pub enabled: bool,

    /// Tags for categorization
    pub tags: Vec<String>,

    /// Owner of this check
    pub owner: Option<String>,

    /// Tenant ID for multi-tenancy
    pub tenant_id: Option<String>,

    /// When this check was created
    pub created_at: DateTime<Utc>,

    /// When this check was last updated
    pub updated_at: DateTime<Utc>,
}

impl AssetCheck {
    /// Create a new asset check with default settings
    pub fn new(name: impl Into<String>, asset_id: Uuid, check_type: CheckType) -> Self {
        let now = Utc::now();
        Self {
            id: CheckId::new(),
            name: name.into(),
            description: None,
            asset_id,
            check_type,
            severity: CheckSeverity::default(),
            blocking: true, // Fail-safe default
            enabled: true,
            tags: Vec::new(),
            owner: None,
            tenant_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a builder for constructing an asset check with fluent API
    pub fn builder(
        name: impl Into<String>,
        asset_id: Uuid,
        check_type: CheckType,
    ) -> AssetCheckBuilder {
        AssetCheckBuilder::new(name, asset_id, check_type)
    }

    /// Returns true if this check should block downstream execution on failure
    pub fn should_block_on_failure(&self) -> bool {
        self.enabled && self.blocking && self.severity.can_block()
    }

    /// Update the check's updated_at timestamp
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }
}

/// Builder for constructing asset checks with a fluent API
pub struct AssetCheckBuilder {
    name: String,
    asset_id: Uuid,
    check_type: CheckType,
    description: Option<String>,
    severity: CheckSeverity,
    blocking: bool,
    enabled: bool,
    tags: Vec<String>,
    owner: Option<String>,
    tenant_id: Option<String>,
}

impl AssetCheckBuilder {
    /// Create a new asset check builder
    pub fn new(name: impl Into<String>, asset_id: Uuid, check_type: CheckType) -> Self {
        Self {
            name: name.into(),
            asset_id,
            check_type,
            description: None,
            severity: CheckSeverity::default(),
            blocking: true,
            enabled: true,
            tags: Vec::new(),
            owner: None,
            tenant_id: None,
        }
    }

    /// Set the check description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the severity level
    pub fn severity(mut self, severity: CheckSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Set whether failures should block downstream execution
    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }

    /// Set whether the check is enabled
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Add a tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Set the owner
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Build the asset check
    pub fn build(self) -> AssetCheck {
        let now = Utc::now();
        AssetCheck {
            id: CheckId::new(),
            name: self.name,
            description: self.description,
            asset_id: self.asset_id,
            check_type: self.check_type,
            severity: self.severity,
            blocking: self.blocking,
            enabled: self.enabled,
            tags: self.tags,
            owner: self.owner,
            tenant_id: self.tenant_id,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Result of executing a data quality check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Unique identifier for this result
    pub id: Uuid,

    /// ID of the check that was executed
    pub check_id: CheckId,

    /// ID of the execution this result belongs to
    pub execution_id: Uuid,

    /// ID of the asset that was checked
    pub asset_id: Uuid,

    /// Outcome of the check
    pub outcome: CheckOutcome,

    /// Severity level (copied from check definition)
    pub severity: CheckSeverity,

    /// Whether this check was configured as blocking
    pub blocking: bool,

    /// Number of rows that failed the check (if applicable)
    pub failed_row_count: Option<i64>,

    /// Total number of rows checked (if applicable)
    pub total_row_count: Option<i64>,

    /// Error message if check failed or errored
    pub error_message: Option<String>,

    /// Sample of failed values for debugging
    pub failed_samples: Option<serde_json::Value>,

    /// Duration of check execution in milliseconds
    pub duration_ms: i64,

    /// When this check was executed
    pub executed_at: DateTime<Utc>,

    /// Additional metadata about the check result
    pub metadata: Option<serde_json::Value>,

    /// Tenant ID for multi-tenancy
    pub tenant_id: Option<String>,
}

impl CheckResult {
    /// Create a passed check result
    pub fn passed(check: &AssetCheck, execution_id: Uuid, duration_ms: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check.id,
            execution_id,
            asset_id: check.asset_id,
            outcome: CheckOutcome::Passed,
            severity: check.severity,
            blocking: check.blocking,
            failed_row_count: None,
            total_row_count: None,
            error_message: None,
            failed_samples: None,
            duration_ms,
            executed_at: Utc::now(),
            metadata: None,
            tenant_id: check.tenant_id.clone(),
        }
    }

    /// Create a failed check result
    pub fn failed(
        check: &AssetCheck,
        execution_id: Uuid,
        duration_ms: i64,
        error_message: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check.id,
            execution_id,
            asset_id: check.asset_id,
            outcome: CheckOutcome::Failed,
            severity: check.severity,
            blocking: check.blocking,
            failed_row_count: None,
            total_row_count: None,
            error_message: Some(error_message.into()),
            failed_samples: None,
            duration_ms,
            executed_at: Utc::now(),
            metadata: None,
            tenant_id: check.tenant_id.clone(),
        }
    }

    /// Create an error check result (check itself failed to execute)
    pub fn error(
        check: &AssetCheck,
        execution_id: Uuid,
        duration_ms: i64,
        error_message: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check.id,
            execution_id,
            asset_id: check.asset_id,
            outcome: CheckOutcome::Error,
            severity: check.severity,
            blocking: check.blocking,
            failed_row_count: None,
            total_row_count: None,
            error_message: Some(error_message.into()),
            failed_samples: None,
            duration_ms,
            executed_at: Utc::now(),
            metadata: None,
            tenant_id: check.tenant_id.clone(),
        }
    }

    /// Create a skipped check result
    pub fn skipped(check: &AssetCheck, execution_id: Uuid, reason: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            check_id: check.id,
            execution_id,
            asset_id: check.asset_id,
            outcome: CheckOutcome::Skipped,
            severity: check.severity,
            blocking: check.blocking,
            failed_row_count: None,
            total_row_count: None,
            error_message: Some(reason.into()),
            failed_samples: None,
            duration_ms: 0,
            executed_at: Utc::now(),
            metadata: None,
            tenant_id: check.tenant_id.clone(),
        }
    }

    /// Returns true if this result should block downstream execution
    pub fn should_block(&self) -> bool {
        self.blocking && self.outcome.is_failure() && self.severity.can_block()
    }

    /// Set the row counts for this result
    pub fn with_row_counts(mut self, failed: i64, total: i64) -> Self {
        self.failed_row_count = Some(failed);
        self.total_row_count = Some(total);
        self
    }

    /// Set the failed samples for this result
    pub fn with_failed_samples(mut self, samples: serde_json::Value) -> Self {
        self.failed_samples = Some(samples);
        self
    }

    /// Set additional metadata for this result
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Calculate failure rate as a percentage
    pub fn failure_rate(&self) -> Option<f64> {
        match (self.failed_row_count, self.total_row_count) {
            (Some(failed), Some(total)) if total > 0 => {
                Some((failed as f64 / total as f64) * 100.0)
            }
            _ => None,
        }
    }
}

/// Summary of check execution for an asset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckExecutionSummary {
    /// Total number of checks executed
    pub total: usize,

    /// Number of checks that passed
    pub passed: usize,

    /// Number of checks that failed
    pub failed: usize,

    /// Number of checks that had execution errors
    pub errors: usize,

    /// Number of checks that were skipped
    pub skipped: usize,

    /// Whether any blocking check failed
    pub has_blocking_failures: bool,

    /// Individual check results
    pub results: Vec<CheckResult>,
}

impl CheckExecutionSummary {
    /// Create a new empty summary
    pub fn new() -> Self {
        Self {
            total: 0,
            passed: 0,
            failed: 0,
            errors: 0,
            skipped: 0,
            has_blocking_failures: false,
            results: Vec::new(),
        }
    }

    /// Add a result to the summary
    pub fn add_result(&mut self, result: CheckResult) {
        self.total += 1;
        match result.outcome {
            CheckOutcome::Passed => self.passed += 1,
            CheckOutcome::Failed => self.failed += 1,
            CheckOutcome::Error => self.errors += 1,
            CheckOutcome::Skipped => self.skipped += 1,
        }
        if result.should_block() {
            self.has_blocking_failures = true;
        }
        self.results.push(result);
    }

    /// Create a summary from a list of results
    pub fn from_results(results: Vec<CheckResult>) -> Self {
        let mut summary = Self::new();
        for result in results {
            summary.add_result(result);
        }
        summary
    }
}

impl Default for CheckExecutionSummary {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_id_creation() {
        let id1 = CheckId::new();
        let id2 = CheckId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_check_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let check_id = CheckId::from_uuid(uuid);
        assert_eq!(check_id.0, uuid);
    }

    #[test]
    fn test_check_type_names() {
        assert_eq!(
            CheckType::NotNull {
                columns: vec!["id".to_string()]
            }
            .type_name(),
            "not_null"
        );
        assert_eq!(
            CheckType::Unique {
                columns: vec!["id".to_string()]
            }
            .type_name(),
            "unique"
        );
        assert_eq!(
            CheckType::InRange {
                column: "age".to_string(),
                min: Some(0.0),
                max: Some(150.0)
            }
            .type_name(),
            "in_range"
        );
    }

    #[test]
    fn test_check_severity_can_block() {
        assert!(!CheckSeverity::Info.can_block());
        assert!(!CheckSeverity::Warning.can_block());
        assert!(CheckSeverity::Error.can_block());
    }

    #[test]
    fn test_check_outcome_is_failure() {
        assert!(!CheckOutcome::Passed.is_failure());
        assert!(CheckOutcome::Failed.is_failure());
        assert!(CheckOutcome::Error.is_failure());
        assert!(!CheckOutcome::Skipped.is_failure());
    }

    #[test]
    fn test_asset_check_creation() {
        let asset_id = Uuid::new_v4();
        let check = AssetCheck::new(
            "not_null_check",
            asset_id,
            CheckType::NotNull {
                columns: vec!["customer_id".to_string()],
            },
        );

        assert_eq!(check.name, "not_null_check");
        assert_eq!(check.asset_id, asset_id);
        assert!(check.blocking);
        assert!(check.enabled);
        assert_eq!(check.severity, CheckSeverity::Error);
    }

    #[test]
    fn test_asset_check_builder() {
        let asset_id = Uuid::new_v4();
        let check = AssetCheck::builder(
            "unique_check",
            asset_id,
            CheckType::Unique {
                columns: vec!["order_id".to_string()],
            },
        )
        .description("Ensure order IDs are unique")
        .severity(CheckSeverity::Warning)
        .blocking(false)
        .tag("orders")
        .owner("data_team")
        .tenant_id("tenant_123")
        .build();

        assert_eq!(check.name, "unique_check");
        assert_eq!(
            check.description,
            Some("Ensure order IDs are unique".to_string())
        );
        assert_eq!(check.severity, CheckSeverity::Warning);
        assert!(!check.blocking);
        assert!(check.tags.contains(&"orders".to_string()));
        assert_eq!(check.owner, Some("data_team".to_string()));
        assert_eq!(check.tenant_id, Some("tenant_123".to_string()));
    }

    #[test]
    fn test_asset_check_should_block_on_failure() {
        let asset_id = Uuid::new_v4();

        // Blocking error check should block
        let check1 = AssetCheck::builder(
            "check1",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        )
        .severity(CheckSeverity::Error)
        .blocking(true)
        .build();
        assert!(check1.should_block_on_failure());

        // Non-blocking check should not block
        let check2 = AssetCheck::builder(
            "check2",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        )
        .blocking(false)
        .build();
        assert!(!check2.should_block_on_failure());

        // Warning severity should not block
        let check3 = AssetCheck::builder(
            "check3",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        )
        .severity(CheckSeverity::Warning)
        .blocking(true)
        .build();
        assert!(!check3.should_block_on_failure());

        // Disabled check should not block
        let check4 = AssetCheck::builder(
            "check4",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        )
        .enabled(false)
        .build();
        assert!(!check4.should_block_on_failure());
    }

    #[test]
    fn test_check_result_passed() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();
        let check = AssetCheck::new(
            "test_check",
            asset_id,
            CheckType::RowCount {
                min: Some(1),
                max: None,
            },
        );

        let result = CheckResult::passed(&check, execution_id, 100);

        assert_eq!(result.outcome, CheckOutcome::Passed);
        assert_eq!(result.check_id, check.id);
        assert_eq!(result.execution_id, execution_id);
        assert_eq!(result.duration_ms, 100);
        assert!(!result.should_block());
    }

    #[test]
    fn test_check_result_failed_blocking() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();
        let check = AssetCheck::new(
            "test_check",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        );

        let result = CheckResult::failed(&check, execution_id, 50, "Found 10 null values");

        assert_eq!(result.outcome, CheckOutcome::Failed);
        assert_eq!(
            result.error_message,
            Some("Found 10 null values".to_string())
        );
        assert!(result.should_block()); // Blocking error severity failure
    }

    #[test]
    fn test_check_result_with_row_counts() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();
        let check = AssetCheck::new(
            "test_check",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        );

        let result = CheckResult::failed(&check, execution_id, 50, "Null values found")
            .with_row_counts(10, 1000);

        assert_eq!(result.failed_row_count, Some(10));
        assert_eq!(result.total_row_count, Some(1000));
        assert!((result.failure_rate().unwrap() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_check_execution_summary() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();

        let check1 = AssetCheck::new(
            "check1",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        );
        let check2 = AssetCheck::builder(
            "check2",
            asset_id,
            CheckType::Unique {
                columns: vec!["id".to_string()],
            },
        )
        .blocking(false)
        .build();

        let result1 = CheckResult::passed(&check1, execution_id, 100);
        let result2 = CheckResult::failed(&check2, execution_id, 50, "Duplicates found");

        let summary = CheckExecutionSummary::from_results(vec![result1, result2]);

        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert!(!summary.has_blocking_failures); // check2 is non-blocking
    }

    #[test]
    fn test_check_execution_summary_with_blocking_failure() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();

        let check = AssetCheck::new(
            "blocking_check",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        );

        let result = CheckResult::failed(&check, execution_id, 50, "Null values found");
        let summary = CheckExecutionSummary::from_results(vec![result]);

        assert!(summary.has_blocking_failures);
    }

    #[test]
    fn test_check_type_serialization() {
        let check_type = CheckType::InRange {
            column: "age".to_string(),
            min: Some(0.0),
            max: Some(150.0),
        };

        let json = serde_json::to_string(&check_type).unwrap();
        let deserialized: CheckType = serde_json::from_str(&json).unwrap();

        assert_eq!(check_type, deserialized);
    }

    #[test]
    fn test_asset_check_serialization() {
        let asset_id = Uuid::new_v4();
        let check = AssetCheck::builder(
            "test_check",
            asset_id,
            CheckType::NotNull {
                columns: vec!["id".to_string()],
            },
        )
        .description("Test check")
        .severity(CheckSeverity::Warning)
        .tenant_id("tenant_123")
        .build();

        let json = serde_json::to_string(&check).unwrap();
        let deserialized: AssetCheck = serde_json::from_str(&json).unwrap();

        assert_eq!(check.name, deserialized.name);
        assert_eq!(check.description, deserialized.description);
        assert_eq!(check.severity, deserialized.severity);
        assert_eq!(check.tenant_id, deserialized.tenant_id);
    }

    #[test]
    fn test_check_result_serialization() {
        let asset_id = Uuid::new_v4();
        let execution_id = Uuid::new_v4();
        let check = AssetCheck::new(
            "test_check",
            asset_id,
            CheckType::RowCount {
                min: Some(1),
                max: None,
            },
        );

        let result = CheckResult::failed(&check, execution_id, 100, "Row count below minimum")
            .with_row_counts(0, 0)
            .with_metadata(serde_json::json!({"expected_min": 1}));

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: CheckResult = serde_json::from_str(&json).unwrap();

        assert_eq!(result.outcome, deserialized.outcome);
        assert_eq!(result.error_message, deserialized.error_message);
        assert_eq!(result.failed_row_count, deserialized.failed_row_count);
    }
}
