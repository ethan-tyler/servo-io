//! Backfill command for triggering partition backfills

use anyhow::{Context, Result};
use chrono::Utc;
use servo_storage::{BackfillJobModel, BackfillPartitionModel, Json, PostgresStorage, TenantId};
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Execute a single partition backfill
///
/// Creates a backfill job for the specified asset and partition key,
/// then triggers execution of that partition.
pub async fn execute_single_partition(
    asset_name: &str,
    partition_key: &str,
    database_url: &str,
) -> Result<Uuid> {
    info!(
        "Starting backfill for asset '{}' partition '{}'",
        asset_name, partition_key
    );

    // Get tenant ID from environment
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    // Create storage
    info!("Connecting to database...");
    let storage = Arc::new(PostgresStorage::new(database_url).await?);

    // Look up asset by name
    info!("Looking up asset '{}'...", asset_name);
    let assets = storage.list_assets(&tenant_id, 100, 0).await?;
    let asset = assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| anyhow::anyhow!("Asset '{}' not found", asset_name))?;

    info!("Found asset: {} ({})", asset.name, asset.id);

    // Generate idempotency key
    let idempotency_key = format!(
        "{}:{}:{}",
        asset.id,
        partition_key,
        Utc::now().format("%Y%m%d%H%M%S")
    );

    // Check for existing job with same idempotency key
    if let Some(existing_job) = storage
        .find_backfill_job_by_idempotency_key(&idempotency_key, &tenant_id)
        .await?
    {
        info!(
            "Found existing backfill job {} with state '{}'",
            existing_job.id, existing_job.state
        );
        return Ok(existing_job.id);
    }

    // Create backfill job
    let job_id = Uuid::new_v4();
    let now = Utc::now();

    let job = BackfillJobModel {
        id: job_id,
        tenant_id: tenant_id.as_str().to_string(),
        asset_id: asset.id,
        asset_name: asset.name.clone(),
        idempotency_key,
        state: "pending".to_string(),
        execution_strategy: Json(serde_json::json!({"type": "sequential"})),
        partition_start: Some(partition_key.to_string()),
        partition_end: Some(partition_key.to_string()),
        partition_keys: Json(vec![partition_key.to_string()]),
        total_partitions: 1,
        completed_partitions: 0,
        failed_partitions: 0,
        skipped_partitions: 0,
        include_upstream: false,
        error_message: None,
        created_by: std::env::var("USER").ok(),
        created_at: now,
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        version: 1,
    };

    storage.create_backfill_job(&job, &tenant_id).await?;
    info!("Created backfill job: {}", job_id);

    // Create partition record
    let partition = BackfillPartitionModel {
        id: Uuid::new_v4(),
        backfill_job_id: job_id,
        partition_key: partition_key.to_string(),
        state: "pending".to_string(),
        attempt_count: 0,
        execution_id: None,
        error_message: None,
        duration_ms: None,
        started_at: None,
        completed_at: None,
        tenant_id: tenant_id.as_str().to_string(),
    };

    storage
        .create_backfill_partition(&partition, &tenant_id)
        .await?;
    info!("Created partition record for '{}'", partition_key);

    // Update job to running state
    let mut running_job = job.clone();
    running_job.state = "running".to_string();
    running_job.started_at = Some(Utc::now());
    running_job.heartbeat_at = Some(Utc::now());
    storage
        .update_backfill_job(&running_job, &tenant_id)
        .await?;

    info!("Backfill job {} started successfully", job_id);
    println!("{}", job_id);

    Ok(job_id)
}

/// List backfill jobs
pub async fn list_jobs(
    status_filter: Option<&str>,
    database_url: &str,
) -> Result<Vec<BackfillJobModel>> {
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    let storage = PostgresStorage::new(database_url).await?;
    let jobs = storage
        .list_backfill_jobs(&tenant_id, status_filter, 100, 0)
        .await?;

    for job in &jobs {
        println!(
            "{}\t{}\t{}\t{}/{}\t{}",
            job.id,
            job.asset_name,
            job.state,
            job.completed_partitions,
            job.total_partitions,
            job.created_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(jobs)
}

/// Get backfill job status
pub async fn get_status(job_id: &str, database_url: &str) -> Result<BackfillJobModel> {
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    let job_uuid = job_id
        .parse::<Uuid>()
        .context("job_id must be a valid UUID")?;

    let storage = PostgresStorage::new(database_url).await?;
    let job = storage.get_backfill_job(job_uuid, &tenant_id).await?;

    println!("Job ID:      {}", job.id);
    println!("Asset:       {}", job.asset_name);
    println!("State:       {}", job.state);
    println!(
        "Progress:    {}/{} completed, {} failed",
        job.completed_partitions, job.total_partitions, job.failed_partitions
    );
    println!(
        "Created:     {}",
        job.created_at.format("%Y-%m-%d %H:%M:%S")
    );

    if let Some(started) = job.started_at {
        println!("Started:     {}", started.format("%Y-%m-%d %H:%M:%S"));
    }
    if let Some(completed) = job.completed_at {
        println!("Completed:   {}", completed.format("%Y-%m-%d %H:%M:%S"));
    }
    if let Some(error) = &job.error_message {
        println!("Error:       {}", error);
    }

    // List partitions
    let partitions = storage
        .list_backfill_partitions(job_uuid, &tenant_id)
        .await?;
    if !partitions.is_empty() {
        println!("\nPartitions:");
        for p in &partitions {
            let duration = p
                .duration_ms
                .map(|d| format!("{}ms", d))
                .unwrap_or_else(|| "-".to_string());
            println!("  {} \t{}\t{}", p.partition_key, p.state, duration);
        }
    }

    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idempotency_key_format() {
        let asset_id = Uuid::new_v4();
        let partition_key = "2024-01-15";
        let timestamp = "20240115120000";
        let key = format!("{}:{}:{}", asset_id, partition_key, timestamp);

        assert!(key.contains(&asset_id.to_string()));
        assert!(key.contains(partition_key));
        assert!(key.contains(timestamp));
    }

    #[test]
    fn test_partition_model_creation() {
        let job_id = Uuid::new_v4();
        let partition = BackfillPartitionModel {
            id: Uuid::new_v4(),
            backfill_job_id: job_id,
            partition_key: "2024-01-15".to_string(),
            state: "pending".to_string(),
            attempt_count: 0,
            execution_id: None,
            error_message: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            tenant_id: "test-tenant".to_string(),
        };

        assert_eq!(partition.backfill_job_id, job_id);
        assert_eq!(partition.state, "pending");
        assert_eq!(partition.attempt_count, 0);
    }

    #[test]
    fn test_job_model_creation() {
        let job = BackfillJobModel {
            id: Uuid::new_v4(),
            tenant_id: "test-tenant".to_string(),
            asset_id: Uuid::new_v4(),
            asset_name: "test_asset".to_string(),
            idempotency_key: "test-key".to_string(),
            state: "pending".to_string(),
            execution_strategy: Json(serde_json::json!({"type": "sequential"})),
            partition_start: Some("2024-01-01".to_string()),
            partition_end: Some("2024-01-31".to_string()),
            partition_keys: Json(vec!["2024-01-01".to_string()]),
            total_partitions: 1,
            completed_partitions: 0,
            failed_partitions: 0,
            skipped_partitions: 0,
            include_upstream: false,
            error_message: None,
            created_by: Some("test_user".to_string()),
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            heartbeat_at: None,
            version: 1,
        };

        assert_eq!(job.state, "pending");
        assert_eq!(job.total_partitions, 1);
        assert!(!job.include_upstream);
    }
}
