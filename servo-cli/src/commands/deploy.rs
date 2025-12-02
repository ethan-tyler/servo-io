//! Deploy command for Servo CLI
//!
//! Deploys workflow definitions to the Servo platform by:
//! 1. Parsing the workflow definition file (YAML, TOML, or JSON)
//! 2. Creating the workflow in the database
//! 3. Creating assets and their dependencies
//!
//! # Workflow Definition Format
//!
//! Workflows can be defined in YAML, TOML, or JSON. Example YAML:
//!
//! ```yaml
//! name: daily_etl
//! description: Daily ETL workflow for sales data
//! owner: data_team
//! tags:
//!   - etl
//!   - daily
//!
//! assets:
//!   - name: raw_sales
//!     type: table
//!     description: Raw sales data from source systems
//!     partition:
//!       type: daily
//!       key: date
//!
//!   - name: transformed_sales
//!     type: table
//!     description: Cleaned and transformed sales data
//!     dependencies:
//!       - asset: raw_sales
//!         type: data
//!     partition:
//!       type: daily
//!       key: date
//!
//!   - name: sales_report
//!     type: report
//!     description: Daily sales summary report
//!     dependencies:
//!       - asset: transformed_sales
//!         type: data
//! ```

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use servo_storage::{models::AssetModel, models::WorkflowModel, PostgresStorage, TenantId};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// Schedule definition for workflow execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleDefinition {
    /// Cron expression (e.g., "0 2 * * *" for daily at 2 AM)
    pub cron: String,

    /// Timezone for cron schedule interpretation (default: UTC)
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// Whether the schedule is enabled (default: true)
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

/// Workflow definition file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Workflow name (required)
    pub name: String,

    /// Workflow description (optional)
    pub description: Option<String>,

    /// Owner of the workflow
    pub owner: Option<String>,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Optional schedule for automated execution
    pub schedule: Option<ScheduleDefinition>,

    /// Assets in this workflow
    #[serde(default)]
    pub assets: Vec<AssetDefinition>,
}

/// Asset definition within a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDefinition {
    /// Asset name (required)
    pub name: String,

    /// Asset type (table, file, model, report, etc.)
    #[serde(rename = "type")]
    pub asset_type: String,

    /// Description of the asset
    pub description: Option<String>,

    /// Owner of the asset
    pub owner: Option<String>,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Dependencies on other assets (by name)
    #[serde(default)]
    pub dependencies: Vec<DependencyDefinition>,

    /// Partition configuration
    pub partition: Option<PartitionDefinition>,

    /// Python module containing the compute function (e.g., "myproject.workflows")
    #[serde(rename = "module")]
    pub compute_fn_module: Option<String>,

    /// Python function name to invoke for this asset (e.g., "clean_customers")
    #[serde(rename = "function")]
    pub compute_fn_function: Option<String>,
}

/// Dependency definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyDefinition {
    /// Name of the upstream asset
    pub asset: String,

    /// Type of dependency (data, metadata, control)
    #[serde(rename = "type", default = "default_dependency_type")]
    pub dependency_type: String,
}

fn default_dependency_type() -> String {
    "data".to_string()
}

/// Partition definition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PartitionDefinition {
    /// Type of partitioning (daily, hourly, weekly, monthly, yearly, custom, range)
    #[serde(rename = "type")]
    pub partition_type: String,

    /// Partition key column/field name
    pub key: String,

    /// For custom partitions: the expression
    pub expression: Option<String>,

    /// For range partitions: lower bound
    pub lower_bound: Option<String>,

    /// For range partitions: upper bound
    pub upper_bound: Option<String>,
}

