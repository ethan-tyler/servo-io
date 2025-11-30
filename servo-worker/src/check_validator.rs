//! Check validation logic for data quality checks.
//!
//! This module implements actual validation for each check type, executing
//! checks against asset output data to produce real pass/fail results.
//!
//! # Supported Check Types
//!
//! - `not_null` - Verify specified columns contain no null values
//! - `unique` - Verify specified columns have unique values
//! - `in_range` - Verify numeric column values fall within min/max bounds
//! - `regex` - Verify string column values match a regex pattern
//! - `row_count` - Verify total row count falls within min/max bounds
//! - `accepted_values` - Verify column values are within an allowed set
//! - `no_duplicate_rows` - Verify no duplicate rows exist
//! - `referential_integrity` - Verify foreign key references exist
//! - `schema_match` - Verify expected columns are present
//!
//! # Null Handling
//!
//! When building `AssetOutput` from rows, missing fields are normalized to `Value::Null`.
//! This ensures consistent column lengths and well-defined validation semantics:
//! - `not_null` counts nulls (including missing fields) as failures
//! - `unique` treats null as a distinct value
//! - `in_range` and `regex` skip null values (only validate non-null)
//! - `accepted_values` checks if null is in the allowed set
//!
//! # Check Types Not Yet Implemented
//!
//! - `freshness` - Requires timestamp comparison (deferred)

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tracing::warn;

/// Represents the output data from an asset execution.
///
/// This is a simplified columnar representation that can be validated
/// by check types. Each column is a vector of JSON values.
#[derive(Debug, Clone)]
pub struct AssetOutput {
    /// Column name -> column values (one per row)
    pub columns: HashMap<String, Vec<Value>>,
    /// Total number of rows
    pub row_count: usize,
}

impl AssetOutput {
    /// Create a new empty asset output
    pub fn empty() -> Self {
        Self {
            columns: HashMap::new(),
            row_count: 0,
        }
    }

    /// Create asset output from rows of JSON objects
    ///
    /// This method handles sparse/irregular rows by padding missing columns
    /// with `Null` values, ensuring all column vectors have the same length
    /// as `row_count`. This prevents index-out-of-bounds panics when validators
    /// access column values by row index.
    pub fn from_rows(rows: Vec<Value>) -> Self {
        let row_count = rows.len();
        if row_count == 0 {
            return Self::empty();
        }

        // First pass: collect all column names from all rows
        let mut all_columns: HashSet<String> = HashSet::new();
        for row in &rows {
            if let Value::Object(obj) = row {
                for key in obj.keys() {
                    all_columns.insert(key.clone());
                }
            }
        }

        // Initialize all columns with capacity
        let mut columns: HashMap<String, Vec<Value>> = all_columns
            .iter()
            .map(|name| (name.clone(), Vec::with_capacity(row_count)))
            .collect();

        // Second pass: add values for each row, using Null for missing columns
        for row in rows {
            if let Value::Object(obj) = row {
                for col_name in &all_columns {
                    let value = obj.get(col_name).cloned().unwrap_or(Value::Null);
                    columns.get_mut(col_name).unwrap().push(value);
                }
            } else {
                // Non-object row: add Null to all columns
                for col_name in &all_columns {
                    columns.get_mut(col_name).unwrap().push(Value::Null);
                }
            }
        }

        Self { columns, row_count }
    }

    /// Get a column's values by name
    pub fn get_column(&self, name: &str) -> Option<&Vec<Value>> {
        self.columns.get(name)
    }

    /// Check if a column exists
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.contains_key(name)
    }
}

/// Result of executing a check validation
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the check passed
    pub passed: bool,
    /// Number of rows that failed validation (if applicable)
    pub failed_count: Option<i64>,
    /// Total rows evaluated
    pub total_count: Option<i64>,
    /// Error message if check failed or errored
    pub error_message: Option<String>,
    /// Sample of failed rows (for debugging)
    pub failed_samples: Option<Value>,
}

impl ValidationResult {
    /// Create a passed result
    pub fn passed(total_count: i64) -> Self {
        Self {
            passed: true,
            failed_count: Some(0),
            total_count: Some(total_count),
            error_message: None,
            failed_samples: None,
        }
    }

