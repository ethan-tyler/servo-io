//! PostgreSQL storage implementation

use crate::{models::*, Result, TenantId};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

/// PostgreSQL storage backend
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Create a new PostgreSQL storage instance
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Create a new asset
    pub async fn create_asset(
        &self,
        asset: &AssetModel,
        tenant_id: Option<&TenantId>,
    ) -> Result<()> {
        let tenant_str = tenant_id.map(|t| t.0.as_str());

        sqlx::query!(
            r#"
            INSERT INTO assets (
                id, name, description, asset_type, owner, tags, tenant_id, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            asset.id,
            asset.name,
            asset.description,
            asset.asset_type,
            asset.owner,
            asset.tags as _,
            tenant_str,
            asset.created_at,
            asset.updated_at
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get an asset by ID
    pub async fn get_asset(
        &self,
        id: Uuid,
        tenant_id: Option<&TenantId>,
    ) -> Result<AssetModel> {
        let tenant_str = tenant_id.map(|t| t.0.as_str());

        let asset = sqlx::query_as!(
            AssetModel,
            r#"
            SELECT id, name, description, asset_type, owner,
                   tags as "tags: sqlx::types::Json<Vec<String>>",
                   tenant_id, created_at, updated_at
            FROM assets
            WHERE id = $1 AND (tenant_id = $2 OR $2 IS NULL)
            "#,
            id,
            tenant_str
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| crate::Error::NotFound(format!("Asset {}", id)))?;

        Ok(asset)
    }

    /// Create a new workflow
    pub async fn create_workflow(
        &self,
        workflow: &WorkflowModel,
        tenant_id: Option<&TenantId>,
    ) -> Result<()> {
        let tenant_str = tenant_id.map(|t| t.0.as_str());

        sqlx::query!(
            r#"
            INSERT INTO workflows (
                id, name, description, owner, tags, tenant_id, version, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            workflow.id,
            workflow.name,
            workflow.description,
            workflow.owner,
            workflow.tags as _,
            tenant_str,
            workflow.version,
            workflow.created_at,
            workflow.updated_at
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Create a new execution
    pub async fn create_execution(
        &self,
        execution: &ExecutionModel,
        tenant_id: Option<&TenantId>,
    ) -> Result<()> {
        let tenant_str = tenant_id.map(|t| t.0.as_str());

        sqlx::query!(
            r#"
            INSERT INTO executions (
                id, workflow_id, state, tenant_id, started_at,
                completed_at, error_message, created_at, updated_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
            execution.id,
            execution.workflow_id,
            execution.state,
            tenant_str,
            execution.started_at,
            execution.completed_at,
            execution.error_message,
            execution.created_at,
            execution.updated_at
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would go here
    // They require a test database setup
}
