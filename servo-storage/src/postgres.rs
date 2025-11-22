//! PostgreSQL storage implementation

use crate::{models::*, Result, TenantId};
use futures::future::BoxFuture;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};
use tracing::instrument;
use uuid::Uuid;

/// Configuration for the PostgreSQL connection pool
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
    pub idle_timeout_secs: Option<u64>,
    pub max_lifetime_secs: Option<u64>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 5,
            min_connections: 1,
            acquire_timeout_secs: 30,
            idle_timeout_secs: None,
            max_lifetime_secs: None,
        }
    }
}

/// PostgreSQL storage backend
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Create a new PostgreSQL storage instance with default configuration
    pub async fn new(database_url: &str) -> Result<Self> {
        Self::with_pool_config(database_url, PoolConfig::default()).await
    }

    /// Create a new PostgreSQL storage instance with custom configuration
    pub async fn with_config(
        database_url: &str,
        max_connections: u32,
        connection_timeout_secs: u64,
    ) -> Result<Self> {
        let cfg = PoolConfig {
            max_connections,
            min_connections: 1,
            acquire_timeout_secs: connection_timeout_secs,
            idle_timeout_secs: None,
            max_lifetime_secs: None,
        };
        Self::with_pool_config(database_url, cfg).await
    }

    /// Create a new PostgreSQL storage instance using a PoolConfig
    pub async fn with_pool_config(database_url: &str, config: PoolConfig) -> Result<Self> {
        if config.min_connections == 0 {
            return Err(crate::Error::ValidationError(
                "min_connections must be > 0".to_string(),
            ));
        }
        if config.max_connections == 0 || config.max_connections < config.min_connections {
            return Err(crate::Error::ValidationError(
                "max_connections must be >= min_connections and > 0".to_string(),
            ));
        }

        let mut opts = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(std::time::Duration::from_secs(config.acquire_timeout_secs));

        if let Some(idle) = config.idle_timeout_secs {
            opts = opts.idle_timeout(std::time::Duration::from_secs(idle));
        }
        if let Some(max_life) = config.max_lifetime_secs {
            opts = opts.max_lifetime(std::time::Duration::from_secs(max_life));
        }

        let pool = opts.connect(database_url).await?;
        Ok(Self { pool })
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Execute a closure within a transaction with tenant context set
    ///
    /// This method:
    /// 1. Begins a new transaction
    /// 2. Sets the `app.current_tenant` session variable for RLS enforcement
    /// 3. Executes the provided closure with the transaction
    /// 4. Commits on success, rolls back on error
    async fn with_tenant_context<F, T>(&self, tenant_id: &TenantId, f: F) -> Result<T>
    where
        F: for<'c> FnOnce(&'c mut Transaction<'_, Postgres>) -> BoxFuture<'c, Result<T>> + Send,
        T: Send,
    {
        let mut tx = self.pool.begin().await?;

        // Set tenant context for RLS enforcement. Use set_config to avoid SET parameter syntax issues.
        sqlx::query("SELECT set_config('app.current_tenant', $1, true)")
            .bind(tenant_id.as_str())
            .execute(&mut *tx)
            .await?;

        // Execute the closure
        match f(&mut tx).await {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }

    /// Execute a closure within a transaction without tenant context
    ///
    /// This method:
    /// 1. Begins a new transaction
    /// 2. Executes the provided closure with the transaction
    /// 3. Commits on success, rolls back on error
    #[allow(dead_code)]
    async fn with_transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: for<'c> FnOnce(&'c mut Transaction<'_, Postgres>) -> BoxFuture<'c, Result<T>> + Send,
        T: Send,
    {
        let mut tx = self.pool.begin().await?;

        // Execute the closure
        match f(&mut tx).await {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        }
    }

    // ========== Input Validation Helpers ==========

    /// Validate pagination parameters
    fn validate_pagination_params(limit: i64, offset: i64) -> Result<()> {
        if limit <= 0 {
            return Err(crate::Error::ValidationError(
                "Limit must be greater than 0".to_string(),
            ));
        }
        if limit > 1000 {
            return Err(crate::Error::ValidationError(
                "Limit cannot exceed 1000".to_string(),
            ));
        }
        if offset < 0 {
            return Err(crate::Error::ValidationError(
                "Offset must be non-negative".to_string(),
            ));
        }
        Ok(())
    }

    /// Validate execution state
    fn validate_execution_state(state: &str) -> Result<()> {
        match state {
            "pending" | "running" | "succeeded" | "failed" | "cancelled" | "timeout" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid execution state: {}",
                state
            ))),
        }
    }

    /// Validate that a string field is not empty
    fn validate_non_empty_string(value: &str, field_name: &str) -> Result<()> {
        if value.trim().is_empty() {
            return Err(crate::Error::ValidationError(format!(
                "{} cannot be empty",
                field_name
            )));
        }
        Ok(())
    }

    /// Validate dependency type
    fn validate_dependency_type(dep_type: &str) -> Result<()> {
        match dep_type {
            "data" | "metadata" | "control" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid dependency type: {}",
                dep_type
            ))),
        }
    }

    // ========== CRUD Operations ==========

    /// Create a new asset
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, asset, tenant_id), fields(tenant = %tenant_id.as_str(), asset_id = %asset.id))]
    pub async fn create_asset(&self, asset: &AssetModel, tenant_id: &TenantId) -> Result<()> {
        // Validate input
        Self::validate_non_empty_string(&asset.name, "Asset name")?;
        Self::validate_non_empty_string(&asset.asset_type, "Asset type")?;

        let asset = asset.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO assets (
                        id, name, description, asset_type, owner, tags, tenant_id, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(asset.id)
                .bind(&asset.name)
                .bind(&asset.description)
                .bind(&asset.asset_type)
                .bind(&asset.owner)
                .bind(&asset.tags)
                .bind(&tenant_str)
                .bind(asset.created_at)
                .bind(asset.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Get an asset by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), asset_id = %id))]
    pub async fn get_asset(&self, id: Uuid, tenant_id: &TenantId) -> Result<AssetModel> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let asset = sqlx::query_as::<_, AssetModel>(
                    r#"
                    SELECT id,
                           name,
                           description,
                           asset_type,
                           owner,
                           tags,
                           tenant_id,
                           created_at,
                           updated_at
                    FROM assets
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Asset {}", id)))?;

                Ok(asset)
            })
        })
        .await
    }

    /// Create a new workflow
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, workflow, tenant_id), fields(tenant = %tenant_id.as_str(), workflow_id = %workflow.id))]
    pub async fn create_workflow(
        &self,
        workflow: &WorkflowModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&workflow.name, "Workflow name")?;

        let workflow = workflow.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO workflows (
                        id, name, description, owner, tags, tenant_id, version, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(workflow.id)
                .bind(&workflow.name)
                .bind(&workflow.description)
                .bind(&workflow.owner)
                .bind(&workflow.tags)
                .bind(&tenant_str)
                .bind(workflow.version)
                .bind(workflow.created_at)
                .bind(workflow.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Create a new execution
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, execution, tenant_id), fields(tenant = %tenant_id.as_str(), execution_id = %execution.id))]
    pub async fn create_execution(
        &self,
        execution: &ExecutionModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_execution_state(&execution.state)?;
        let execution = execution.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO executions (
                        id, workflow_id, state, tenant_id, started_at,
                        completed_at, error_message, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    "#,
                )
                .bind(execution.id)
                .bind(execution.workflow_id)
                .bind(&execution.state)
                .bind(&tenant_str)
                .bind(execution.started_at)
                .bind(execution.completed_at)
                .bind(&execution.error_message)
                .bind(execution.created_at)
                .bind(execution.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Update an existing asset
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, asset, tenant_id), fields(tenant = %tenant_id.as_str(), asset_id = %asset.id))]
    pub async fn update_asset(&self, asset: &AssetModel, tenant_id: &TenantId) -> Result<()> {
        Self::validate_non_empty_string(&asset.name, "Asset name")?;
        Self::validate_non_empty_string(&asset.asset_type, "Asset type")?;

        let asset = asset.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE assets
                    SET name = $2,
                        description = $3,
                        asset_type = $4,
                        owner = $5,
                        tags = $6,
                        updated_at = $7
                    WHERE id = $1
                    "#,
                )
                .bind(asset.id)
                .bind(&asset.name)
                .bind(&asset.description)
                .bind(&asset.asset_type)
                .bind(&asset.owner)
                .bind(&asset.tags)
                .bind(asset.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Asset {}", asset.id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Update an existing workflow
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, workflow, tenant_id), fields(tenant = %tenant_id.as_str(), workflow_id = %workflow.id))]
    pub async fn update_workflow(
        &self,
        workflow: &WorkflowModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&workflow.name, "Workflow name")?;

        let workflow = workflow.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE workflows
                    SET name = $2,
                        description = $3,
                        owner = $4,
                        tags = $5,
                        version = $6,
                        updated_at = $7
                    WHERE id = $1
                    "#,
                )
                .bind(workflow.id)
                .bind(&workflow.name)
                .bind(&workflow.description)
                .bind(&workflow.owner)
                .bind(&workflow.tags)
                .bind(workflow.version)
                .bind(workflow.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Workflow {}", workflow.id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Update an existing execution
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, execution, tenant_id), fields(tenant = %tenant_id.as_str(), execution_id = %execution.id))]
    pub async fn update_execution(
        &self,
        execution: &ExecutionModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_execution_state(&execution.state)?;
        let execution = execution.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE executions
                    SET state = $2,
                        started_at = $3,
                        completed_at = $4,
                        error_message = $5,
                        updated_at = $6
                    WHERE id = $1
                    "#,
                )
                .bind(execution.id)
                .bind(&execution.state)
                .bind(execution.started_at)
                .bind(execution.completed_at)
                .bind(&execution.error_message)
                .bind(execution.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Execution {}",
                        execution.id
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// Delete an asset by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    pub async fn delete_asset(&self, id: Uuid, tenant_id: &TenantId) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    DELETE FROM assets
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Asset {}", id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Delete a workflow by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    pub async fn delete_workflow(&self, id: Uuid, tenant_id: &TenantId) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    DELETE FROM workflows
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Workflow {}", id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Delete an execution by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    pub async fn delete_execution(&self, id: Uuid, tenant_id: &TenantId) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    DELETE FROM executions
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Execution {}", id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// List assets with pagination
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn list_assets(
        &self,
        tenant_id: &TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AssetModel>> {
        Self::validate_pagination_params(limit, offset)?;

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let assets = sqlx::query_as::<_, AssetModel>(
                    r#"
                    SELECT id, name, description, asset_type, owner, tags, tenant_id, created_at, updated_at
                    FROM assets
                    ORDER BY created_at DESC, id DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(assets)
            })
        })
        .await
    }

    /// Get workflow by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    pub async fn get_workflow(&self, id: Uuid, tenant_id: &TenantId) -> Result<WorkflowModel> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let workflow = sqlx::query_as::<_, WorkflowModel>(
                    r#"
                    SELECT id, name, description, owner, tags, tenant_id, version, created_at, updated_at
                    FROM workflows
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Workflow {}", id)))?;

                Ok(workflow)
            })
        })
        .await
    }

    /// List workflows with pagination
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn list_workflows(
        &self,
        tenant_id: &TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WorkflowModel>> {
        Self::validate_pagination_params(limit, offset)?;

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let workflows = sqlx::query_as::<_, WorkflowModel>(
                    r#"
                    SELECT id, name, description, owner, tags, tenant_id, version, created_at, updated_at
                    FROM workflows
                    ORDER BY created_at DESC, id DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(workflows)
            })
        })
        .await
    }

    /// Get execution by ID
    ///
    /// This operation enforces tenant isolation via RLS policies.
    pub async fn get_execution(&self, id: Uuid, tenant_id: &TenantId) -> Result<ExecutionModel> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let execution = sqlx::query_as::<_, ExecutionModel>(
                    r#"
                    SELECT id, workflow_id, state, tenant_id, started_at, completed_at, error_message, created_at, updated_at
                    FROM executions
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Execution {}", id)))?;

                Ok(execution)
            })
        })
        .await
    }

    /// List executions with pagination
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn list_executions(
        &self,
        tenant_id: &TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ExecutionModel>> {
        Self::validate_pagination_params(limit, offset)?;

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let executions = sqlx::query_as::<_, ExecutionModel>(
                    r#"
                    SELECT id, workflow_id, state, tenant_id, started_at, completed_at, error_message, created_at, updated_at
                    FROM executions
                    ORDER BY created_at DESC, id DESC
                    LIMIT $1 OFFSET $2
                    "#,
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(executions)
            })
        })
        .await
    }

    /// List executions for a specific workflow
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), workflow_id = %workflow_id))]
    pub async fn list_workflow_executions(
        &self,
        workflow_id: Uuid,
        tenant_id: &TenantId,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ExecutionModel>> {
        Self::validate_pagination_params(limit, offset)?;

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let executions = sqlx::query_as::<_, ExecutionModel>(
                    r#"
                    SELECT id, workflow_id, state, tenant_id, started_at, completed_at, error_message, created_at, updated_at
                    FROM executions
                    WHERE workflow_id = $1
                    ORDER BY created_at DESC, id DESC
                    LIMIT $2 OFFSET $3
                    "#,
                )
                .bind(workflow_id)
                .bind(limit)
                .bind(offset)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(executions)
            })
        })
        .await
    }

    /// Count total assets for a tenant
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn count_assets(&self, tenant_id: &TenantId) -> Result<i64> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let count: (i64,) = sqlx::query_as(
                    r#"
                    SELECT COUNT(*) FROM assets
                    "#,
                )
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(count.0)
            })
        })
        .await
    }

    /// Count total workflows for a tenant
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn count_workflows(&self, tenant_id: &TenantId) -> Result<i64> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let count: (i64,) = sqlx::query_as(
                    r#"
                    SELECT COUNT(*) FROM workflows
                    "#,
                )
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(count.0)
            })
        })
        .await
    }

    /// Count total executions for a tenant
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str()))]
    pub async fn count_executions(&self, tenant_id: &TenantId) -> Result<i64> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let count: (i64,) = sqlx::query_as(
                    r#"
                    SELECT COUNT(*) FROM executions
                    "#,
                )
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(count.0)
            })
        })
        .await
    }

    // ========== Asset Dependency Operations (for lineage) ==========

    /// Create an asset dependency relationship
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), upstream = %upstream_asset_id, downstream = %downstream_asset_id))]
    pub async fn create_asset_dependency(
        &self,
        upstream_asset_id: Uuid,
        downstream_asset_id: Uuid,
        dependency_type: &str,
        tenant_id: &TenantId,
    ) -> Result<Uuid> {
        if upstream_asset_id == downstream_asset_id {
            return Err(crate::Error::ValidationError(
                "Cannot create self-referential dependency".to_string(),
            ));
        }
        Self::validate_dependency_type(dependency_type)?;

        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let dependency_type = dependency_type.to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO asset_dependencies (
                        id, upstream_asset_id, downstream_asset_id, dependency_type, created_at
                    ) VALUES ($1, $2, $3, $4, $5)
                    "#,
                )
                .bind(id)
                .bind(upstream_asset_id)
                .bind(downstream_asset_id)
                .bind(&dependency_type)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(id)
            })
        })
        .await
    }

    /// Get all dependencies for a given asset (both upstream and downstream)
    pub async fn get_asset_dependencies(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<(
        Vec<crate::models::AssetDependencyModel>,
        Vec<crate::models::AssetDependencyModel>,
    )> {
        use crate::models::AssetDependencyModel;

        #[allow(clippy::redundant_clone)]
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let upstream = sqlx::query_as::<_, AssetDependencyModel>(
                    r#"
                    SELECT id, upstream_asset_id, downstream_asset_id, dependency_type, created_at
                    FROM asset_dependencies
                    WHERE downstream_asset_id = $1
                    "#,
                )
                .bind(asset_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                let downstream = sqlx::query_as::<_, AssetDependencyModel>(
                    r#"
                    SELECT id, upstream_asset_id, downstream_asset_id, dependency_type, created_at
                    FROM asset_dependencies
                    WHERE upstream_asset_id = $1
                    "#,
                )
                .bind(asset_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok((upstream, downstream))
            })
        })
        .await
    }

    /// Get upstream dependencies for an asset (what the asset depends on)
    pub async fn get_upstream_dependencies(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<crate::models::AssetDependencyModel>> {
        use crate::models::AssetDependencyModel;

        #[allow(clippy::redundant_clone)]
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let upstream = sqlx::query_as::<_, AssetDependencyModel>(
                    r#"
                    SELECT id, upstream_asset_id, downstream_asset_id, dependency_type, created_at
                    FROM asset_dependencies
                    WHERE downstream_asset_id = $1
                    "#,
                )
                .bind(asset_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(upstream)
            })
        })
        .await
    }

    /// Get downstream dependencies for an asset (what depends on the asset)
    pub async fn get_downstream_dependencies(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<crate::models::AssetDependencyModel>> {
        use crate::models::AssetDependencyModel;

        #[allow(clippy::redundant_clone)]
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let downstream = sqlx::query_as::<_, AssetDependencyModel>(
                    r#"
                    SELECT id, upstream_asset_id, downstream_asset_id, dependency_type, created_at
                    FROM asset_dependencies
                    WHERE upstream_asset_id = $1
                    "#,
                )
                .bind(asset_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(downstream)
            })
        })
        .await
    }

    /// Delete an asset dependency by ID
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), dependency_id = %id))]
    pub async fn delete_asset_dependency(&self, id: Uuid, tenant_id: &TenantId) -> Result<()> {
        let result = self
            .with_tenant_context(tenant_id, |tx| {
                Box::pin(async move {
                    let res = sqlx::query(
                        r#"
                        DELETE FROM asset_dependencies
                        WHERE id = $1
                        "#,
                    )
                    .bind(id)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    Ok(res)
                })
            })
            .await?;

        if result.rows_affected() == 0 {
            return Err(crate::Error::NotFound(format!("AssetDependency {}", id)));
        }

        Ok(())
    }

    /// Delete all dependencies for an asset
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), asset_id = %asset_id))]
    pub async fn delete_asset_dependencies_for_asset(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<u64> {
        let result = self
            .with_tenant_context(tenant_id, |tx| {
                Box::pin(async move {
                    let res = sqlx::query(
                        r#"
                        DELETE FROM asset_dependencies
                        WHERE upstream_asset_id = $1 OR downstream_asset_id = $1
                        "#,
                    )
                    .bind(asset_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    Ok(res)
                })
            })
            .await?;

        Ok(result.rows_affected())
    }

    /// Get the full lineage graph for an asset (recursively traverses all dependencies)
    #[instrument(skip(self, tenant_id), fields(tenant = %tenant_id.as_str(), asset_id = %asset_id, max_depth))]
    pub async fn get_asset_lineage(
        &self,
        asset_id: Uuid,
        max_depth: usize,
        tenant_id: &TenantId,
    ) -> Result<Vec<crate::models::AssetDependencyModel>> {
        use std::collections::{HashSet, VecDeque};

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut lineage = Vec::new();

        queue.push_back((asset_id, 0));
        visited.insert(asset_id);

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            // Get all dependencies (both upstream and downstream) for current asset
            let (upstream, downstream) = self.get_asset_dependencies(current_id, tenant_id).await?;

            // Add upstream dependencies to lineage
            for dep in upstream {
                if !visited.contains(&dep.upstream_asset_id) {
                    visited.insert(dep.upstream_asset_id);
                    queue.push_back((dep.upstream_asset_id, depth + 1));
                }
                lineage.push(dep);
            }

            // Add downstream dependencies to lineage
            for dep in downstream {
                if !visited.contains(&dep.downstream_asset_id) {
                    visited.insert(dep.downstream_asset_id);
                    queue.push_back((dep.downstream_asset_id, depth + 1));
                }
                lineage.push(dep);
            }
        }

        Ok(lineage)
    }

    /// Health check for readiness probes
    #[instrument(skip(self))]
    pub async fn health_check(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_db_error)
    }
}