impl PartitionDefinition {
    /// Convert to servo-core PartitionConfig JSON format
    fn to_partition_config_json(&self) -> serde_json::Value {
        let partition_type = match self.partition_type.to_lowercase().as_str() {
            "daily" => serde_json::json!({
                "Time": { "granularity": "Daily" }
            }),
            "hourly" => serde_json::json!({
                "Time": { "granularity": "Hourly" }
            }),
            "weekly" => serde_json::json!({
                "Time": { "granularity": "Weekly" }
            }),
            "monthly" => serde_json::json!({
                "Time": { "granularity": "Monthly" }
            }),
            "yearly" => serde_json::json!({
                "Time": { "granularity": "Yearly" }
            }),
            "custom" => serde_json::json!({
                "Custom": { "expression": self.expression.clone().unwrap_or_default() }
            }),
            "range" => serde_json::json!({
                "Range": {
                    "lower_bound": self.lower_bound.clone().unwrap_or_default(),
                    "upper_bound": self.upper_bound.clone().unwrap_or_default()
                }
            }),
            _ => serde_json::json!({
                "Time": { "granularity": "Daily" }
            }),
        };

        serde_json::json!({
            "partition_type": partition_type,
            "partition_key": self.key
        })
    }
}

/// Result of a workflow deployment
#[derive(Debug)]
pub struct DeploymentResult {
    /// Workflow ID
    pub workflow_id: Uuid,

    /// Asset IDs created (asset name -> asset ID)
    pub asset_ids: HashMap<String, Uuid>,

    /// Number of dependencies created
    pub dependency_count: usize,

    /// Whether this was an update or new deployment
    pub was_update: bool,
}

/// Execute the deploy command
pub async fn execute(workflow_file: &str) -> Result<()> {
    tracing::info!("Deploying workflow from {}", workflow_file);

    // Load configuration
    let config = crate::config::Config::load()?;
    let database_url = config.database_url.clone();

    // Resolve tenant ID
    let tenant_id = config
        .tenant_id
        .clone()
        .or_else(|| std::env::var("TENANT_ID").ok())
        .unwrap_or_else(|| "default".to_string());

    let tenant = TenantId::new(&tenant_id);

    // Connect to database
    let storage = Arc::new(PostgresStorage::new(&database_url).await.context(
        "Failed to connect to database. Check DATABASE_URL environment variable or servo.toml",
    )?);

    // Parse the workflow definition to check for schedule
    let definition = parse_workflow_file(workflow_file)?;
    let has_schedule = definition.schedule.is_some();

    // Check for existing workflow to detect schedule removal
    let existing_workflow =
        find_workflow_by_name(storage.as_ref(), &tenant, &definition.name).await?;
    let had_schedule = existing_workflow
        .as_ref()
        .map(|w| w.schedule_cron.is_some() && w.scheduler_job_name.is_some())
        .unwrap_or(false);

    // Deploy the workflow
    let result = deploy_workflow(workflow_file, storage.as_ref(), &tenant).await?;

    // Handle schedule lifecycle
    if has_schedule {
        // Create or update the Cloud Scheduler job
        if let Err(e) = sync_scheduler(
            &config,
            &storage,
            &tenant,
            &definition,
            result.workflow_id,
            &tenant_id,
        )
        .await
        {
            tracing::warn!(
                error = %e,
                "Failed to sync Cloud Scheduler. Disabling schedule to prevent drift."
            );

            // Disable the schedule in the database to prevent drift
            // (schedule metadata is preserved but marked disabled)
            if let Err(disable_err) =
                disable_schedule_on_sync_failure(&storage, &tenant, result.workflow_id).await
            {
                tracing::error!(
                    error = %disable_err,
                    "Failed to disable schedule after sync failure"
                );
            }

            println!(
                "Warning: Workflow deployed but Cloud Scheduler sync failed: {}",
                e
            );
            println!("Schedule has been disabled. To enable:");
            println!("  1. Configure GCP settings (project_id, region, worker_url, service_account_file)");
            println!("  2. Re-deploy the workflow");
        }
    } else if had_schedule {
        // Schedule was removed - delete the Cloud Scheduler job
        tracing::info!(
            workflow_id = %result.workflow_id,
            "Schedule removed from workflow, deleting Cloud Scheduler job"
        );

        if let Err(e) = delete_scheduler_job(&config, result.workflow_id, &tenant_id).await {
            tracing::warn!(
                error = %e,
                "Failed to delete Cloud Scheduler job. Manual cleanup may be required."
            );
            println!(
                "Warning: Schedule removed but failed to delete Cloud Scheduler job: {}",
                e
            );
        } else {
            // Clear scheduler metadata from workflow
            if let Err(e) = clear_schedule_metadata(&storage, &tenant, result.workflow_id).await {
                tracing::warn!(
                    error = %e,
                    "Failed to clear schedule metadata from workflow"
                );
            }
            println!("Cloud Scheduler job deleted (schedule removed from workflow)");
        }
    }

    // Print deployment summary
    print_deployment_summary(&result);

    Ok(())
}

