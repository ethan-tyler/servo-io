//! Backfill command for triggering partition backfills
//!
//! This module provides CLI commands for creating and managing backfill jobs.
//! Jobs are created in "pending" state and picked up by the BackfillExecutor
//! background service for execution.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use servo_storage::{BackfillJobModel, BackfillPartitionModel, Json, PostgresStorage, TenantId};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Generates a list of partition keys for a date range (inclusive).
/// Each partition key is in YYYY-MM-DD format.
fn generate_date_range_partitions(start: &str, end: &str) -> Result<Vec<String>> {
    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date '{}'. Expected YYYY-MM-DD format.", start))?;

    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date '{}'. Expected YYYY-MM-DD format.", end))?;

    if end_date < start_date {
        bail!(
            "End date '{}' is before start date '{}'. End must be >= start.",
            end,
            start
        );
    }

    // Safety limit to prevent accidental huge backfills
    let max_partitions = 366; // ~1 year of daily partitions
    let day_count = (end_date - start_date).num_days() + 1;
    if day_count > max_partitions {
        bail!(
            "Date range spans {} days, which exceeds maximum of {} partitions. \
             Please use smaller date ranges.",
            day_count,
            max_partitions
        );
    }

    let mut partitions = Vec::with_capacity(day_count as usize);
    let mut current = start_date;
    while current <= end_date {
        partitions.push(current.format("%Y-%m-%d").to_string());
        current += Duration::days(1);
    }

    Ok(partitions)
}

/// Validates partition key format.
/// Accepts: YYYY-MM-DD (daily), YYYY-MM-DDTHH (hourly), or custom strings.
/// Returns normalized partition key or error.
fn validate_partition_key(partition_key: &str) -> Result<String> {
    let trimmed = partition_key.trim();

    if trimmed.is_empty() {
        bail!("Partition key cannot be empty");
    }

    // Reject obviously invalid inputs
    if trimmed.len() > 64 {
        bail!("Partition key too long (max 64 characters)");
    }

    // Try to parse as date (YYYY-MM-DD)
    if let Ok(_date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(trimmed.to_string());
    }

    // Try to parse as datetime (YYYY-MM-DDTHH)
    if trimmed.len() == 13
        && trimmed.chars().nth(10) == Some('T')
        && NaiveDate::parse_from_str(&trimmed[..10], "%Y-%m-%d").is_ok()
    {
        if let Ok(h) = trimmed[11..13].parse::<u32>() {
            if h < 24 {
                return Ok(trimmed.to_string());
            }
        }
    }

    // Accept other formats but warn
    warn!(
        "Partition key '{}' is not a recognized date format (YYYY-MM-DD or YYYY-MM-DDTHH)",
        trimmed
    );
    Ok(trimmed.to_string())
}

