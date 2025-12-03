//! Quality command for inspecting data quality checks and results
//!
//! Provides commands to list checks, view results, and inspect check definitions.

use anyhow::Result;
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;
use uuid::Uuid;

/// List all quality checks defined for an asset.
///
/// Displays check name, type, severity, and blocking status.
pub async fn list(asset_name: &str, tenant_id: &str, database_url: &str) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    // Find the asset
    let asset = storage.get_asset_by_name(asset_name, &tenant).await?;

    // List checks for this asset (including disabled checks)
    let checks = storage.list_asset_checks(asset.id, &tenant, false).await?;

    println!("Asset: {}", asset.name);
    println!();

    if checks.is_empty() {
        println!("No quality checks defined for this asset.");
        return Ok(());
    }

    println!("Quality Checks ({} total):", checks.len());
    println!(
        "{:<30} {:<20} {:<10} {:<10} {:<10}",
        "NAME", "TYPE", "SEVERITY", "BLOCKING", "ENABLED"
    );
    println!("{}", "-".repeat(82));

    for check in &checks {
        let check_type = extract_check_type(&check.check_type.0);
        let blocking = if check.blocking { "yes" } else { "no" };
        let enabled = if check.enabled { "yes" } else { "no" };

        println!(
            "{:<30} {:<20} {:<10} {:<10} {:<10}",
            truncate(&check.name, 28),
            truncate(&check_type, 18),
            check.severity,
            blocking,
            enabled
        );
    }

    Ok(())
}

/// Show quality check results for a specific execution.
///
/// Displays outcome, severity, and any failure details.
pub async fn results(execution_id: &str, tenant_id: &str, database_url: &str) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    let exec_id = Uuid::parse_str(execution_id)
        .map_err(|_| anyhow::anyhow!("Invalid execution ID format: {}", execution_id))?;

    // Get check results for this execution
    let results = storage
        .list_check_results_for_execution(exec_id, &tenant)
        .await?;

    println!("Execution: {}", execution_id);
    println!();

    if results.is_empty() {
        println!("No quality check results found for this execution.");
        return Ok(());
    }

    // Count outcomes
    let passed = results.iter().filter(|r| r.outcome == "passed").count();
    let failed = results.iter().filter(|r| r.outcome == "failed").count();
    let errored = results.iter().filter(|r| r.outcome == "error").count();
    let skipped = results.iter().filter(|r| r.outcome == "skipped").count();

    println!(
        "Summary: {} passed, {} failed, {} errored, {} skipped",
        passed, failed, errored, skipped
    );
    println!();

    println!(
        "{:<30} {:<10} {:<10} {:<10} {:<12}",
        "CHECK", "OUTCOME", "SEVERITY", "BLOCKING", "DURATION"
    );
    println!("{}", "-".repeat(74));

    for result in &results {
        // Get check name
        let check_name = match storage.get_asset_check(result.check_id, &tenant).await {
            Ok(check) => check.name,
            Err(_) => format!("{}", result.check_id),
        };

        let blocking = if result.blocking { "yes" } else { "no" };
        let duration = format!("{}ms", result.duration_ms);

        println!(
            "{:<30} {:<10} {:<10} {:<10} {:<12}",
            truncate(&check_name, 28),
            result.outcome,
            result.severity,
            blocking,
            duration
        );

        // Show failure details if any
        if result.outcome == "failed" || result.outcome == "error" {
            if let Some(ref error_msg) = result.error_message {
                println!("    Error: {}", truncate(error_msg, 70));
            }
            if let Some(failed_rows) = result.failed_row_count {
                let total = result.total_row_count.unwrap_or(0);
                if total > 0 {
                    let pct = (failed_rows as f64 / total as f64) * 100.0;
                    println!("    Failed rows: {} / {} ({:.1}%)", failed_rows, total, pct);
                } else {
                    println!("    Failed rows: {}", failed_rows);
                }
            }
        }
    }

    Ok(())
}

/// Show details of a specific quality check including recent history.
///
/// Displays check definition and last N results.
pub async fn show(check_id: &str, tenant_id: &str, limit: usize, database_url: &str) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    let id = Uuid::parse_str(check_id)
        .map_err(|_| anyhow::anyhow!("Invalid check ID format: {}", check_id))?;

    // Get the check definition
    let check = storage.get_asset_check(id, &tenant).await?;

    // Get the asset name
    let asset = storage.get_asset(check.asset_id, &tenant).await?;

    println!("Check: {}", check.name);
    println!("ID: {}", check.id);
    println!("Asset: {}", asset.name);
    println!();

    println!("Configuration:");
    println!("  Type: {}", extract_check_type(&check.check_type.0));
    println!("  Severity: {}", check.severity);
    println!("  Blocking: {}", if check.blocking { "yes" } else { "no" });
    println!("  Enabled: {}", if check.enabled { "yes" } else { "no" });

    if let Some(ref desc) = check.description {
        println!("  Description: {}", desc);
    }

    if let Some(ref owner) = check.owner {
        println!("  Owner: {}", owner);
    }

    if !check.tags.0.is_empty() {
        println!("  Tags: {}", check.tags.0.join(", "));
    }

    // Show check type details
    println!();
    println!("Check Definition:");
    println!("  {}", serde_json::to_string_pretty(&check.check_type.0)?);

    // Get recent results for this check
    let results = storage
        .list_recent_check_results_for_asset(check.asset_id, &tenant, limit as i64)
        .await?;

    // Filter to just this check
    let check_results: Vec<_> = results.iter().filter(|r| r.check_id == id).collect();

    println!();
    if check_results.is_empty() {
        println!("No execution history found.");
    } else {
        println!("Recent Results (last {}):", check_results.len());
        println!(
            "{:<38} {:<10} {:<12} {:<10}",
            "EXECUTION", "OUTCOME", "DURATION", "TIMESTAMP"
        );
        println!("{}", "-".repeat(72));

        for result in check_results {
            let duration = format!("{}ms", result.duration_ms);
            let timestamp = result.executed_at.format("%Y-%m-%d %H:%M");

            println!(
                "{:<38} {:<10} {:<12} {:<10}",
                result.execution_id, result.outcome, duration, timestamp
            );
        }
    }

    Ok(())
}

/// Extract a human-readable check type from the check_type JSON.
fn extract_check_type(check_type: &serde_json::Value) -> String {
    if let Some(obj) = check_type.as_object() {
        // Try common patterns
        if let Some(t) = obj.get("type").and_then(|v| v.as_str()) {
            return t.to_string();
        }
        if let Some(t) = obj.get("check_type").and_then(|v| v.as_str()) {
            return t.to_string();
        }
        // Return first key as type
        if let Some(key) = obj.keys().next() {
            return key.clone();
        }
    }
    "custom".to_string()
}

/// Helper to truncate strings for display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