/// Sync workflow schedule with Cloud Scheduler
async fn sync_scheduler(
    config: &crate::config::Config,
    storage: &Arc<PostgresStorage>,
    tenant: &TenantId,
    definition: &WorkflowDefinition,
    workflow_id: Uuid,
    tenant_id: &str,
) -> Result<()> {
    use servo_cloud_gcp::{CloudSchedulerManager, ScheduleConfig};

    // Validate GCP configuration
    let project_id = config
        .project_id
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project_id required for Cloud Scheduler"))?;
    let region = config
        .region
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("region required for Cloud Scheduler"))?;
    let worker_url = config
        .worker_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("worker_url required for Cloud Scheduler"))?;
    let service_account_file = config
        .service_account_file
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("service_account_file required for Cloud Scheduler"))?;

    // Load service account credentials
    let service_account_json = std::fs::read_to_string(service_account_file)
        .context("Failed to read service account file")?;

    // Create scheduler manager
    let scheduler = CloudSchedulerManager::new(
        project_id.clone(),
        region.clone(),
        worker_url.clone(),
        &service_account_json,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Cloud Scheduler client: {}", e))?;

    // Get schedule from definition
    let schedule_def = definition
        .schedule
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No schedule defined"))?;

    let schedule_config = ScheduleConfig {
        cron: schedule_def.cron.clone(),
        timezone: schedule_def.timezone.clone(),
        enabled: schedule_def.enabled.unwrap_or(true),
    };

    // Create or update the scheduler job
    let job_name = scheduler
        .create_or_update_job(workflow_id, &definition.name, tenant_id, &schedule_config)
        .await
        .map_err(|e| anyhow::anyhow!("Cloud Scheduler API error: {}", e))?;

    // Update workflow with the job name
    let mut workflow = storage
        .get_workflow(workflow_id, tenant)
        .await
        .context("Failed to get workflow for job name update")?;
    workflow.scheduler_job_name = Some(job_name.clone());
    workflow.updated_at = Utc::now();
    storage
        .update_workflow(&workflow, tenant)
        .await
        .context("Failed to update workflow scheduler_job_name")?;

    tracing::info!(
        job_name = %job_name,
        workflow_id = %workflow_id,
        cron = %schedule_def.cron,
        "Cloud Scheduler job synced"
    );

    println!("Cloud Scheduler job created/updated: {}", job_name);

    Ok(())
}