fn map_db_error(err: sqlx::Error) -> crate::Error {
    // Map connection-related errors
    match &err {
        sqlx::Error::PoolTimedOut => {
            return crate::Error::PoolExhausted("Connection pool timed out".to_string());
        }
        sqlx::Error::PoolClosed => {
            return crate::Error::ConnectionFailed("Connection pool closed".to_string());
        }
        _ => {}
    }

    // Map PostgreSQL-specific errors
    if let sqlx::Error::Database(db_err) = &err {
        if let Some(code) = db_err.code().as_deref() {
            match code {
                // unique_violation - duplicate key
                "23505" => {
                    return crate::Error::AlreadyExists(db_err.message().to_string());
                }
                // foreign_key_violation - referenced record doesn't exist
                "23503" => {
                    return crate::Error::NotFound(db_err.message().to_string());
                }
                // not_null_violation - required field is null
                "23502" => {
                    return crate::Error::ValidationError(format!(
                        "Required field cannot be null: {}",
                        db_err.message()
                    ));
                }
                // check_violation - CHECK constraint failed
                "23514" => {
                    return crate::Error::ValidationError(format!(
                        "Constraint violation: {}",
                        db_err.message()
                    ));
                }
                // too_many_connections - connection pool exhausted
                "53300" => {
                    return crate::Error::PoolExhausted(db_err.message().to_string());
                }
                // connection_failure - database connection failed
                "08006" | "08001" | "08003" | "08004" => {
                    return crate::Error::ConnectionFailed(db_err.message().to_string());
                }
                _ => {}
            }
        }
    }

    crate::Error::Database(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::types::Json;

    /// Get test database URL from environment or use default
    fn get_test_database_url() -> String {
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://servo:servo@localhost:5432/servo_test".to_string())
    }

    /// Setup test database with migrations
    async fn setup_test_db() -> Result<PostgresStorage> {
        let db_url = get_test_database_url();
        let storage = PostgresStorage::new(&db_url).await?;

        // Drop all tables to ensure clean state (important for CI where DB persists across test runs)
        sqlx::query("DROP SCHEMA public CASCADE")
            .execute(storage.pool())
            .await?;

        sqlx::query("CREATE SCHEMA public")
            .execute(storage.pool())
            .await?;

        // Grant permissions on the schema
        sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
            .execute(storage.pool())
            .await?;

        // Run migrations fresh
        crate::migrations::run_migrations(storage.pool()).await?;

        // Close the pool and create a new connection to ensure schema changes are picked up
        drop(storage);
        let storage = PostgresStorage::new(&db_url).await?;

        Ok(storage)
    }

    fn unique_tenant() -> TenantId {
        TenantId::new(uuid::Uuid::new_v4().to_string())
    }

    fn unique_name(prefix: &str) -> String {
        format!("{}_{}", prefix, uuid::Uuid::new_v4())
    }

    /// Clean up all data for a specific tenant (tenant-scoped, preserves RLS)
    async fn cleanup_tenant(storage: &PostgresStorage, tenant: &TenantId) -> Result<()> {
        // Clone tenant string to satisfy 'static lifetime requirement of async move
        let tenant_str = tenant.as_str().to_string();

        // Delete in correct order to respect foreign keys
        // asset_dependencies will cascade from assets
        storage
            .with_tenant_context(tenant, |tx| {
                let tenant_str = tenant_str.clone();
                Box::pin(async move {
                    // Delete executions first (references workflows)
                    sqlx::query("DELETE FROM executions WHERE tenant_id = $1")
                        .bind(&tenant_str)
                        .execute(&mut **tx)
                        .await?;

                    // Delete workflows
                    sqlx::query("DELETE FROM workflows WHERE tenant_id = $1")
                        .bind(&tenant_str)
                        .execute(&mut **tx)
                        .await?;

                    // Delete asset_dependencies (will be deleted by cascade, but explicit is safer)
                    sqlx::query(
                        "DELETE FROM asset_dependencies WHERE id IN (
                            SELECT ad.id FROM asset_dependencies ad
                            JOIN assets a ON ad.upstream_asset_id = a.id
                            WHERE a.tenant_id = $1
                        )",
                    )
                    .bind(&tenant_str)
                    .execute(&mut **tx)
                    .await?;

                    // Delete assets (this will cascade to asset_dependencies due to FK)
                    sqlx::query("DELETE FROM assets WHERE tenant_id = $1")
                        .bind(&tenant_str)
                        .execute(&mut **tx)
                        .await?;

                    Ok(())
                })
            })
            .await
    }

    #[test]
    fn test_validate_dependency_type() {
        assert!(PostgresStorage::validate_dependency_type("data").is_ok());
        assert!(PostgresStorage::validate_dependency_type("metadata").is_ok());
        assert!(PostgresStorage::validate_dependency_type("control").is_ok());
        assert!(PostgresStorage::validate_dependency_type("invalid").is_err());
    }

    #[test]
    fn test_validate_execution_state() {
        for state in [
            "pending",
            "running",
            "succeeded",
            "failed",
            "cancelled",
            "timeout",
        ] {
            assert!(PostgresStorage::validate_execution_state(state).is_ok());
        }
        assert!(PostgresStorage::validate_execution_state("bogus").is_err());
    }

    #[tokio::test]
    #[ignore] // Run with: cargo test -- --ignored
    async fn test_create_and_get_asset() {
        let storage = setup_test_db().await.expect("Failed to setup test db");
        let tenant = unique_tenant();

        let asset = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("test_asset"),
            description: Some("Test description".to_string()),
            asset_type: "table".to_string(),
            owner: Some("test_user".to_string()),
            tags: Json(vec!["tag1".to_string(), "tag2".to_string()]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Create asset
        storage
            .create_asset(&asset, &tenant)
            .await
            .expect("Failed to create asset");

        // Get asset
        let retrieved = storage
            .get_asset(asset.id, &tenant)
            .await
            .expect("Failed to get asset");

        assert_eq!(retrieved.id, asset.id);
        assert_eq!(retrieved.name, asset.name);
        assert_eq!(retrieved.description, asset.description);

        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_update_asset() {
        let storage = setup_test_db().await.expect("Failed to setup test db");
        let tenant = unique_tenant();

        let mut asset = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("test_asset"),
            description: Some("Original description".to_string()),
            asset_type: "table".to_string(),
            owner: Some("test_user".to_string()),
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset, &tenant)
            .await
            .expect("Failed to create asset");

        // Update asset
        asset.description = Some("Updated description".to_string());
        asset.updated_at = Utc::now();

        storage
            .update_asset(&asset, &tenant)
            .await
            .expect("Failed to update asset");

        // Verify update
        let retrieved = storage
            .get_asset(asset.id, &tenant)
            .await
            .expect("Failed to get asset");

        assert_eq!(
            retrieved.description,
            Some("Updated description".to_string())
        );
        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_delete_asset() {
        let storage = setup_test_db().await.expect("Failed to setup test db");
        let tenant = unique_tenant();

        let asset = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("test_asset"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset, &tenant)
            .await
            .expect("Failed to create asset");

        // Delete asset
        storage
            .delete_asset(asset.id, &tenant)
            .await
            .expect("Failed to delete asset");

        // Verify deletion
        let result = storage.get_asset(asset.id, &tenant).await;

        assert!(result.is_err());
        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_assets_with_pagination() {
        let storage = setup_test_db().await.expect("Failed to setup test db");
        let tenant = unique_tenant();

        // Create multiple assets
        for i in 0..5 {
            let asset = AssetModel {
                id: Uuid::new_v4(),
                name: unique_name(&format!("asset_{}", i)),
                description: None,
                asset_type: "table".to_string(),
                owner: None,
                tags: Json(vec![]),
                tenant_id: Some(tenant.as_str().to_string()),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };

            storage
                .create_asset(&asset, &tenant)
                .await
                .expect("Failed to create asset");
        }

        // List with pagination
        let page1 = storage
            .list_assets(&tenant, 2, 0)
            .await
            .expect("Failed to list assets");

        let page2 = storage
            .list_assets(&tenant, 2, 2)
            .await
            .expect("Failed to list assets");

        assert_eq!(page1.len(), 2);
        assert_eq!(page2.len(), 2);

        // Verify total count
        let count = storage
            .count_assets(&tenant)
            .await
            .expect("Failed to count assets");

        assert_eq!(count, 5);
        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_tenant_isolation() {
        let storage = setup_test_db().await.expect("Failed to setup test db");

        let tenant1 = unique_tenant();
        let tenant2 = unique_tenant();

        // Create assets for tenant1
        let asset1 = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("tenant1_asset"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant1.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset1, &tenant1)
            .await
            .expect("Failed to create asset for tenant1");

        // Create assets for tenant2
        let asset2 = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("tenant2_asset"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant2.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset2, &tenant2)
            .await
            .expect("Failed to create asset for tenant2");

        // Tenant1 should only see their asset
        let tenant1_assets = storage
            .list_assets(&tenant1, 100, 0)
            .await
            .expect("Failed to list tenant1 assets");

        assert_eq!(tenant1_assets.len(), 1);
        assert_eq!(tenant1_assets[0].id, asset1.id);

        // Tenant2 should only see their asset
        let tenant2_assets = storage
            .list_assets(&tenant2, 100, 0)
            .await
            .expect("Failed to list tenant2 assets");

        assert_eq!(tenant2_assets.len(), 1);
        assert_eq!(tenant2_assets[0].id, asset2.id);

        // Tenant1 should not be able to access tenant2's asset
        let result = storage.get_asset(asset2.id, &tenant1).await;

        assert!(result.is_err());
        cleanup_tenant(&storage, &tenant1).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_asset_dependencies() {
        let storage = setup_test_db().await.expect("Failed to setup test db");
        let tenant = unique_tenant();

        // Create upstream asset
        let upstream = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("upstream"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&upstream, &tenant)
            .await
            .expect("Failed to create upstream asset");

        // Create downstream asset
        let downstream = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("downstream"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&downstream, &tenant)
            .await
            .expect("Failed to create downstream asset");

        // Create dependency
        let dep_id = storage
            .create_asset_dependency(upstream.id, downstream.id, "data", &tenant)
            .await
            .expect("Failed to create dependency");

        assert!(dep_id != Uuid::nil());

        // Get dependencies
        let (upstream_deps, _downstream_deps) = storage
            .get_asset_dependencies(downstream.id, &tenant)
            .await
            .expect("Failed to get dependencies");

        assert_eq!(upstream_deps.len(), 1);
        assert_eq!(upstream_deps[0].upstream_asset_id, upstream.id);
        assert_eq!(upstream_deps[0].downstream_asset_id, downstream.id);

        // Get downstream dependencies of upstream
        let downstream_of_upstream = storage
            .get_downstream_dependencies(upstream.id, &tenant)
            .await
            .expect("Failed to get downstream dependencies");

        assert_eq!(downstream_of_upstream.len(), 1);
        assert_eq!(downstream_of_upstream[0].downstream_asset_id, downstream.id);
        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_asset_lineage() {
        let storage = setup_test_db().await.expect("Failed to setup test db");

        let tenant = unique_tenant();

        // Create a chain: A -> B -> C
        let asset_a = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("asset_a"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let asset_b = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("asset_b"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let asset_c = AssetModel {
            id: Uuid::new_v4(),
            name: unique_name("asset_c"),
            description: None,
            asset_type: "table".to_string(),
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some(tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage.create_asset(&asset_a, &tenant).await.unwrap();
        storage.create_asset(&asset_b, &tenant).await.unwrap();
        storage.create_asset(&asset_c, &tenant).await.unwrap();

        // Create dependencies
        storage
            .create_asset_dependency(asset_a.id, asset_b.id, "data", &tenant)
            .await
            .unwrap();

        storage
            .create_asset_dependency(asset_b.id, asset_c.id, "data", &tenant)
            .await
            .unwrap();

        // Get lineage for asset_b (should include both A and C)
        let lineage = storage
            .get_asset_lineage(asset_b.id, 10, &tenant)
            .await
            .expect("Failed to get lineage");

        assert_eq!(lineage.len(), 2);
        cleanup_tenant(&storage, &tenant).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_workflows_crud() {
        let storage = setup_test_db().await.expect("Failed to setup test db");

        let workflow = WorkflowModel {
            id: Uuid::new_v4(),
            name: "test_workflow".to_string(),
            description: Some("Test workflow".to_string()),
            owner: Some("test_user".to_string()),
            tags: Json(vec!["daily".to_string()]),
            tenant_id: Some("tenant1".to_string()),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // Create workflow
        storage
            .create_workflow(&workflow, &TenantId::new("tenant1"))
            .await
            .expect("Failed to create workflow");

        // Get workflow
        let retrieved = storage
            .get_workflow(workflow.id, &TenantId::new("tenant1"))
            .await
            .expect("Failed to get workflow");

        assert_eq!(retrieved.name, workflow.name);

        // List workflows
        let workflows = storage
            .list_workflows(&TenantId::new("tenant1"), 10, 0)
            .await
            .expect("Failed to list workflows");

        assert_eq!(workflows.len(), 1);
        cleanup_tenant(&storage, &TenantId::new("tenant1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_executions_crud() {
        let storage = setup_test_db().await.expect("Failed to setup test db");

        // First create a workflow
        let workflow = WorkflowModel {
            id: Uuid::new_v4(),
            name: "test_workflow".to_string(),
            description: None,
            owner: None,
            tags: Json(vec![]),
            tenant_id: Some("tenant1".to_string()),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_workflow(&workflow, &TenantId::new("tenant1"))
            .await
            .expect("Failed to create workflow");

        // Create execution
        let execution = ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id: workflow.id,
            state: "running".to_string(),
            tenant_id: Some("tenant1".to_string()),
            started_at: Some(Utc::now()),
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_execution(&execution, &TenantId::new("tenant1"))
            .await
            .expect("Failed to create execution");

        // Get execution
        let retrieved = storage
            .get_execution(execution.id, &TenantId::new("tenant1"))
            .await
            .expect("Failed to get execution");

        assert_eq!(retrieved.state, "running");

        // List executions for workflow
        let executions = storage
            .list_workflow_executions(workflow.id, &TenantId::new("tenant1"), 10, 0)
            .await
            .expect("Failed to list executions");

        assert_eq!(executions.len(), 1);
        cleanup_tenant(&storage, &TenantId::new("tenant1"))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_rls_enforcement_comprehensive() {
        let storage = setup_test_db().await.expect("Failed to setup test db");

        // Create asset for tenant1
        let asset1 = AssetModel {
            id: Uuid::new_v4(),
            name: "tenant1_private_asset".to_string(),
            description: Some("Tenant1's private data".to_string()),
            asset_type: "table".to_string(),
            owner: Some("tenant1_owner".to_string()),
            tags: Json(vec!["private".to_string()]),
            tenant_id: Some("tenant1".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset1, &TenantId::new("tenant1"))
            .await
            .expect("Failed to create asset for tenant1");

        // Create asset for tenant2
        let asset2 = AssetModel {
            id: Uuid::new_v4(),
            name: "tenant2_private_asset".to_string(),
            description: Some("Tenant2's private data".to_string()),
            asset_type: "table".to_string(),
            owner: Some("tenant2_owner".to_string()),
            tags: Json(vec!["private".to_string()]),
            tenant_id: Some("tenant2".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage
            .create_asset(&asset2, &TenantId::new("tenant2"))
            .await
            .expect("Failed to create asset for tenant2");

        // TEST 1: Tenant1 cannot GET tenant2's asset
        let result = storage
            .get_asset(asset2.id, &TenantId::new("tenant1"))
            .await;
        assert!(
            result.is_err(),
            "Tenant1 should not be able to GET tenant2's asset"
        );

        // TEST 2: Tenant1 cannot UPDATE tenant2's asset
        let mut asset2_modified = asset2.clone();
        asset2_modified.description = Some("HACKED by tenant1".to_string());

        let result = storage
            .update_asset(&asset2_modified, &TenantId::new("tenant1"))
            .await;
        assert!(
            result.is_err(),
            "Tenant1 should not be able to UPDATE tenant2's asset"
        );

        // Verify asset2 was not modified
        let asset2_check = storage
            .get_asset(asset2.id, &TenantId::new("tenant2"))
            .await
            .expect("Failed to get asset2 as tenant2");
        assert_eq!(
            asset2_check.description.as_deref(),
            Some("Tenant2's private data"),
            "Asset2 should not have been modified"
        );

        // TEST 3: Tenant1 cannot DELETE tenant2's asset
        let result = storage
            .delete_asset(asset2.id, &TenantId::new("tenant1"))
            .await;
        assert!(
            result.is_err(),
            "Tenant1 should not be able to DELETE tenant2's asset"
        );

        // Verify asset2 still exists
        let asset2_check = storage
            .get_asset(asset2.id, &TenantId::new("tenant2"))
            .await;
        assert!(
            asset2_check.is_ok(),
            "Asset2 should still exist after failed delete attempt"
        );

        // TEST 4: Tenant1 can only see their own assets in list
        let tenant1_assets = storage
            .list_assets(&TenantId::new("tenant1"), 100, 0)
            .await
            .expect("Failed to list tenant1 assets");

        assert_eq!(tenant1_assets.len(), 1);
        assert_eq!(tenant1_assets[0].id, asset1.id);

        // TEST 5: Tenant1 count should only include their assets
        let tenant1_count = storage
            .count_assets(&TenantId::new("tenant1"))
            .await
            .expect("Failed to count tenant1 assets");

        assert_eq!(tenant1_count, 1);

        // TEST 6: Tenant2 can access their own data
        let asset2_valid = storage
            .get_asset(asset2.id, &TenantId::new("tenant2"))
            .await
            .expect("Tenant2 should be able to access their own asset");

        assert_eq!(asset2_valid.id, asset2.id);
        cleanup_tenant(&storage, &TenantId::new("tenant1"))
            .await
            .unwrap();
    }
}