/// Execute a single partition backfill
///
/// Creates a backfill job for the specified asset and partition key.
/// The job is created in "pending" state and will be picked up by the
/// BackfillExecutor background service for execution.
///
/// If `include_upstream` is true, the executor will discover and backfill
/// upstream dependencies first before processing this asset.
pub async fn execute_single_partition(
    asset_name: &str,
    partition_key: &str,
    include_upstream: bool,
    max_upstream_depth: i32,
    sla_deadline: Option<&str>,
    priority: i32,
    database_url: &str,
) -> Result<Uuid> {
    // Validate partition key format
    let partition_key = validate_partition_key(partition_key)?;

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

    // TODO: Validate that asset is partitioned and partition key matches the partitioning scheme
    // This requires adding partition_scheme field to AssetModel (future enhancement)
    // For now, we accept any partition key format for any asset

    // Check for existing active job on same asset/partition
    // This prevents duplicate backfills while one is still running
    // Note: RLS ensures this check is tenant-scoped
    if let Some(active_job) = storage
        .find_active_backfill_for_partition(asset.id, &partition_key, &tenant_id)
        .await?
    {
        warn!(
            "Active backfill job {} already exists for asset '{}' partition '{}' (state: {})",
            active_job.id, asset_name, partition_key, active_job.state
        );
        println!("{}", active_job.id);
        return Ok(active_job.id);
    }

    // Generate idempotency key (unique per asset + partition + timestamp)
    let idempotency_key = format!(
        "{}:{}:{}",
        asset.id,
        partition_key,
        Utc::now().format("%Y%m%d%H%M%S")
    );

    // Parse SLA deadline if provided
    let sla_deadline_at: Option<DateTime<Utc>> = if let Some(deadline) = sla_deadline {
        Some(
            DateTime::parse_from_rfc3339(deadline)
                .with_context(|| {
                    format!(
                        "Invalid SLA deadline '{}'. Expected ISO 8601 format (e.g., '2024-01-16T00:00:00Z')",
                        deadline
                    )
                })?
                .with_timezone(&Utc),
        )
    } else {
        None
    };

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
        include_upstream,
        error_message: None,
        created_by: std::env::var("USER").ok(),
        created_at: now,
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        version: 1,
        paused_at: None,
        checkpoint_partition_key: None,
        estimated_completion_at: None,
        avg_partition_duration_ms: None,
        parent_job_id: None,
        max_upstream_depth,
        upstream_job_count: 0,
        completed_upstream_jobs: 0,
        execution_order: 0,
        sla_deadline_at,
        priority,
    };

    storage.create_backfill_job(&job, &tenant_id).await?;

    if include_upstream {
        info!(
            "Created backfill job: {} (with upstream propagation, max_depth={})",
            job_id, max_upstream_depth
        );
    } else {
        info!("Created backfill job: {}", job_id);
    }

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

    info!(
        "Backfill job {} created. Will be picked up by the BackfillExecutor service.",
        job_id
    );

    println!("{}", job_id);
    Ok(job_id)
}