/// Delete Cloud Scheduler job when schedule is removed from workflow
async fn delete_scheduler_job(
    config: &crate::config::Config,
    workflow_id: Uuid,
    tenant_id: &str,
) -> Result<()> {
    use servo_cloud_gcp::CloudSchedulerManager;

    // Validate GCP configuration
    let project_id = config
        .project_id
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("project_id required for Cloud Scheduler"))?;
    let region = config
        .region
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("region required for Cloud Scheduler"))?;
    let worker_url = config
        .worker_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("worker_url required for Cloud Scheduler"))?;
    let service_account_file = config
        .service_account_file
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("service_account_file required for Cloud Scheduler"))?;

    // Load service account credentials
    let service_account_json = std::fs::read_to_string(service_account_file)
        .context("Failed to read service account file")?;

    // Create scheduler manager
    let scheduler = CloudSchedulerManager::new(
        project_id.clone(),
        region.clone(),
        worker_url.clone(),
        &service_account_json,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Cloud Scheduler client: {}", e))?;

    // Delete the job
    scheduler
        .delete_job(workflow_id, tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!("Cloud Scheduler delete failed: {}", e))?;

    tracing::info!(
        workflow_id = %workflow_id,
        tenant_id = %tenant_id,
        "Cloud Scheduler job deleted"
    );

    Ok(())
}

/// Clear schedule metadata from workflow after scheduler job deletion
async fn clear_schedule_metadata(
    storage: &Arc<PostgresStorage>,
    tenant: &TenantId,
    workflow_id: Uuid,
) -> Result<()> {
    let mut workflow = storage
        .get_workflow(workflow_id, tenant)
        .await
        .context("Failed to get workflow for schedule metadata clear")?;

    workflow.schedule_cron = None;
    workflow.schedule_timezone = None;
    workflow.schedule_enabled = None;
    workflow.scheduler_job_name = None;
    workflow.last_scheduled_run = None;
    workflow.next_scheduled_run = None;
    workflow.updated_at = Utc::now();

    storage
        .update_workflow(&workflow, tenant)
        .await
        .context("Failed to clear workflow schedule metadata")?;

    tracing::info!(
        workflow_id = %workflow_id,
        "Schedule metadata cleared from workflow"
    );

    Ok(())
}

/// Disable schedule when Cloud Scheduler sync fails
///
/// This prevents drift where the database says schedule_enabled=true
/// but no actual Cloud Scheduler job exists.
async fn disable_schedule_on_sync_failure(
    storage: &Arc<PostgresStorage>,
    tenant: &TenantId,
    workflow_id: Uuid,
) -> Result<()> {
    // Get the current workflow
    let mut workflow = storage
        .get_workflow(workflow_id, tenant)
        .await
        .context("Failed to get workflow for schedule disable")?;

    // Mark schedule as disabled
    workflow.schedule_enabled = Some(false);
    workflow.updated_at = Utc::now();

    // Update in database
    storage
        .update_workflow(&workflow, tenant)
        .await
        .context("Failed to update workflow schedule_enabled")?;

    tracing::info!(
        workflow_id = %workflow_id,
        "Schedule disabled due to Cloud Scheduler sync failure"
    );

    Ok(())
}

