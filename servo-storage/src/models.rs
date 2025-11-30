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