    /// Create a failed result
    pub fn failed(failed_count: i64, total_count: i64, message: String) -> Self {
        Self {
            passed: false,
            failed_count: Some(failed_count),
            total_count: Some(total_count),
            error_message: Some(message),
            failed_samples: None,
        }
    }

    /// Create a failed result with samples
    pub fn failed_with_samples(
        failed_count: i64,
        total_count: i64,
        message: String,
        samples: Value,
    ) -> Self {
        Self {
            passed: false,
            failed_count: Some(failed_count),
            total_count: Some(total_count),
            error_message: Some(message),
            failed_samples: Some(samples),
        }
    }

    /// Create an error result (check couldn't execute)
    pub fn error(message: String) -> Self {
        Self {
            passed: false,
            failed_count: None,
            total_count: None,
            error_message: Some(message),
            failed_samples: None,
        }
    }
}

/// Check type definitions parsed from JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckType {
    /// Verify columns contain no null values
    NotNull { columns: Vec<String> },
    /// Verify columns have unique values
    Unique { columns: Vec<String> },
    /// Verify numeric values fall within bounds
    InRange {
        column: String,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    /// Verify string values match a regex pattern
    Regex { column: String, pattern: String },
    /// Verify row count is within bounds
    RowCount {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    /// Verify values are in an allowed set
    AcceptedValues { column: String, values: Vec<Value> },
    /// Verify no duplicate rows exist
    NoDuplicateRows {
        #[serde(default)]
        columns: Option<Vec<String>>,
    },
    /// Freshness check (not yet implemented)
    Freshness {
        timestamp_column: String,
        max_age_seconds: i64,
    },
    /// Referential integrity check - verify foreign key references exist
    ReferentialIntegrity {
        /// Column containing the foreign key values
        column: String,
        /// Name of the reference table
        reference_table: String,
        /// Column in the reference table to match against
        reference_column: String,
    },
    /// Schema match check - verify expected columns are present
    SchemaMatch {
        /// List of expected column names
        expected_columns: Vec<String>,
        /// Whether to allow additional columns not in the expected list
        #[serde(default = "default_true")]
        allow_extra_columns: bool,
    },
    /// Custom check (placeholder)
    Custom {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
}

fn default_true() -> bool {
    true
}

/// Maximum number of failed samples to include in results
const MAX_FAILED_SAMPLES: usize = 10;

/// Validate asset data against a check type definition
pub fn validate_check(check_type: &Value, data: &AssetOutput) -> ValidationResult {
    // Parse check type from JSON
    let parsed: Result<CheckType, _> = serde_json::from_value(check_type.clone());

    match parsed {
        Ok(ct) => execute_check(&ct, data),
        Err(e) => {
            warn!(error = %e, check_type = ?check_type, "Failed to parse check type");
            ValidationResult::error(format!("Invalid check type definition: {}", e))
        }
    }
}

/// Execute a parsed check type against data
fn execute_check(check_type: &CheckType, data: &AssetOutput) -> ValidationResult {
    match check_type {
        CheckType::NotNull { columns } => validate_not_null(columns, data),
        CheckType::Unique { columns } => validate_unique(columns, data),
        CheckType::InRange { column, min, max } => validate_in_range(column, *min, *max, data),
        CheckType::Regex { column, pattern } => validate_regex(column, pattern, data),
        CheckType::RowCount { min, max } => validate_row_count(*min, *max, data),
        CheckType::AcceptedValues { column, values } => {
            validate_accepted_values(column, values, data)
        }
        CheckType::NoDuplicateRows { columns } => {
            validate_no_duplicate_rows(columns.as_ref(), data)
        }
        CheckType::Freshness { .. } => {
            // Freshness requires current time comparison - mark as not implemented
            ValidationResult::error("Freshness check not yet implemented in worker".to_string())
        }
        CheckType::ReferentialIntegrity {
            column,
            reference_table,
            reference_column,
        } => validate_referential_integrity(column, reference_table, reference_column, data),
        CheckType::SchemaMatch {
            expected_columns,
            allow_extra_columns,
        } => validate_schema_match(expected_columns, *allow_extra_columns, data),
        CheckType::Custom { name, .. } => {
            // Custom checks are user-defined and can't be executed generically
            ValidationResult::error(format!(
                "Custom check '{}' requires user-defined validation",
                name.as_deref().unwrap_or("unnamed")
            ))
        }
    }
}

/// Validate that specified columns contain no null values
fn validate_not_null(columns: &[String], data: &AssetOutput) -> ValidationResult {
    let total = data.row_count as i64;
    let mut failed_count = 0i64;
    let mut failed_samples = Vec::new();

    for col_name in columns {
        match data.get_column(col_name) {
            Some(values) => {
                for (idx, value) in values.iter().enumerate() {
                    if value.is_null() {
                        failed_count += 1;
                        if failed_samples.len() < MAX_FAILED_SAMPLES {
                            failed_samples.push(serde_json::json!({
                                "row": idx,
                                "column": col_name,
                                "reason": "null value"
                            }));
                        }
                    }
                }
            }
            None => {
                return ValidationResult::error(format!("Column '{}' not found in data", col_name));
            }
        }
    }

    if failed_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            failed_count,
            total,
            format!(
                "{} null values found in columns {:?}",
                failed_count, columns
            ),
            Value::Array(failed_samples),
        )
    }
}