/// Deploy a workflow from a definition file
pub async fn deploy_workflow(
    workflow_file: &str,
    storage: &PostgresStorage,
    tenant: &TenantId,
) -> Result<DeploymentResult> {
    // Parse the workflow definition
    let definition = parse_workflow_file(workflow_file)?;

    // Check if workflow already exists by listing and filtering
    let existing_workflow = find_workflow_by_name(storage, tenant, &definition.name).await?;

    let (workflow_id, was_update) = if let Some(existing) = existing_workflow {
        tracing::info!(
            workflow_id = %existing.id,
            workflow_name = %definition.name,
            "Updating existing workflow"
        );
        (existing.id, true)
    } else {
        let workflow_id = Uuid::new_v4();
        tracing::info!(
            workflow_id = %workflow_id,
            workflow_name = %definition.name,
            "Creating new workflow"
        );
        (workflow_id, false)
    };

    let now = Utc::now();

    // Create or update workflow
    let workflow = WorkflowModel {
        id: workflow_id,
        name: definition.name.clone(),
        description: definition.description.clone(),
        owner: definition.owner.clone(),
        tags: sqlx::types::Json(definition.tags.clone()),
        tenant_id: Some(tenant.as_str().to_string()),
        version: if was_update { 0 } else { 1 }, // Version will be incremented on update
        schedule_cron: definition.schedule.as_ref().map(|s| s.cron.clone()),
        schedule_timezone: definition.schedule.as_ref().map(|s| s.timezone.clone()),
        schedule_enabled: definition
            .schedule
            .as_ref()
            .map(|s| s.enabled.unwrap_or(true)),
        scheduler_job_name: None, // Will be set when Cloud Scheduler job is created
        last_scheduled_run: None,
        next_scheduled_run: None,
        created_at: now,
        updated_at: now,
    };

    if was_update {
        storage
            .update_workflow(&workflow, tenant)
            .await
            .context("Failed to update workflow")?;
    } else {
        storage
            .create_workflow(&workflow, tenant)
            .await
            .context("Failed to create workflow")?;
    }

    // Create assets and track their IDs for dependency resolution
    let mut asset_ids: HashMap<String, Uuid> = HashMap::new();

    // First pass: create all assets
    for asset_def in &definition.assets {
        let asset_id =
            create_or_update_asset(storage, tenant, asset_def, &definition.owner).await?;
        asset_ids.insert(asset_def.name.clone(), asset_id);
    }

    // Second pass: create dependencies
    let mut dependency_count = 0;
    for asset_def in &definition.assets {
        let downstream_id = asset_ids
            .get(&asset_def.name)
            .copied()
            .expect("Asset ID should exist");

        for dep in &asset_def.dependencies {
            let upstream_id = asset_ids.get(&dep.asset).ok_or_else(|| {
                anyhow::anyhow!(
                    "Asset '{}' depends on '{}' which is not defined in this workflow",
                    asset_def.name,
                    dep.asset
                )
            })?;

            // Validate dependency type
            let dep_type = match dep.dependency_type.as_str() {
                "data" => "data",
                "metadata" => "metadata",
                "control" => "control",
                other => {
                    anyhow::bail!(
                        "Invalid dependency type '{}'. Must be one of: data, metadata, control",
                        other
                    )
                }
            };

            // Create the dependency (idempotent - ignores duplicates)
            match storage
                .create_asset_dependency(*upstream_id, downstream_id, dep_type, tenant)
                .await
            {
                Ok(_) => {
                    dependency_count += 1;
                    tracing::debug!(
                        upstream = %dep.asset,
                        downstream = %asset_def.name,
                        dep_type = dep_type,
                        "Created dependency"
                    );
                }
                Err(e) => {
                    // Check if it's a duplicate constraint violation (which is OK)
                    if e.to_string().contains("duplicate key")
                        || e.to_string().contains("already exists")
                    {
                        tracing::debug!(
                            upstream = %dep.asset,
                            downstream = %asset_def.name,
                            "Dependency already exists"
                        );
                    } else {
                        return Err(e).context(format!(
                            "Failed to create dependency {} -> {}",
                            dep.asset, asset_def.name
                        ));
                    }
                }
            }
        }
    }

    Ok(DeploymentResult {
        workflow_id,
        asset_ids,
        dependency_count,
        was_update,
    })
}

