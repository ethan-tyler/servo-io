//! Partition command for inspecting partition information
//!
//! Provides commands to inspect partition status, dependencies, and list partitions.

use anyhow::Result;
use servo_core::{infer_mapping, PartitionConfig, PartitionMapper};
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;

/// Show status of a specific partition for an asset.
///
/// Displays the partition execution history and current state.
pub async fn status(
    asset_name: &str,
    partition_key: &str,
    tenant_id: &str,
    database_url: &str,
) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    // Find the asset
    let asset = storage.get_asset_by_name(asset_name, &tenant).await?;

    // Check partition config
    let partition_config: Option<PartitionConfig> = asset
        .partition_config
        .as_ref()
        .and_then(|j| serde_json::from_value(j.0.clone()).ok());

    println!("Asset: {}", asset.name);
    println!("Partition Key: {}", partition_key);

    if let Some(config) = &partition_config {
        println!("Partition Type: {:?}", config.partition_type);
    } else {
        println!("Partition Type: Not partitioned");
    }

    // Check if this partition key appears to be multi-dimensional
    if partition_key.starts_with('{') {
        if let Ok(dims) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(partition_key)
        {
            println!("\nDimensions:");
            for (name, value) in &dims {
                println!("  {}: {}", name, value);
            }
        }
    }

    // Look up any backfill partition records for this asset/partition
    let backfill_jobs = storage.list_backfill_jobs(&tenant, None, 0, 100).await?;
    let relevant_jobs: Vec<_> = backfill_jobs
        .iter()
        .filter(|j| j.asset_id == asset.id)
        .collect();

    if !relevant_jobs.is_empty() {
        println!("\nBackfill History:");
        for job in relevant_jobs {
            // Get partitions for this job to check if our partition is included
            let partitions = storage.list_backfill_partitions(job.id, &tenant).await?;
            if let Some(partition) = partitions.iter().find(|p| p.partition_key == partition_key) {
                println!(
                    "  Job {}: {} ({})",
                    job.id,
                    partition.state,
                    job.created_at.format("%Y-%m-%d %H:%M:%S")
                );
                if let Some(exec_id) = partition.execution_id {
                    println!("    Execution ID: {}", exec_id);
                }
                if let Some(duration) = partition.duration_ms {
                    println!("    Duration: {}ms", duration);
                }
            }
        }
    } else {
        println!("\nNo backfill history found.");
    }

    Ok(())
}

/// Show upstream partition dependencies for a specific partition.
///
/// Uses partition mapping to determine which upstream partitions are required.
pub async fn deps(
    asset_name: &str,
    partition_key: &str,
    tenant_id: &str,
    database_url: &str,
) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    // Find the asset
    let asset = storage.get_asset_by_name(asset_name, &tenant).await?;

    // Get partition config for downstream asset
    let downstream_config: Option<PartitionConfig> = asset
        .partition_config
        .as_ref()
        .and_then(|j| serde_json::from_value(j.0.clone()).ok());

    println!("Asset: {}", asset.name);
    println!("Partition Key: {}", partition_key);
    println!();

    // Get upstream dependencies
    let upstream_deps = storage.get_upstream_dependencies(asset.id, &tenant).await?;

    if upstream_deps.is_empty() {
        println!("No upstream dependencies.");
        return Ok(());
    }

    println!("Upstream Partition Dependencies:");
    for dep in upstream_deps {
        // Get the upstream asset
        let upstream_asset = storage.get_asset(dep.upstream_asset_id, &tenant).await?;
        let upstream_config: Option<PartitionConfig> = upstream_asset
            .partition_config
            .as_ref()
            .and_then(|j| serde_json::from_value(j.0.clone()).ok());

        // Infer mapping and resolve upstream partitions
        let upstream_keys =
            resolve_upstream_partitions(partition_key, &downstream_config, &upstream_config);

        println!("\n  {} ({}):", upstream_asset.name, dep.dependency_type);
        for key in upstream_keys {
            println!("    - {}", key);
        }
    }

    Ok(())
}

/// List partitions for an asset (from backfill records).
///
/// Shows partitions that have been processed or are pending in backfill jobs.
pub async fn list(
    asset_name: &str,
    tenant_id: &str,
    database_url: &str,
    limit: Option<usize>,
) -> Result<()> {
    let storage = Arc::new(PostgresStorage::new(database_url).await?);
    let tenant = TenantId::new(tenant_id);

    // Find the asset
    let asset = storage.get_asset_by_name(asset_name, &tenant).await?;

    println!("Asset: {}", asset.name);

    // Get partition config
    let partition_config: Option<PartitionConfig> = asset
        .partition_config
        .as_ref()
        .and_then(|j| serde_json::from_value(j.0.clone()).ok());

    if let Some(config) = &partition_config {
        println!("Partition Type: {:?}", config.partition_type);
    } else {
        println!("Note: Asset is not partitioned. Showing backfill records only.");
    }
    println!();

    // Get all backfill jobs for this asset
    let backfill_jobs = storage.list_backfill_jobs(&tenant, None, 0, 100).await?;
    let relevant_jobs: Vec<_> = backfill_jobs
        .iter()
        .filter(|j| j.asset_id == asset.id)
        .collect();

    if relevant_jobs.is_empty() {
        println!("No partition records found.");
        return Ok(());
    }

    // Collect all partitions across jobs
    let mut all_partitions = Vec::new();
    for job in &relevant_jobs {
        let partitions = storage.list_backfill_partitions(job.id, &tenant).await?;
        all_partitions.extend(partitions);
    }

    // Sort by partition key
    all_partitions.sort_by(|a, b| a.partition_key.cmp(&b.partition_key));

    // Dedupe by partition key (keep most recent)
    let mut seen = std::collections::HashSet::new();
    let partitions: Vec<_> = all_partitions
        .into_iter()
        .filter(|p| seen.insert(p.partition_key.clone()))
        .collect();

    let display_limit = limit.unwrap_or(50);
    let partitions_to_show: Vec<_> = partitions.iter().take(display_limit).collect();

    println!(
        "Partitions ({} total, showing {}):",
        partitions.len(),
        partitions_to_show.len()
    );
    println!(
        "{:<30} {:<12} {:<10}",
        "PARTITION KEY", "STATUS", "DURATION"
    );
    println!("{}", "-".repeat(54));

    for partition in partitions_to_show {
        let duration = partition
            .duration_ms
            .map(|d| format!("{}ms", d))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<30} {:<12} {:<10}",
            truncate(&partition.partition_key, 28),
            partition.state,
            duration
        );
    }

    if partitions.len() > display_limit {
        println!(
            "\n... and {} more partitions",
            partitions.len() - display_limit
        );
    }

    Ok(())
}

/// Helper to resolve upstream partition keys using partition mapping.
fn resolve_upstream_partitions(
    downstream_key: &str,
    downstream_config: &Option<PartitionConfig>,
    upstream_config: &Option<PartitionConfig>,
) -> Vec<String> {
    match (downstream_config, upstream_config) {
        (Some(down), Some(up)) => match infer_mapping(&up.partition_type, &down.partition_type) {
            Ok(mapping) => {
                let mapper = PartitionMapper::new(mapping);
                mapper.get_upstream_partitions(downstream_key)
            }
            Err(_) => vec![downstream_key.to_string()],
        },
        _ => vec![downstream_key.to_string()],
    }
}

/// Helper to truncate strings for display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