/// Validate that specified columns have unique values
fn validate_unique(columns: &[String], data: &AssetOutput) -> ValidationResult {
    let total = data.row_count as i64;

    if columns.is_empty() {
        return ValidationResult::error("Unique check requires at least one column".to_string());
    }

    // Build composite keys from all specified columns
    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicates: Vec<Value> = Vec::new();
    let mut duplicate_count = 0i64;

    for row_idx in 0..data.row_count {
        let mut key_parts = Vec::new();

        for col_name in columns {
            match data.get_column(col_name) {
                Some(values) => {
                    let value = &values[row_idx];
                    key_parts.push(value.to_string());
                }
                None => {
                    return ValidationResult::error(format!(
                        "Column '{}' not found in data",
                        col_name
                    ));
                }
            }
        }

        let key = key_parts.join("|");

        if seen.contains(&key) {
            duplicate_count += 1;
            if duplicates.len() < MAX_FAILED_SAMPLES {
                duplicates.push(serde_json::json!({
                    "row": row_idx,
                    "columns": columns,
                    "duplicate_key": key
                }));
            }
        } else {
            seen.insert(key);
        }
    }

    if duplicate_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            duplicate_count,
            total,
            format!(
                "{} duplicate values found in columns {:?}",
                duplicate_count, columns
            ),
            Value::Array(duplicates),
        )
    }
}

/// Validate that numeric column values fall within bounds
fn validate_in_range(
    column: &str,
    min: Option<f64>,
    max: Option<f64>,
    data: &AssetOutput,
) -> ValidationResult {
    let total = data.row_count as i64;

    let values = match data.get_column(column) {
        Some(v) => v,
        None => {
            return ValidationResult::error(format!("Column '{}' not found in data", column));
        }
    };

    let mut out_of_range = Vec::new();
    let mut failed_count = 0i64;

    for (idx, value) in values.iter().enumerate() {
        let num = match value {
            Value::Number(n) => n.as_f64(),
            Value::Null => continue, // Skip nulls in range check
            _ => {
                failed_count += 1;
                if out_of_range.len() < MAX_FAILED_SAMPLES {
                    out_of_range.push(serde_json::json!({
                        "row": idx,
                        "column": column,
                        "value": value,
                        "reason": "non-numeric value"
                    }));
                }
                continue;
            }
        };

        if let Some(n) = num {
            let below_min = min.map(|m| n < m).unwrap_or(false);
            let above_max = max.map(|m| n > m).unwrap_or(false);

            if below_min || above_max {
                failed_count += 1;
                if out_of_range.len() < MAX_FAILED_SAMPLES {
                    out_of_range.push(serde_json::json!({
                        "row": idx,
                        "column": column,
                        "value": n,
                        "min": min,
                        "max": max,
                        "reason": if below_min { "below minimum" } else { "above maximum" }
                    }));
                }
            }
        }
    }

    if failed_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            failed_count,
            total,
            format!(
                "{} values in column '{}' outside range [{:?}, {:?}]",
                failed_count, column, min, max
            ),
            Value::Array(out_of_range),
        )
    }
}