/// Create or update an asset in the database
async fn create_or_update_asset(
    storage: &PostgresStorage,
    tenant: &TenantId,
    asset_def: &AssetDefinition,
    workflow_owner: &Option<String>,
) -> Result<Uuid> {
    let now = Utc::now();

    // Check if asset already exists by listing and filtering
    let existing_asset = find_asset_by_name(storage, tenant, &asset_def.name).await?;

    let asset_id = if let Some(existing) = &existing_asset {
        existing.id
    } else {
        Uuid::new_v4()
    };

    // Use asset owner if specified, otherwise fall back to workflow owner
    let owner = asset_def.owner.clone().or_else(|| workflow_owner.clone());

    // Convert partition definition to PartitionConfig JSON if present
    let partition_config = asset_def
        .partition
        .as_ref()
        .map(|p| sqlx::types::Json(p.to_partition_config_json()));

    let asset = AssetModel {
        id: asset_id,
        name: asset_def.name.clone(),
        description: asset_def.description.clone(),
        asset_type: asset_def.asset_type.clone(),
        owner,
        tags: sqlx::types::Json(asset_def.tags.clone()),
        partition_config,
        compute_fn_module: asset_def.compute_fn_module.clone(),
        compute_fn_function: asset_def.compute_fn_function.clone(),
        tenant_id: Some(tenant.as_str().to_string()),
        created_at: existing_asset.as_ref().map(|a| a.created_at).unwrap_or(now),
        updated_at: now,
    };

    if existing_asset.is_some() {
        storage
            .update_asset(&asset, tenant)
            .await
            .context(format!("Failed to update asset '{}'", asset_def.name))?;
        tracing::debug!(asset_id = %asset_id, asset_name = %asset_def.name, "Updated existing asset");
    } else {
        storage
            .create_asset(&asset, tenant)
            .await
            .context(format!("Failed to create asset '{}'", asset_def.name))?;
        tracing::debug!(asset_id = %asset_id, asset_name = %asset_def.name, "Created new asset");
    }

    Ok(asset_id)
}

/// Find a workflow by name
async fn find_workflow_by_name(
    storage: &PostgresStorage,
    tenant: &TenantId,
    name: &str,
) -> Result<Option<WorkflowModel>> {
    // List workflows and filter by name (not efficient for large datasets,
    // but sufficient for typical workflow counts)
    let workflows = storage.list_workflows(tenant, 1000, 0).await?;
    Ok(workflows.into_iter().find(|w| w.name == name))
}

/// Find an asset by name
async fn find_asset_by_name(
    storage: &PostgresStorage,
    tenant: &TenantId,
    name: &str,
) -> Result<Option<AssetModel>> {
    // List assets and filter by name
    let assets = storage.list_assets(tenant, 10000, 0).await?;
    Ok(assets.into_iter().find(|a| a.name == name))
}

/// Parse a workflow definition file
fn parse_workflow_file(path: &str) -> Result<WorkflowDefinition> {
    let path = Path::new(path);

    if !path.exists() {
        anyhow::bail!("Workflow file not found: {}", path.display());
    }

    let content = std::fs::read_to_string(path)
        .context(format!("Failed to read file: {}", path.display()))?;

    // Determine format from extension
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let definition: WorkflowDefinition = match extension.as_str() {
        "yaml" | "yml" => {
            serde_yaml::from_str(&content).context("Failed to parse YAML workflow definition")?
        }
        "toml" => toml::from_str(&content).context("Failed to parse TOML workflow definition")?,
        "json" => {
            serde_json::from_str(&content).context("Failed to parse JSON workflow definition")?
        }
        _ => {
            // Try to auto-detect format
            if content.trim().starts_with('{') {
                serde_json::from_str(&content).context("Failed to parse as JSON")?
            } else if content.contains(':') && !content.contains('=') {
                serde_yaml::from_str(&content).context("Failed to parse as YAML")?
            } else {
                toml::from_str(&content).context("Failed to parse as TOML")?
            }
        }
    };

    validate_workflow_definition(&definition)?;

    Ok(definition)
}

/// Validate schedule definition (cron expression and timezone)
fn validate_schedule(schedule: &ScheduleDefinition) -> Result<()> {
    use cron::Schedule;
    use std::str::FromStr;

    // Validate cron expression
    // Cloud Scheduler uses standard 5-field cron (minute hour day month weekday)
    // The cron crate expects 6 or 7 fields, so we prepend seconds
    let cron_with_seconds = format!("0 {}", schedule.cron);
    if Schedule::from_str(&cron_with_seconds).is_err() {
        anyhow::bail!(
            "Invalid cron expression '{}'. Expected format: 'minute hour day month weekday' (e.g., '0 2 * * *' for daily at 2 AM)",
            schedule.cron
        );
    }

    // Validate timezone
    if schedule.timezone.parse::<chrono_tz::Tz>().is_err() {
        anyhow::bail!(
            "Invalid timezone '{}'. Use IANA timezone format (e.g., 'America/New_York', 'UTC', 'Europe/London')",
            schedule.timezone
        );
    }

    Ok(())
}