/// Execute a date range backfill
///
/// Creates a single backfill job with multiple partition records for each date
/// in the specified range (inclusive). The job is created in "pending" state
/// and will be picked up by the BackfillExecutor background service.
///
/// If `include_upstream` is true, the executor will discover and backfill
/// upstream dependencies first before processing this asset.
pub async fn execute_range_backfill(
    asset_name: &str,
    start_date: &str,
    end_date: &str,
    include_upstream: bool,
    max_upstream_depth: i32,
    sla_deadline: Option<&str>,
    priority: i32,
    database_url: &str,
) -> Result<Uuid> {
    // Generate partition keys from date range
    let partition_keys = generate_date_range_partitions(start_date, end_date)?;
    let partition_count = partition_keys.len();

    info!(
        "Starting range backfill for asset '{}' from {} to {} ({} partitions)",
        asset_name, start_date, end_date, partition_count
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

    // TODO: Validate that asset is partitioned and date range matches the partitioning scheme
    // This requires adding partition_scheme field to AssetModel (future enhancement)
    // For now, we accept any date range for any asset

    // Check for overlapping active jobs
    // We check if any partition in our range is already being backfilled
    // Note: RLS ensures this check is tenant-scoped
    for pk in &partition_keys {
        if let Some(active_job) = storage
            .find_active_backfill_for_partition(asset.id, pk, &tenant_id)
            .await?
        {
            warn!(
                "Active backfill job {} already exists for asset '{}' partition '{}' (state: {}). \
                 Aborting range backfill to prevent overlap.",
                active_job.id, asset_name, pk, active_job.state
            );
            bail!(
                "Cannot start range backfill: partition '{}' is already being backfilled by job {}",
                pk,
                active_job.id
            );
        }
    }

    // Generate idempotency key (unique per asset + range + timestamp)
    let idempotency_key = format!(
        "{}:{}:{}:{}",
        asset.id,
        start_date,
        end_date,
        Utc::now().format("%Y%m%d%H%M%S")
    );

    // Parse SLA deadline if provided
    let sla_deadline_at: Option<DateTime<Utc>> = if let Some(deadline) = sla_deadline {
        Some(
            DateTime::parse_from_rfc3339(deadline)
                .with_context(|| {
                    format!(
                        "Invalid SLA deadline '{}'. Expected ISO 8601 format (e.g., '2024-01-16T00:00:00Z')",
                        deadline
                    )
                })?
                .with_timezone(&Utc),
        )
    } else {
        None
    };

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
        partition_start: Some(start_date.to_string()),
        partition_end: Some(end_date.to_string()),
        partition_keys: Json(partition_keys.clone()),
        total_partitions: partition_count as i32,
        completed_partitions: 0,
        failed_partitions: 0,
        skipped_partitions: 0,
        include_upstream,
        error_message: None,
        created_by: std::env::var("USER").ok(),
        created_at: now,
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        version: 1,
        paused_at: None,
        checkpoint_partition_key: None,
        estimated_completion_at: None,
        avg_partition_duration_ms: None,
        parent_job_id: None,
        max_upstream_depth,
        upstream_job_count: 0,
        completed_upstream_jobs: 0,
        execution_order: 0,
        sla_deadline_at,
        priority,
    };

    storage.create_backfill_job(&job, &tenant_id).await?;

    if include_upstream {
        info!(
            "Created backfill job: {} (with upstream propagation, max_depth={})",
            job_id, max_upstream_depth
        );
    } else {
        info!("Created backfill job: {}", job_id);
    }

    // Create partition records in batch
    for pk in &partition_keys {
        let partition = BackfillPartitionModel {
            id: Uuid::new_v4(),
            backfill_job_id: job_id,
            partition_key: pk.clone(),
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
    }
    info!("Created {} partition records", partition_count);

    info!(
        "Backfill job {} created with {} partitions. Will be picked up by the BackfillExecutor service.",
        job_id, partition_count
    );

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

/// Cancel a backfill job
///
/// Cancels a pending or running backfill job. Running partitions will complete,
/// but no new partitions will be started. Pending partitions are marked as skipped.
pub async fn cancel_job(job_id: &str, reason: Option<&str>, database_url: &str) -> Result<()> {
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    let job_uuid = job_id
        .parse::<Uuid>()
        .context("job_id must be a valid UUID")?;

    info!("Cancelling backfill job {}", job_id);

    let storage = PostgresStorage::new(database_url).await?;
    storage
        .cancel_backfill_job(job_uuid, reason, &tenant_id)
        .await?;

    info!("Backfill job {} cancelled", job_id);
    println!("Job {} cancelled", job_id);
    Ok(())
}

/// Pause a running backfill job
///
/// Pauses a running backfill job at the next partition boundary. The current
/// partition will complete, but no new partitions will be started. The job
/// can be resumed later from where it left off.
pub async fn pause_job(job_id: &str, database_url: &str) -> Result<()> {
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    let job_uuid = job_id
        .parse::<Uuid>()
        .context("job_id must be a valid UUID")?;

    info!("Pausing backfill job {}", job_id);

    let storage = PostgresStorage::new(database_url).await?;

    // Pause the job - the storage layer handles finding the checkpoint
    storage
        .pause_backfill_job(job_uuid, &tenant_id)
        .await?;

    info!("Backfill job {} paused", job_id);

    // Get the updated job to show status
    let job = storage.get_backfill_job(job_uuid, &tenant_id).await?;

    let completed = job.completed_partitions;
    let total = job.total_partitions;
    let remaining = total - completed - job.failed_partitions - job.skipped_partitions;

    println!("Job {} paused", job_id);
    println!(
        "Progress: {}/{} completed ({} remaining)",
        completed, total, remaining
    );
    if let Some(cp) = &job.checkpoint_partition_key {
        println!("Checkpoint: {}", cp);
    }
    println!("Use 'servo backfill resume {}' to continue", job_id);

    Ok(())
}

/// Resume a paused backfill job
///
/// Resumes a paused backfill job from where it left off. The job will continue
/// processing from the checkpoint partition.
pub async fn resume_job(job_id: &str, database_url: &str) -> Result<()> {
    let tenant_id = std::env::var("TENANT_ID").context("TENANT_ID environment variable not set")?;
    let tenant_id = TenantId::new(&tenant_id);

    let job_uuid = job_id
        .parse::<Uuid>()
        .context("job_id must be a valid UUID")?;

    info!("Resuming backfill job {}", job_id);

    let storage = PostgresStorage::new(database_url).await?;

    // Get the job first to validate state
    let job = storage.get_backfill_job(job_uuid, &tenant_id).await?;

    if job.state != "paused" {
        bail!(
            "Cannot resume job in state '{}'. Only paused jobs can be resumed.",
            job.state
        );
    }

    storage.resume_backfill_job(job_uuid, &tenant_id).await?;

    info!("Backfill job {} resumed", job_id);

    let completed = job.completed_partitions;
    let total = job.total_partitions;
    let remaining = total - completed - job.failed_partitions - job.skipped_partitions;

    println!("Job {} resumed", job_id);
    println!(
        "Progress: {}/{} completed ({} remaining)",
        completed, total, remaining
    );
    if let Some(cp) = &job.checkpoint_partition_key {
        println!("Resuming from checkpoint: {}", cp);
    }

    Ok(())
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

    // Show upstream propagation info if enabled
    if job.include_upstream {
        println!(
            "Upstream:    enabled (depth={}, {}/{} upstream jobs completed)",
            job.max_upstream_depth, job.completed_upstream_jobs, job.upstream_job_count
        );
        if job.state == "waiting_upstream" {
            println!(
                "             Waiting for {} upstream jobs to complete",
                job.upstream_job_count - job.completed_upstream_jobs
            );
        }
    }

    // Show parent job info if this is a child job
    if let Some(parent_id) = job.parent_job_id {
        println!("Parent Job:  {}", parent_id);
        println!("Exec Order:  {} (lower executes first)", job.execution_order);
    }

    // Calculate progress percentage
    let progress_pct = if job.total_partitions > 0 {
        (job.completed_partitions as f64 / job.total_partitions as f64) * 100.0
    } else {
        0.0
    };

    let remaining =
        job.total_partitions - job.completed_partitions - job.failed_partitions - job.skipped_partitions;

    println!(
        "Progress:    {}/{} ({:.1}%) - {} remaining, {} failed, {} skipped",
        job.completed_partitions,
        job.total_partitions,
        progress_pct,
        remaining,
        job.failed_partitions,
        job.skipped_partitions
    );

    // Show rate if available
    if let Some(avg_ms) = job.avg_partition_duration_ms {
        if avg_ms > 0 {
            let rate_per_min = 60_000.0 / avg_ms as f64;
            println!(
                "Rate:        {:.1} partitions/min (avg {}ms/partition)",
                rate_per_min, avg_ms
            );
        }
    }

    // Show ETA if available and job is running
    if job.state == "running" {
        if let Some(eta) = job.estimated_completion_at {
            let now = Utc::now();
            if eta > now {
                let remaining_duration = eta - now;
                let hours = remaining_duration.num_hours();
                let mins = remaining_duration.num_minutes() % 60;
                let secs = remaining_duration.num_seconds() % 60;

                let eta_str = if hours > 0 {
                    format!("{}h {}m", hours, mins)
                } else if mins > 0 {
                    format!("{}m {}s", mins, secs)
                } else {
                    format!("{}s", secs)
                };

                println!(
                    "ETA:         {} ({})",
                    eta.format("%Y-%m-%d %H:%M:%S"),
                    eta_str
                );
            }
        }
    }

    println!(
        "Created:     {}",
        job.created_at.format("%Y-%m-%d %H:%M:%S")
    );

    if let Some(started) = job.started_at {
        println!("Started:     {}", started.format("%Y-%m-%d %H:%M:%S"));
    }
    if let Some(paused) = job.paused_at {
        println!("Paused:      {}", paused.format("%Y-%m-%d %H:%M:%S"));
    }
    if let Some(completed) = job.completed_at {
        println!("Completed:   {}", completed.format("%Y-%m-%d %H:%M:%S"));
    }
    if let Some(checkpoint) = &job.checkpoint_partition_key {
        println!("Checkpoint:  {}", checkpoint);
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
            paused_at: None,
            checkpoint_partition_key: None,
            estimated_completion_at: None,
            avg_partition_duration_ms: None,
            parent_job_id: None,
            max_upstream_depth: 0,
            upstream_job_count: 0,
            completed_upstream_jobs: 0,
            execution_order: 0,
            sla_deadline_at: None,
            priority: 0,
        };

        assert_eq!(job.state, "pending");
        assert_eq!(job.total_partitions, 1);
        assert!(!job.include_upstream);
    }

    // Partition key validation tests

    #[test]
    fn test_validate_partition_key_daily_format() {
        let result = validate_partition_key("2024-01-15");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-01-15");
    }

    #[test]
    fn test_validate_partition_key_hourly_format() {
        let result = validate_partition_key("2024-01-15T14");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-01-15T14");
    }

    #[test]
    fn test_validate_partition_key_trims_whitespace() {
        let result = validate_partition_key("  2024-01-15  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "2024-01-15");
    }

    #[test]
    fn test_validate_partition_key_rejects_empty() {
        let result = validate_partition_key("");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_partition_key_rejects_whitespace_only() {
        let result = validate_partition_key("   ");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_partition_key_rejects_too_long() {
        let long_key = "a".repeat(100);
        let result = validate_partition_key(&long_key);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too long"));
    }

    #[test]
    fn test_validate_partition_key_accepts_custom_format() {
        // Custom formats are accepted with a warning (logged but not returned)
        let result = validate_partition_key("custom-partition-123");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "custom-partition-123");
    }

    #[test]
    fn test_validate_partition_key_invalid_hour() {
        // Hour 25 is invalid, should still be accepted as custom format
        let result = validate_partition_key("2024-01-15T25");
        assert!(result.is_ok()); // Accepted as custom format
    }

    #[test]
    fn test_validate_partition_key_invalid_date() {
        // Feb 30 is invalid, should still be accepted as custom format
        let result = validate_partition_key("2024-02-30");
        assert!(result.is_ok()); // Accepted as custom format (NaiveDate::parse fails)
    }

    // Date range generation tests

    #[test]
    fn test_generate_date_range_single_day() {
        let result = generate_date_range_partitions("2024-01-15", "2024-01-15");
        assert!(result.is_ok());
        let partitions = result.unwrap();
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0], "2024-01-15");
    }

    #[test]
    fn test_generate_date_range_multiple_days() {
        let result = generate_date_range_partitions("2024-01-15", "2024-01-18");
        assert!(result.is_ok());
        let partitions = result.unwrap();
        assert_eq!(partitions.len(), 4);
        assert_eq!(partitions[0], "2024-01-15");
        assert_eq!(partitions[1], "2024-01-16");
        assert_eq!(partitions[2], "2024-01-17");
        assert_eq!(partitions[3], "2024-01-18");
    }

    #[test]
    fn test_generate_date_range_month_boundary() {
        let result = generate_date_range_partitions("2024-01-30", "2024-02-02");
        assert!(result.is_ok());
        let partitions = result.unwrap();
        assert_eq!(partitions.len(), 4);
        assert_eq!(partitions[0], "2024-01-30");
        assert_eq!(partitions[1], "2024-01-31");
        assert_eq!(partitions[2], "2024-02-01");
        assert_eq!(partitions[3], "2024-02-02");
    }

    #[test]
    fn test_generate_date_range_leap_year() {
        let result = generate_date_range_partitions("2024-02-28", "2024-03-01");
        assert!(result.is_ok());
        let partitions = result.unwrap();
        assert_eq!(partitions.len(), 3);
        assert_eq!(partitions[0], "2024-02-28");
        assert_eq!(partitions[1], "2024-02-29"); // Leap year
        assert_eq!(partitions[2], "2024-03-01");
    }

    #[test]
    fn test_generate_date_range_end_before_start() {
        let result = generate_date_range_partitions("2024-01-15", "2024-01-10");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("before start"));
    }

    #[test]
    fn test_generate_date_range_invalid_start_format() {
        let result = generate_date_range_partitions("2024/01/15", "2024-01-20");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid start date"));
    }

    #[test]
    fn test_generate_date_range_invalid_end_format() {
        let result = generate_date_range_partitions("2024-01-15", "01-20-2024");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid end date"));
    }

    #[test]
    fn test_generate_date_range_exceeds_maximum() {
        // More than 366 days should fail
        let result = generate_date_range_partitions("2024-01-01", "2025-01-02");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum"));
    }

    #[test]
    fn test_generate_date_range_at_maximum() {
        // Exactly 366 days should succeed
        let result = generate_date_range_partitions("2024-01-01", "2024-12-31");
        assert!(result.is_ok());
        let partitions = result.unwrap();
        assert_eq!(partitions.len(), 366); // 2024 is a leap year
    }
}