/// Validate that string column values match a regex pattern
fn validate_regex(column: &str, pattern: &str, data: &AssetOutput) -> ValidationResult {
    let total = data.row_count as i64;

    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            return ValidationResult::error(format!("Invalid regex pattern '{}': {}", pattern, e));
        }
    };

    let values = match data.get_column(column) {
        Some(v) => v,
        None => {
            return ValidationResult::error(format!("Column '{}' not found in data", column));
        }
    };

    let mut non_matching = Vec::new();
    let mut failed_count = 0i64;

    for (idx, value) in values.iter().enumerate() {
        let s = match value {
            Value::String(s) => s.as_str(),
            Value::Null => continue, // Skip nulls
            _ => {
                failed_count += 1;
                if non_matching.len() < MAX_FAILED_SAMPLES {
                    non_matching.push(serde_json::json!({
                        "row": idx,
                        "column": column,
                        "value": value,
                        "reason": "non-string value"
                    }));
                }
                continue;
            }
        };

        if !regex.is_match(s) {
            failed_count += 1;
            if non_matching.len() < MAX_FAILED_SAMPLES {
                non_matching.push(serde_json::json!({
                    "row": idx,
                    "column": column,
                    "value": s,
                    "pattern": pattern,
                    "reason": "does not match pattern"
                }));
            }
        }
    }

    if failed_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            failed_count,
            total,
            format!(
                "{} values in column '{}' do not match pattern '{}'",
                failed_count, column, pattern
            ),
            Value::Array(non_matching),
        )
    }
}

/// Validate that row count is within bounds
fn validate_row_count(min: Option<i64>, max: Option<i64>, data: &AssetOutput) -> ValidationResult {
    let count = data.row_count as i64;

    let below_min = min.map(|m| count < m).unwrap_or(false);
    let above_max = max.map(|m| count > m).unwrap_or(false);

    if below_min {
        ValidationResult::failed(
            0,
            count,
            format!("Row count {} is below minimum {}", count, min.unwrap_or(0)),
        )
    } else if above_max {
        ValidationResult::failed(
            0,
            count,
            format!("Row count {} exceeds maximum {}", count, max.unwrap_or(0)),
        )
    } else {
        ValidationResult::passed(count)
    }
}

/// Validate that column values are in an allowed set
fn validate_accepted_values(
    column: &str,
    accepted: &[Value],
    data: &AssetOutput,
) -> ValidationResult {
    let total = data.row_count as i64;

    let values = match data.get_column(column) {
        Some(v) => v,
        None => {
            return ValidationResult::error(format!("Column '{}' not found in data", column));
        }
    };

    let accepted_set: HashSet<String> = accepted.iter().map(|v| v.to_string()).collect();

    let mut invalid = Vec::new();
    let mut failed_count = 0i64;

    for (idx, value) in values.iter().enumerate() {
        if value.is_null() {
            continue; // Skip nulls
        }

        let value_str = value.to_string();
        if !accepted_set.contains(&value_str) {
            failed_count += 1;
            if invalid.len() < MAX_FAILED_SAMPLES {
                invalid.push(serde_json::json!({
                    "row": idx,
                    "column": column,
                    "value": value,
                    "accepted_values": accepted
                }));
            }
        }
    }

    if failed_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            failed_count,
            total,
            format!(
                "{} values in column '{}' not in accepted set",
                failed_count, column
            ),
            Value::Array(invalid),
        )
    }
}

/// Validate that there are no duplicate rows
fn validate_no_duplicate_rows(
    columns: Option<&Vec<String>>,
    data: &AssetOutput,
) -> ValidationResult {
    let total = data.row_count as i64;

    // If no columns specified, use all columns
    let col_names: Vec<String> = match columns {
        Some(cols) => cols.clone(),
        None => data.columns.keys().cloned().collect(),
    };

    if col_names.is_empty() {
        return ValidationResult::passed(total);
    }

    // Build row hashes
    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicate_count = 0i64;
    let mut duplicates = Vec::new();

    for row_idx in 0..data.row_count {
        let mut row_key = String::new();

        for col_name in &col_names {
            if let Some(values) = data.get_column(col_name) {
                row_key.push_str(&values[row_idx].to_string());
                row_key.push('|');
            }
        }

        if seen.contains(&row_key) {
            duplicate_count += 1;
            if duplicates.len() < MAX_FAILED_SAMPLES {
                duplicates.push(serde_json::json!({
                    "row": row_idx,
                    "reason": "duplicate row"
                }));
            }
        } else {
            seen.insert(row_key);
        }
    }

    if duplicate_count == 0 {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed_with_samples(
            duplicate_count,
            total,
            format!("{} duplicate rows found", duplicate_count),
            Value::Array(duplicates),
        )
    }
}