/// Validate workflow definition
fn validate_workflow_definition(def: &WorkflowDefinition) -> Result<()> {
    if def.name.is_empty() {
        anyhow::bail!("Workflow name cannot be empty");
    }

    // Validate schedule if present
    if let Some(schedule) = &def.schedule {
        validate_schedule(schedule)?;
    }

    // Check for duplicate asset names
    let mut asset_names = std::collections::HashSet::new();
    for asset in &def.assets {
        if asset.name.is_empty() {
            anyhow::bail!("Asset name cannot be empty");
        }
        if !asset_names.insert(&asset.name) {
            anyhow::bail!("Duplicate asset name: '{}'", asset.name);
        }
        if asset.asset_type.is_empty() {
            anyhow::bail!("Asset '{}' must have a type", asset.name);
        }
    }

    // Validate all dependency references exist
    for asset in &def.assets {
        for dep in &asset.dependencies {
            if !asset_names.contains(&dep.asset) {
                anyhow::bail!(
                    "Asset '{}' depends on '{}' which is not defined in this workflow",
                    asset.name,
                    dep.asset
                );
            }
        }
    }

    // Check for self-dependencies
    for asset in &def.assets {
        for dep in &asset.dependencies {
            if dep.asset == asset.name {
                anyhow::bail!("Asset '{}' cannot depend on itself", asset.name);
            }
        }
    }

    Ok(())
}

