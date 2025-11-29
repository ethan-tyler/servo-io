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