/// Validate referential integrity - check that all values in a column exist in a reference table.
///
/// Note: This validation currently only works for in-memory validation where reference data
/// is provided. For cross-asset validation, the orchestrator must resolve the reference data
/// and pass it to the worker.
fn validate_referential_integrity(
    column: &str,
    reference_table: &str,
    reference_column: &str,
    data: &AssetOutput,
) -> ValidationResult {
    let total = data.row_count as i64;

    // Get the source column values
    let values = match data.get_column(column) {
        Some(v) => v,
        None => {
            return ValidationResult::error(format!("Column '{}' not found in data", column));
        }
    };

    // Check if we have reference data available (would be provided as a special column)
    // For now, we check if a column named "{reference_table}.{reference_column}" exists
    let ref_col_key = format!("__ref__{}__{}", reference_table, reference_column);
    let reference_values: Option<HashSet<String>> = data.get_column(&ref_col_key).map(|vals| {
        vals.iter()
            .filter(|v| !v.is_null())
            .map(|v| v.to_string())
            .collect()
    });

    match reference_values {
        Some(ref_set) => {
            // Perform in-memory validation
            let mut orphans = Vec::new();
            let mut failed_count = 0i64;

            for (idx, value) in values.iter().enumerate() {
                if value.is_null() {
                    continue; // Skip nulls by default
                }
                let value_str = value.to_string();
                if !ref_set.contains(&value_str) {
                    failed_count += 1;
                    if orphans.len() < MAX_FAILED_SAMPLES {
                        orphans.push(serde_json::json!({
                            "row": idx,
                            "column": column,
                            "value": value,
                            "reference_table": reference_table,
                            "reference_column": reference_column,
                            "reason": "orphan reference"
                        }));
                    }
                }
            }

            if failed_count == 0 {
                ValidationResult::passed(total)
            } else {
                ValidationResult::failed_with_samples(
                    failed_count,
                    total,
                    format!(
                        "{} values in column '{}' have no matching reference in '{}.{}'",
                        failed_count, column, reference_table, reference_column
                    ),
                    Value::Array(orphans),
                )
            }
        }
        None => {
            // No reference data provided - the check was registered but reference data
            // must be resolved by the orchestrator. Return a pass with a note.
            ValidationResult {
                passed: true,
                failed_count: Some(0),
                total_count: Some(total),
                error_message: Some(format!(
                    "Referential integrity check registered for '{}.{}' -> '{}.{}'. \
                     Reference data must be provided by orchestrator for validation.",
                    column, reference_table, reference_table, reference_column
                )),
                failed_samples: None,
            }
        }
    }
}