/// Print deployment summary to console
fn print_deployment_summary(result: &DeploymentResult) {
    let action = if result.was_update {
        "Updated"
    } else {
        "Created"
    };

    println!();
    println!("{} workflow successfully!", action);
    println!("  Workflow ID: {}", result.workflow_id);
    println!("  Assets: {}", result.asset_ids.len());
    println!("  Dependencies: {}", result.dependency_count);
    println!();

    if !result.asset_ids.is_empty() {
        println!("Assets:");
        for (name, id) in &result.asset_ids {
            println!("  - {} ({})", name, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_workflow() {
        let yaml = r#"
name: test_workflow
description: A test workflow
owner: test_user
tags:
  - test
  - example

assets:
  - name: source_data
    type: table
    description: Source data table

  - name: processed_data
    type: table
    dependencies:
      - asset: source_data
        type: data
"#;
        let def: WorkflowDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "test_workflow");
        assert_eq!(def.assets.len(), 2);
        assert_eq!(def.assets[1].dependencies.len(), 1);
        assert_eq!(def.assets[1].dependencies[0].asset, "source_data");
    }

    #[test]
    fn test_parse_toml_workflow() {
        let toml_content = r#"
name = "test_workflow"
description = "A test workflow"
owner = "test_user"
tags = ["test", "example"]

[[assets]]
name = "source_data"
type = "table"
description = "Source data table"

[[assets]]
name = "processed_data"
type = "table"

[[assets.dependencies]]
asset = "source_data"
type = "data"
"#;
        let def: WorkflowDefinition = toml::from_str(toml_content).unwrap();
        assert_eq!(def.name, "test_workflow");
        assert_eq!(def.assets.len(), 2);
    }

    #[test]
    fn test_validation_empty_name() {
        let def = WorkflowDefinition {
            name: "".to_string(),
            description: None,
            owner: None,
            tags: vec![],
            schedule: None,
            assets: vec![],
        };
        assert!(validate_workflow_definition(&def).is_err());
    }

    #[test]
    fn test_validation_duplicate_assets() {
        let def = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            owner: None,
            tags: vec![],
            schedule: None,
            assets: vec![
                AssetDefinition {
                    name: "duplicate".to_string(),
                    asset_type: "table".to_string(),
                    description: None,
                    owner: None,
                    tags: vec![],
                    dependencies: vec![],
                    partition: None,
                    compute_fn_module: None,
                    compute_fn_function: None,
                },
                AssetDefinition {
                    name: "duplicate".to_string(),
                    asset_type: "table".to_string(),
                    description: None,
                    owner: None,
                    tags: vec![],
                    dependencies: vec![],
                    partition: None,
                    compute_fn_module: None,
                    compute_fn_function: None,
                },
            ],
        };
        let result = validate_workflow_definition(&def);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate"));
    }

    #[test]
    fn test_validation_self_dependency() {
        let def = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            owner: None,
            tags: vec![],
            schedule: None,
            assets: vec![AssetDefinition {
                name: "self_ref".to_string(),
                asset_type: "table".to_string(),
                description: None,
                owner: None,
                tags: vec![],
                dependencies: vec![DependencyDefinition {
                    asset: "self_ref".to_string(),
                    dependency_type: "data".to_string(),
                }],
                partition: None,
                compute_fn_module: None,
                compute_fn_function: None,
            }],
        };
        let result = validate_workflow_definition(&def);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cannot depend on itself"));
    }

    #[test]
    fn test_validation_undefined_dependency() {
        let def = WorkflowDefinition {
            name: "test".to_string(),
            description: None,
            owner: None,
            tags: vec![],
            schedule: None,
            assets: vec![AssetDefinition {
                name: "downstream".to_string(),
                asset_type: "table".to_string(),
                description: None,
                owner: None,
                tags: vec![],
                dependencies: vec![DependencyDefinition {
                    asset: "nonexistent".to_string(),
                    dependency_type: "data".to_string(),
                }],
                partition: None,
                compute_fn_module: None,
                compute_fn_function: None,
            }],
        };
        let result = validate_workflow_definition(&def);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not defined"));
    }

    #[test]
    fn test_default_dependency_type() {
        let yaml = r#"
name: test
assets:
  - name: a
    type: table
  - name: b
    type: table
    dependencies:
      - asset: a
"#;
        let def: WorkflowDefinition = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.assets[1].dependencies[0].dependency_type, "data");
    }

    #[test]
    fn test_schedule_validation_valid() {
        let schedule = ScheduleDefinition {
            cron: "0 2 * * *".to_string(),
            timezone: "America/New_York".to_string(),
            enabled: Some(true),
        };
        assert!(validate_schedule(&schedule).is_ok());
    }

    #[test]
    fn test_schedule_validation_valid_utc() {
        let schedule = ScheduleDefinition {
            cron: "*/5 * * * *".to_string(),
            timezone: "UTC".to_string(),
            enabled: None,
        };
        assert!(validate_schedule(&schedule).is_ok());
    }

    #[test]
    fn test_schedule_validation_invalid_cron() {
        let schedule = ScheduleDefinition {
            cron: "invalid cron".to_string(),
            timezone: "UTC".to_string(),
            enabled: Some(true),
        };
        let result = validate_schedule(&schedule);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid cron"));
    }

    #[test]
    fn test_schedule_validation_invalid_timezone() {
        let schedule = ScheduleDefinition {
            cron: "0 2 * * *".to_string(),
            timezone: "Invalid/Timezone".to_string(),
            enabled: Some(true),
        };
        let result = validate_schedule(&schedule);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid timezone"));
    }

    #[test]
    fn test_workflow_with_schedule_validation() {
        let yaml = r#"
name: scheduled_workflow
schedule:
  cron: "0 2 * * *"
  timezone: "Europe/London"
  enabled: true
assets:
  - name: data
    type: table
"#;
        let def: WorkflowDefinition = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_workflow_definition(&def).is_ok());
        assert!(def.schedule.is_some());
        let schedule = def.schedule.unwrap();
        assert_eq!(schedule.cron, "0 2 * * *");
        assert_eq!(schedule.timezone, "Europe/London");
    }
}
