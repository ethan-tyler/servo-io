//! Database models for Servo metadata

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Asset model for database storage
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AssetModel {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub asset_type: String,
    pub owner: Option<String>,
    pub tags: sqlx::types::Json<Vec<String>>,
    pub tenant_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Workflow model for database storage
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct WorkflowModel {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub tags: sqlx::types::Json<Vec<String>>,
    pub tenant_id: Option<String>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Execution model for database storage
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ExecutionModel {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub state: String,
    pub tenant_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Asset dependency model for lineage tracking
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AssetDependencyModel {
    pub id: Uuid,
    pub upstream_asset_id: Uuid,
    pub downstream_asset_id: Uuid,
    pub dependency_type: String,
    pub created_at: DateTime<Utc>,
}

/// Asset check model for data quality checks
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AssetCheckModel {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub asset_id: Uuid,
    pub check_type: sqlx::types::Json<serde_json::Value>,
    pub severity: String,
    pub blocking: bool,
    pub enabled: bool,
    pub tags: sqlx::types::Json<Vec<String>>,
    pub owner: Option<String>,
    pub tenant_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Check result model for storing check execution outcomes
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CheckResultModel {
    pub id: Uuid,
    pub check_id: Uuid,
    pub execution_id: Uuid,
    pub asset_id: Uuid,
    pub outcome: String,
    pub severity: String,
    pub blocking: bool,
    pub failed_row_count: Option<i64>,
    pub total_row_count: Option<i64>,
    pub error_message: Option<String>,
    pub failed_samples: Option<sqlx::types::Json<serde_json::Value>>,
    pub duration_ms: i64,
    pub executed_at: DateTime<Utc>,
    pub metadata: Option<sqlx::types::Json<serde_json::Value>>,
    pub tenant_id: Option<String>,
}

/// Backfill job model for managing partition backfill operations
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BackfillJobModel {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub asset_name: String,
    pub idempotency_key: String,
    pub state: String,
    pub execution_strategy: sqlx::types::Json<serde_json::Value>,
    pub partition_start: Option<String>,
    pub partition_end: Option<String>,
    pub partition_keys: sqlx::types::Json<Vec<String>>,
    pub total_partitions: i32,
    pub completed_partitions: i32,
    pub failed_partitions: i32,
    pub skipped_partitions: i32,
    pub include_upstream: bool,
    pub error_message: Option<String>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub version: i32,
    /// Timestamp when the job was paused (for pause/resume)
    pub paused_at: Option<DateTime<Utc>>,
    /// Last successfully processed partition key (for resumption)
    pub checkpoint_partition_key: Option<String>,
    /// Estimated completion time based on EWMA
    pub estimated_completion_at: Option<DateTime<Utc>>,
    /// Average partition duration in milliseconds (EWMA)
    pub avg_partition_duration_ms: Option<i64>,
    /// Parent job ID for upstream propagation hierarchy (None for root jobs)
    pub parent_job_id: Option<Uuid>,
    /// Maximum depth for upstream discovery (0 = direct dependencies only)
    pub max_upstream_depth: i32,
    /// Total number of upstream child jobs created
    pub upstream_job_count: i32,
    /// Number of upstream jobs that have completed successfully
    pub completed_upstream_jobs: i32,
    /// Topological execution order (0 = furthest upstream, execute first)
    pub execution_order: i32,
    /// Optional deadline for SLA tracking; job should complete before this time
    pub sla_deadline_at: Option<DateTime<Utc>>,
    /// Job priority for scheduling (-10 to 10, higher = more urgent, default 0)
    pub priority: i32,
}

/// Summary of upstream job status for a parent job
#[derive(Debug, Clone, Default)]
pub struct UpstreamJobsSummary {
    /// Total number of upstream jobs
    pub total: i32,
    /// Number of completed upstream jobs
    pub completed: i32,
    /// Number of failed upstream jobs
    pub failed: i32,
    /// Number of running upstream jobs
    pub running: i32,
    /// Number of pending/waiting upstream jobs
    pub pending: i32,
}

/// Parameters for creating an upstream child backfill job
#[derive(Debug, Clone)]
pub struct CreateUpstreamChildJobParams {
    /// Parent job ID that this child belongs to
    pub parent_job_id: Uuid,
    /// Asset ID to backfill
    pub asset_id: Uuid,
    /// Asset name for display
    pub asset_name: String,
    /// Idempotency key for deduplication
    pub idempotency_key: String,
    /// Optional start partition key
    pub partition_start: Option<String>,
    /// Optional end partition key
    pub partition_end: Option<String>,
    /// Execution order in topological sort (0 = execute first)
    pub execution_order: i32,
}

/// Backfill partition model for tracking individual partition execution
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BackfillPartitionModel {
    pub id: Uuid,
    pub backfill_job_id: Uuid,
    pub partition_key: String,
    pub state: String,
    pub attempt_count: i32,
    pub execution_id: Option<Uuid>,
    pub error_message: Option<String>,
    pub duration_ms: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub tenant_id: String,
}

/// Parameters for updating backfill job progress with ETA information
///
/// This struct groups related parameters for atomic progress updates,
/// addressing the function argument count clippy warning while improving
/// API clarity.
#[derive(Debug, Clone, Default)]
pub struct BackfillProgressUpdate {
    /// Number of completed partitions
    pub completed: i32,
    /// Number of failed partitions
    pub failed: i32,
    /// Number of skipped partitions
    pub skipped: i32,
    /// Average partition duration in milliseconds (EWMA)
    pub avg_partition_duration_ms: Option<i64>,
    /// Estimated completion time
    pub estimated_completion_at: Option<DateTime<Utc>>,
    /// Last successfully completed partition key (checkpoint for resumption)
    pub checkpoint_partition_key: Option<String>,
}