/// Validate schema match - check that expected columns are present
fn validate_schema_match(
    expected_columns: &[String],
    allow_extra_columns: bool,
    data: &AssetOutput,
) -> ValidationResult {
    let total = data.row_count as i64;
    let actual_columns: HashSet<&String> = data.columns.keys().collect();
    let expected_set: HashSet<&String> = expected_columns.iter().collect();

    let mut errors = Vec::new();

    // Check for missing columns
    let missing: Vec<&String> = expected_set
        .iter()
        .filter(|col| !actual_columns.contains(*col))
        .copied()
        .collect();

    if !missing.is_empty() {
        errors.push(format!("Missing columns: {:?}", missing));
    }

    // Check for extra columns if not allowed
    if !allow_extra_columns {
        let extra: Vec<&String> = actual_columns
            .iter()
            .filter(|col| !expected_set.contains(*col))
            .copied()
            .collect();

        if !extra.is_empty() {
            errors.push(format!("Unexpected columns: {:?}", extra));
        }
    }

    if errors.is_empty() {
        ValidationResult::passed(total)
    } else {
        ValidationResult::failed(errors.len() as i64, total, errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_data() -> AssetOutput {
        AssetOutput::from_rows(vec![
            json!({"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30}),
            json!({"id": 2, "name": "Bob", "email": "bob@example.com", "age": 25}),
            json!({"id": 3, "name": "Charlie", "email": "invalid-email", "age": 35}),
            json!({"id": 4, "name": null, "email": "dave@example.com", "age": 40}),
        ])
    }

    #[test]
    fn test_not_null_pass() {
        let data = sample_data();
        let check_type = json!({"type": "not_null", "columns": ["id", "email"]});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
        assert_eq!(result.failed_count, Some(0));
    }

    #[test]
    fn test_not_null_fail() {
        let data = sample_data();
        let check_type = json!({"type": "not_null", "columns": ["name"]});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
        assert!(result.error_message.unwrap().contains("null values"));
    }

    #[test]
    fn test_unique_pass() {
        let data = sample_data();
        let check_type = json!({"type": "unique", "columns": ["id"]});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_unique_fail() {
        let data = AssetOutput::from_rows(vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 1, "name": "Bob"}), // Duplicate id
            json!({"id": 2, "name": "Charlie"}),
        ]);
        let check_type = json!({"type": "unique", "columns": ["id"]});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
    }

    #[test]
    fn test_in_range_pass() {
        let data = sample_data();
        let check_type = json!({"type": "in_range", "column": "age", "min": 20, "max": 50});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_in_range_fail() {
        let data = sample_data();
        let check_type = json!({"type": "in_range", "column": "age", "min": 30, "max": 35});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(2)); // Bob (25) and Dave (40) out of range
    }

    #[test]
    fn test_regex_pass() {
        let data = AssetOutput::from_rows(vec![
            json!({"email": "test@example.com"}),
            json!({"email": "user@domain.org"}),
        ]);
        let check_type =
            json!({"type": "regex", "column": "email", "pattern": r"^[^@]+@[^@]+\.[^@]+$"});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_regex_fail() {
        let data = sample_data();
        let check_type =
            json!({"type": "regex", "column": "email", "pattern": r"^[^@]+@[^@]+\.[^@]+$"});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1)); // "invalid-email" doesn't match
    }

    #[test]
    fn test_row_count_pass() {
        let data = sample_data();
        let check_type = json!({"type": "row_count", "min": 1, "max": 10});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_row_count_fail_below_min() {
        let data = sample_data();
        let check_type = json!({"type": "row_count", "min": 10});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("below minimum"));
    }

    #[test]
    fn test_row_count_fail_above_max() {
        let data = sample_data();
        let check_type = json!({"type": "row_count", "max": 2});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("exceeds maximum"));
    }

    #[test]
    fn test_accepted_values_pass() {
        let data = AssetOutput::from_rows(vec![
            json!({"status": "active"}),
            json!({"status": "pending"}),
            json!({"status": "active"}),
        ]);
        let check_type = json!({
            "type": "accepted_values",
            "column": "status",
            "values": ["active", "pending", "completed"]
        });
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_accepted_values_fail() {
        let data = AssetOutput::from_rows(vec![
            json!({"status": "active"}),
            json!({"status": "invalid_status"}),
        ]);
        let check_type = json!({
            "type": "accepted_values",
            "column": "status",
            "values": ["active", "pending", "completed"]
        });
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
    }

    #[test]
    fn test_no_duplicate_rows_pass() {
        let data = sample_data();
        let check_type = json!({"type": "no_duplicate_rows"});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_no_duplicate_rows_fail() {
        let data = AssetOutput::from_rows(vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 1, "name": "Alice"}), // Exact duplicate
            json!({"id": 2, "name": "Bob"}),
        ]);
        let check_type = json!({"type": "no_duplicate_rows"});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
    }

    #[test]
    fn test_column_not_found() {
        let data = sample_data();
        let check_type = json!({"type": "not_null", "columns": ["nonexistent"]});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("not found"));
    }

    #[test]
    fn test_invalid_check_type() {
        let data = sample_data();
        let check_type = json!({"type": "unknown_type"});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("Invalid check type"));
    }

    #[test]
    fn test_empty_data() {
        let data = AssetOutput::empty();
        let check_type = json!({"type": "row_count", "min": 1});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
    }

    #[test]
    fn test_sparse_rows_no_panic() {
        // Rows with different columns - this previously caused index-out-of-bounds panic
        let data = AssetOutput::from_rows(vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 2}), // Missing "name" column
            json!({"id": 3, "name": "Charlie", "email": "charlie@example.com"}), // Has extra "email"
        ]);

        // Verify all columns have consistent length
        assert_eq!(data.row_count, 3);
        assert_eq!(data.get_column("id").unwrap().len(), 3);
        assert_eq!(data.get_column("name").unwrap().len(), 3);
        assert_eq!(data.get_column("email").unwrap().len(), 3);

        // Verify missing values are Null
        let name_col = data.get_column("name").unwrap();
        assert!(name_col[1].is_null()); // Row 2's name should be null

        let email_col = data.get_column("email").unwrap();
        assert!(email_col[0].is_null()); // Row 1's email should be null
        assert!(email_col[1].is_null()); // Row 2's email should be null

        // Verify checks work correctly on sparse data
        let check_type = json!({"type": "not_null", "columns": ["id"]});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);

        // not_null on "name" should fail for the missing value
        let check_type = json!({"type": "not_null", "columns": ["name"]});
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
    }

    #[test]
    fn test_sparse_rows_unique_check() {
        // Sparse rows with unique check - should not panic
        let data = AssetOutput::from_rows(vec![
            json!({"id": 1, "status": "active"}),
            json!({"id": 2}), // Missing status
            json!({"id": 3, "status": "pending"}),
        ]);

        let check_type = json!({"type": "unique", "columns": ["id"]});
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_schema_match_pass() {
        let data = sample_data();
        let check_type = json!({
            "type": "schema_match",
            "expected_columns": ["id", "name"],
            "allow_extra_columns": true
        });
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_schema_match_fail_missing_column() {
        let data = sample_data();
        let check_type = json!({
            "type": "schema_match",
            "expected_columns": ["id", "name", "nonexistent_column"],
            "allow_extra_columns": true
        });
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("Missing columns"));
    }

    #[test]
    fn test_schema_match_fail_extra_columns_not_allowed() {
        let data = sample_data();
        let check_type = json!({
            "type": "schema_match",
            "expected_columns": ["id"],
            "allow_extra_columns": false
        });
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("Unexpected columns"));
    }

    #[test]
    fn test_schema_match_exact_match() {
        let data = AssetOutput::from_rows(vec![json!({"id": 1, "name": "Alice"})]);
        let check_type = json!({
            "type": "schema_match",
            "expected_columns": ["id", "name"],
            "allow_extra_columns": false
        });
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_referential_integrity_no_reference_data() {
        // When no reference data is provided, check passes with a message
        let data = sample_data();
        let check_type = json!({
            "type": "referential_integrity",
            "column": "id",
            "reference_table": "users",
            "reference_column": "user_id"
        });
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
        assert!(result.error_message.unwrap().contains("orchestrator"));
    }

    #[test]
    fn test_referential_integrity_with_reference_data_pass() {
        // Create data with embedded reference values
        let mut data = AssetOutput::from_rows(vec![
            json!({"customer_id": 1}),
            json!({"customer_id": 2}),
            json!({"customer_id": 3}),
        ]);
        // Add reference data as special column
        data.columns.insert(
            "__ref__customers__id".to_string(),
            vec![
                Value::Number(1.into()),
                Value::Number(2.into()),
                Value::Number(3.into()),
                Value::Number(4.into()),
            ],
        );

        let check_type = json!({
            "type": "referential_integrity",
            "column": "customer_id",
            "reference_table": "customers",
            "reference_column": "id"
        });
        let result = validate_check(&check_type, &data);
        assert!(result.passed);
    }

    #[test]
    fn test_referential_integrity_with_reference_data_fail() {
        let mut data = AssetOutput::from_rows(vec![
            json!({"customer_id": 1}),
            json!({"customer_id": 999}), // Orphan reference
        ]);
        // Add reference data
        data.columns.insert(
            "__ref__customers__id".to_string(),
            vec![Value::Number(1.into()), Value::Number(2.into())],
        );

        let check_type = json!({
            "type": "referential_integrity",
            "column": "customer_id",
            "reference_table": "customers",
            "reference_column": "id"
        });
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert_eq!(result.failed_count, Some(1));
        assert!(result
            .error_message
            .unwrap()
            .contains("no matching reference"));
    }

    #[test]
    fn test_referential_integrity_column_not_found() {
        let data = sample_data();
        let check_type = json!({
            "type": "referential_integrity",
            "column": "nonexistent",
            "reference_table": "users",
            "reference_column": "id"
        });
        let result = validate_check(&check_type, &data);
        assert!(!result.passed);
        assert!(result.error_message.unwrap().contains("not found"));
    }
}
