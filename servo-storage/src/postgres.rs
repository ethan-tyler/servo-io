//! PostgreSQL storage implementation

use crate::{models::*, Result, TenantId};
use chrono::Utc;
use futures::future::BoxFuture;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};
use std::time::Instant;
use tracing::{error, instrument, warn};
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
        let start = Instant::now();
        let mut tx = self.pool.begin().await?;

        // Set tenant context for RLS enforcement. Use set_config to avoid SET parameter syntax issues.
        sqlx::query("SELECT set_config('app.current_tenant', $1, true)")
            .bind(tenant_id.as_str())
            .execute(&mut *tx)
            .await?;

        // Execute the closure
        let result = match f(&mut tx).await {
            Ok(result) => {
                tx.commit().await?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback().await?;
                Err(e)
            }
        };

        // Log slow operations (>100ms)
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 100 {
            warn!(
                tenant_id = %tenant_id.as_str(),
                duration_ms = elapsed.as_millis(),
                "Slow database operation detected"
            );
        }

        result
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
    #[instrument(
        skip(self, asset, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "assets",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset.id
        )
    )]
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
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "assets",
            tenant_id = %tenant_id.as_str(),
            asset_id = %id
        )
    )]
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
    #[instrument(
        skip(self, workflow, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "workflows",
            tenant_id = %tenant_id.as_str(),
            workflow_id = %workflow.id
        )
    )]
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
    #[instrument(
        skip(self, execution, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "executions",
            tenant_id = %tenant_id.as_str(),
            execution_id = %execution.id
        )
    )]
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
                        id, workflow_id, state, tenant_id, idempotency_key, started_at,
                        completed_at, error_message, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                    "#,
                )
                .bind(execution.id)
                .bind(execution.workflow_id)
                .bind(&execution.state)
                .bind(&tenant_str)
                .bind(&execution.idempotency_key)
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

    /// Create execution or return existing one if idempotency key matches (atomic)
    ///
    /// This method uses `INSERT ... ON CONFLICT DO NOTHING` to atomically handle
    /// idempotency without race conditions. If an execution with the same
    /// (tenant_id, workflow_id, idempotency_key) already exists, it returns the
    /// existing execution instead of creating a new one.
    ///
    /// # Returns
    ///
    /// Returns `(execution_id, was_created)` where:
    /// - `execution_id`: UUID of the execution (new or existing)
    /// - `was_created`: true if a new execution was created, false if existing was returned
    ///
    /// # Arguments
    ///
    /// * `execution` - The execution model to create
    /// * `tenant_id` - Tenant identifier for RLS enforcement
    #[instrument(
        skip(self, execution, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "executions",
            tenant_id = %tenant_id.as_str(),
            execution_id = %execution.id,
            was_created = tracing::field::Empty
        )
    )]
    pub async fn create_execution_or_get_existing(
        &self,
        execution: &ExecutionModel,
        tenant_id: &TenantId,
    ) -> Result<(Uuid, bool)> {
        Self::validate_execution_state(&execution.state)?;
        let execution = execution.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Attempt to insert with ON CONFLICT DO NOTHING
                // This makes the operation atomic and eliminates TOCTOU race conditions
                let result = sqlx::query(
                    r#"
                    INSERT INTO executions (
                        id, workflow_id, state, tenant_id, idempotency_key, started_at,
                        completed_at, error_message, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                    ON CONFLICT (tenant_id, workflow_id, idempotency_key)
                    WHERE idempotency_key IS NOT NULL
                    DO NOTHING
                    "#,
                )
                .bind(execution.id)
                .bind(execution.workflow_id)
                .bind(&execution.state)
                .bind(&tenant_str)
                .bind(&execution.idempotency_key)
                .bind(execution.started_at)
                .bind(execution.completed_at)
                .bind(&execution.error_message)
                .bind(execution.created_at)
                .bind(execution.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                // Check if the insert was successful (rows_affected > 0)
                if result.rows_affected() > 0 {
                    // New execution was created
                    tracing::info!(
                        execution_id = %execution.id,
                        workflow_id = %execution.workflow_id,
                        "Created new execution"
                    );
                    Ok((execution.id, true))
                } else {
                    // Conflict occurred, fetch the existing execution
                    tracing::info!(
                        workflow_id = %execution.workflow_id,
                        idempotency_key = ?execution.idempotency_key,
                        "Idempotency conflict detected, fetching existing execution"
                    );

                    let existing = sqlx::query_as::<_, ExecutionModel>(
                        r#"
                        SELECT id, workflow_id, state, tenant_id, idempotency_key, started_at, completed_at, error_message, created_at, updated_at
                        FROM executions
                        WHERE workflow_id = $1 AND idempotency_key = $2
                        "#,
                    )
                    .bind(execution.workflow_id)
                    .bind(execution.idempotency_key.as_ref().expect("idempotency_key must be Some if conflict occurred"))
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    Ok((existing.id, false))
                }
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
    #[instrument(
        skip(self, workflow, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "workflows",
            tenant_id = %tenant_id.as_str(),
            workflow_id = %workflow.id
        )
    )]
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
    #[instrument(
        skip(self, execution, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "executions",
            tenant_id = %tenant_id.as_str(),
            execution_id = %execution.id,
            execution_state = %execution.state
        )
    )]
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
                    SELECT id, workflow_id, state, tenant_id, idempotency_key, started_at, completed_at, error_message, created_at, updated_at
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
                    SELECT id, workflow_id, state, tenant_id, idempotency_key, started_at, completed_at, error_message, created_at, updated_at
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
                    SELECT id, workflow_id, state, tenant_id, idempotency_key, started_at, completed_at, error_message, created_at, updated_at
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

    /// Find an execution by workflow_id and idempotency_key
    ///
    /// Returns the existing execution if an idempotency_key matches, or None if not found.
    /// This is used to enforce idempotency for execution creation.
    ///
    /// This operation enforces tenant isolation via RLS policies.
    #[instrument(skip(self, tenant_id, idempotency_key), fields(tenant = %tenant_id.as_str(), workflow_id = %workflow_id))]
    pub async fn find_execution_by_idempotency_key(
        &self,
        workflow_id: Uuid,
        idempotency_key: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<ExecutionModel>> {
        let idempotency_key = idempotency_key.to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let execution = sqlx::query_as::<_, ExecutionModel>(
                    r#"
                    SELECT id, workflow_id, state, tenant_id, idempotency_key, started_at, completed_at, error_message, created_at, updated_at
                    FROM executions
                    WHERE workflow_id = $1 AND idempotency_key = $2
                    "#,
                )
                .bind(workflow_id)
                .bind(&idempotency_key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(execution)
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
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "asset_dependencies",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset_id,
            max_depth = %max_depth,
            assets_visited = tracing::field::Empty
        )
    )]
    pub async fn get_asset_lineage(
        &self,
        asset_id: Uuid,
        max_depth: usize,
        tenant_id: &TenantId,
    ) -> Result<Vec<crate::models::AssetDependencyModel>> {
        use std::collections::{HashSet, VecDeque};

        let mut visited_assets = HashSet::new();
        let mut visited_deps = HashSet::new();
        let mut queue = VecDeque::new();
        let mut lineage = Vec::new();

        queue.push_back((asset_id, 0));
        visited_assets.insert(asset_id);

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            // Get all dependencies (both upstream and downstream) for current asset
            let (upstream, downstream) = self.get_asset_dependencies(current_id, tenant_id).await?;

            // Add upstream dependencies to lineage
            for dep in upstream {
                // Track unique dependencies by (upstream, downstream, type)
                let dep_key = (
                    dep.upstream_asset_id,
                    dep.downstream_asset_id,
                    dep.dependency_type.clone(),
                );
                if visited_deps.insert(dep_key) {
                    lineage.push(dep.clone());
                }

                // Queue unvisited assets for traversal
                if !visited_assets.contains(&dep.upstream_asset_id) {
                    visited_assets.insert(dep.upstream_asset_id);
                    queue.push_back((dep.upstream_asset_id, depth + 1));
                }
            }

            // Add downstream dependencies to lineage
            for dep in downstream {
                // Track unique dependencies by (upstream, downstream, type)
                let dep_key = (
                    dep.upstream_asset_id,
                    dep.downstream_asset_id,
                    dep.dependency_type.clone(),
                );
                if visited_deps.insert(dep_key) {
                    lineage.push(dep.clone());
                }

                // Queue unvisited assets for traversal
                if !visited_assets.contains(&dep.downstream_asset_id) {
                    visited_assets.insert(dep.downstream_asset_id);
                    queue.push_back((dep.downstream_asset_id, depth + 1));
                }
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

    // ========== Data Quality Check Operations ==========

    /// Validate check severity
    fn validate_check_severity(severity: &str) -> Result<()> {
        match severity {
            "info" | "warning" | "error" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid check severity: {}",
                severity
            ))),
        }
    }

    /// Validate check outcome
    fn validate_check_outcome(outcome: &str) -> Result<()> {
        match outcome {
            "passed" | "failed" | "skipped" | "error" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid check outcome: {}",
                outcome
            ))),
        }
    }

    /// Create a new asset check
    #[instrument(
        skip(self, check, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "asset_checks",
            tenant_id = %tenant_id.as_str(),
            check_id = %check.id
        )
    )]
    pub async fn create_asset_check(
        &self,
        check: &AssetCheckModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&check.name, "Check name")?;
        Self::validate_check_severity(&check.severity)?;

        let check = check.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO asset_checks (
                        id, name, description, asset_id, check_type, severity,
                        blocking, enabled, tags, owner, tenant_id, created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                    "#,
                )
                .bind(check.id)
                .bind(&check.name)
                .bind(&check.description)
                .bind(check.asset_id)
                .bind(&check.check_type)
                .bind(&check.severity)
                .bind(check.blocking)
                .bind(check.enabled)
                .bind(&check.tags)
                .bind(&check.owner)
                .bind(&tenant_str)
                .bind(check.created_at)
                .bind(check.updated_at)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Get an asset check by ID
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "asset_checks",
            tenant_id = %tenant_id.as_str(),
            check_id = %id
        )
    )]
    pub async fn get_asset_check(&self, id: Uuid, tenant_id: &TenantId) -> Result<AssetCheckModel> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let check = sqlx::query_as::<_, AssetCheckModel>(
                    r#"
                    SELECT id, name, description, asset_id, check_type, severity,
                           blocking, enabled, tags, owner, tenant_id, created_at, updated_at
                    FROM asset_checks
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Asset check {}", id)))?;

                Ok(check)
            })
        })
        .await
    }

    /// List asset checks for a specific asset
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "asset_checks",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset_id
        )
    )]
    pub async fn list_asset_checks(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
        enabled_only: bool,
    ) -> Result<Vec<AssetCheckModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let checks = if enabled_only {
                    sqlx::query_as::<_, AssetCheckModel>(
                        r#"
                        SELECT id, name, description, asset_id, check_type, severity,
                               blocking, enabled, tags, owner, tenant_id, created_at, updated_at
                        FROM asset_checks
                        WHERE asset_id = $1 AND enabled = true
                        ORDER BY created_at ASC
                        "#,
                    )
                    .bind(asset_id)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(map_db_error)?
                } else {
                    sqlx::query_as::<_, AssetCheckModel>(
                        r#"
                        SELECT id, name, description, asset_id, check_type, severity,
                               blocking, enabled, tags, owner, tenant_id, created_at, updated_at
                        FROM asset_checks
                        WHERE asset_id = $1
                        ORDER BY created_at ASC
                        "#,
                    )
                    .bind(asset_id)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(map_db_error)?
                };

                Ok(checks)
            })
        })
        .await
    }

    /// Update an asset check
    #[instrument(
        skip(self, check, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "asset_checks",
            tenant_id = %tenant_id.as_str(),
            check_id = %check.id
        )
    )]
    pub async fn update_asset_check(
        &self,
        check: &AssetCheckModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&check.name, "Check name")?;
        Self::validate_check_severity(&check.severity)?;

        let check = check.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE asset_checks
                    SET name = $1, description = $2, check_type = $3, severity = $4,
                        blocking = $5, enabled = $6, tags = $7, owner = $8, updated_at = $9
                    WHERE id = $10
                    "#,
                )
                .bind(&check.name)
                .bind(&check.description)
                .bind(&check.check_type)
                .bind(&check.severity)
                .bind(check.blocking)
                .bind(check.enabled)
                .bind(&check.tags)
                .bind(&check.owner)
                .bind(check.updated_at)
                .bind(check.id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Asset check {}", check.id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Delete an asset check
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "DELETE",
            db.sql.table = "asset_checks",
            tenant_id = %tenant_id.as_str(),
            check_id = %id
        )
    )]
    pub async fn delete_asset_check(&self, id: Uuid, tenant_id: &TenantId) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query("DELETE FROM asset_checks WHERE id = $1")
                    .bind(id)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!("Asset check {}", id)));
                }

                Ok(())
            })
        })
        .await
    }

    /// Create a check result
    #[instrument(
        skip(self, result, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "check_results",
            tenant_id = %tenant_id.as_str(),
            result_id = %result.id
        )
    )]
    pub async fn create_check_result(
        &self,
        result: &CheckResultModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_check_outcome(&result.outcome)?;
        Self::validate_check_severity(&result.severity)?;

        let result = result.clone();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO check_results (
                        id, check_id, execution_id, asset_id, outcome, severity, blocking,
                        failed_row_count, total_row_count, error_message, failed_samples,
                        duration_ms, executed_at, metadata, tenant_id
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                    "#,
                )
                .bind(result.id)
                .bind(result.check_id)
                .bind(result.execution_id)
                .bind(result.asset_id)
                .bind(&result.outcome)
                .bind(&result.severity)
                .bind(result.blocking)
                .bind(result.failed_row_count)
                .bind(result.total_row_count)
                .bind(&result.error_message)
                .bind(&result.failed_samples)
                .bind(result.duration_ms)
                .bind(result.executed_at)
                .bind(&result.metadata)
                .bind(&tenant_str)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Create multiple check results in a batch
    #[instrument(
        skip(self, results, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "check_results",
            tenant_id = %tenant_id.as_str(),
            batch_size = results.len()
        )
    )]
    pub async fn create_check_results_batch(
        &self,
        results: &[CheckResultModel],
        tenant_id: &TenantId,
    ) -> Result<()> {
        if results.is_empty() {
            return Ok(());
        }

        // Validate all results
        for result in results {
            Self::validate_check_outcome(&result.outcome)?;
            Self::validate_check_severity(&result.severity)?;
        }

        let results = results.to_vec();
        let tenant_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                for result in results {
                    sqlx::query(
                        r#"
                        INSERT INTO check_results (
                            id, check_id, execution_id, asset_id, outcome, severity, blocking,
                            failed_row_count, total_row_count, error_message, failed_samples,
                            duration_ms, executed_at, metadata, tenant_id
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                        "#,
                    )
                    .bind(result.id)
                    .bind(result.check_id)
                    .bind(result.execution_id)
                    .bind(result.asset_id)
                    .bind(&result.outcome)
                    .bind(&result.severity)
                    .bind(result.blocking)
                    .bind(result.failed_row_count)
                    .bind(result.total_row_count)
                    .bind(&result.error_message)
                    .bind(&result.failed_samples)
                    .bind(result.duration_ms)
                    .bind(result.executed_at)
                    .bind(&result.metadata)
                    .bind(&tenant_str)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;
                }

                Ok(())
            })
        })
        .await
    }

    /// List check results for an execution
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "check_results",
            tenant_id = %tenant_id.as_str(),
            execution_id = %execution_id
        )
    )]
    pub async fn list_check_results_for_execution(
        &self,
        execution_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<CheckResultModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let results = sqlx::query_as::<_, CheckResultModel>(
                    r#"
                    SELECT id, check_id, execution_id, asset_id, outcome, severity, blocking,
                           failed_row_count, total_row_count, error_message, failed_samples,
                           duration_ms, executed_at, metadata, tenant_id
                    FROM check_results
                    WHERE execution_id = $1
                    ORDER BY executed_at ASC
                    "#,
                )
                .bind(execution_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(results)
            })
        })
        .await
    }

    /// Get blocking check failures for an execution
    ///
    /// Returns check results that are blocking, failed, and have error severity
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "check_results",
            tenant_id = %tenant_id.as_str(),
            execution_id = %execution_id
        )
    )]
    pub async fn get_blocking_check_failures(
        &self,
        execution_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<CheckResultModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let results = sqlx::query_as::<_, CheckResultModel>(
                    r#"
                    SELECT id, check_id, execution_id, asset_id, outcome, severity, blocking,
                           failed_row_count, total_row_count, error_message, failed_samples,
                           duration_ms, executed_at, metadata, tenant_id
                    FROM check_results
                    WHERE execution_id = $1
                      AND blocking = true
                      AND outcome IN ('failed', 'error')
                      AND severity = 'error'
                    ORDER BY executed_at ASC
                    "#,
                )
                .bind(execution_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(results)
            })
        })
        .await
    }

    // ========== Backfill Job Operations ==========

    /// Validate backfill job state
    fn validate_backfill_state(state: &str) -> Result<()> {
        match state {
            "pending" | "waiting_upstream" | "running" | "paused" | "resuming" | "completed"
            | "failed" | "cancelled" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid backfill state: {}",
                state
            ))),
        }
    }

    /// Validate backfill partition state
    fn validate_partition_state(state: &str) -> Result<()> {
        match state {
            "pending" | "running" | "completed" | "failed" | "skipped" => Ok(()),
            _ => Err(crate::Error::ValidationError(format!(
                "Invalid partition state: {}",
                state
            ))),
        }
    }

    /// Create a new backfill job
    #[instrument(
        skip(self, job, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %job.id
        )
    )]
    pub async fn create_backfill_job(
        &self,
        job: &BackfillJobModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&job.asset_name, "Asset name")?;
        Self::validate_non_empty_string(&job.idempotency_key, "Idempotency key")?;
        Self::validate_backfill_state(&job.state)?;

        let job = job.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO backfill_jobs (
                        id, tenant_id, asset_id, asset_name, idempotency_key, state,
                        execution_strategy, partition_start, partition_end, partition_keys,
                        total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                        include_upstream, error_message, created_by, created_at, started_at,
                        completed_at, heartbeat_at, version, parent_job_id, max_upstream_depth,
                        upstream_job_count, completed_upstream_jobs, execution_order,
                        sla_deadline_at, priority
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29)
                    "#,
                )
                .bind(job.id)
                .bind(&job.tenant_id)
                .bind(job.asset_id)
                .bind(&job.asset_name)
                .bind(&job.idempotency_key)
                .bind(&job.state)
                .bind(&job.execution_strategy)
                .bind(&job.partition_start)
                .bind(&job.partition_end)
                .bind(&job.partition_keys)
                .bind(job.total_partitions)
                .bind(job.completed_partitions)
                .bind(job.failed_partitions)
                .bind(job.skipped_partitions)
                .bind(job.include_upstream)
                .bind(&job.error_message)
                .bind(&job.created_by)
                .bind(job.created_at)
                .bind(job.started_at)
                .bind(job.completed_at)
                .bind(job.heartbeat_at)
                .bind(job.version)
                .bind(job.parent_job_id)
                .bind(job.max_upstream_depth)
                .bind(job.upstream_job_count)
                .bind(job.completed_upstream_jobs)
                .bind(job.execution_order)
                .bind(job.sla_deadline_at)
                .bind(job.priority)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Get a backfill job by ID
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %id
        )
    )]
    pub async fn get_backfill_job(
        &self,
        id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<BackfillJobModel> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let job = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE id = $1
                    "#,
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Backfill job {}", id)))?;

                Ok(job)
            })
        })
        .await
    }

    /// Update a backfill job
    #[instrument(
        skip(self, job, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %job.id
        )
    )]
    pub async fn update_backfill_job(
        &self,
        job: &BackfillJobModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_backfill_state(&job.state)?;

        let job = job.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = $1, completed_partitions = $2, failed_partitions = $3,
                        skipped_partitions = $4, error_message = $5, started_at = $6,
                        completed_at = $7, heartbeat_at = $8, version = version + 1
                    WHERE id = $9 AND version = $10
                    "#,
                )
                .bind(&job.state)
                .bind(job.completed_partitions)
                .bind(job.failed_partitions)
                .bind(job.skipped_partitions)
                .bind(&job.error_message)
                .bind(job.started_at)
                .bind(job.completed_at)
                .bind(job.heartbeat_at)
                .bind(job.id)
                .bind(job.version)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill job {} (version {})",
                        job.id, job.version
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// List backfill jobs with optional state filter
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str()
        )
    )]
    pub async fn list_backfill_jobs(
        &self,
        tenant_id: &TenantId,
        state_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BackfillJobModel>> {
        Self::validate_pagination_params(limit, offset)?;
        if let Some(state) = state_filter {
            Self::validate_backfill_state(state)?;
        }

        let state_filter = state_filter.map(|s| s.to_string());

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let jobs = if let Some(state) = state_filter {
                    sqlx::query_as::<_, BackfillJobModel>(
                        r#"
                        SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                               execution_strategy, partition_start, partition_end, partition_keys,
                               total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                               include_upstream, error_message, created_by, created_at, started_at,
                               completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                               estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                               max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                               sla_deadline_at, priority
                        FROM backfill_jobs
                        WHERE state = $1
                        ORDER BY created_at DESC
                        LIMIT $2 OFFSET $3
                        "#,
                    )
                    .bind(&state)
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(map_db_error)?
                } else {
                    sqlx::query_as::<_, BackfillJobModel>(
                        r#"
                        SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                               execution_strategy, partition_start, partition_end, partition_keys,
                               total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                               include_upstream, error_message, created_by, created_at, started_at,
                               completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                               estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                               max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                               sla_deadline_at, priority
                        FROM backfill_jobs
                        ORDER BY created_at DESC
                        LIMIT $1 OFFSET $2
                        "#,
                    )
                    .bind(limit)
                    .bind(offset)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(map_db_error)?
                };

                Ok(jobs)
            })
        })
        .await
    }

    /// Atomically claim a pending backfill job for processing.
    /// Uses FOR UPDATE SKIP LOCKED to prevent multiple executors from claiming the same job.
    /// Returns None if no claimable jobs are available.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT + UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            executor_id = %executor_id
        )
    )]
    pub async fn claim_pending_backfill_job(
        &self,
        executor_id: &str,
        stale_heartbeat_threshold_secs: i64,
        tenant_id: &TenantId,
    ) -> Result<Option<BackfillJobModel>> {
        let _executor_id = executor_id; // Reserved for future use (e.g., tracking which executor owns the job)
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // First, try to claim a pending/resuming job, or reclaim a running job with stale heartbeat
                // Note: Jobs in waiting_upstream state are not claimable - they wait for upstream completion
                let job = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE (state = 'pending')
                       OR (state = 'resuming')
                       OR (state = 'running' AND heartbeat_at < $1 - ($2 || ' seconds')::interval)
                    ORDER BY
                        CASE WHEN state = 'pending' THEN 0
                             WHEN state = 'resuming' THEN 1
                             ELSE 2 END,
                        priority DESC,
                        execution_order ASC,
                        created_at ASC
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                    "#,
                )
                .bind(now)
                .bind(stale_heartbeat_threshold_secs)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if let Some(job) = job {
                    // Claim the job by transitioning to running (if pending/resuming) or updating heartbeat
                    let new_state = if job.state == "pending" || job.state == "resuming" { "running" } else { &job.state };
                    let started_at = if job.state == "pending" { Some(now) } else { job.started_at };
                    // Clear paused_at when resuming
                    let clear_paused = job.state == "resuming";

                    sqlx::query(
                        r#"
                        UPDATE backfill_jobs
                        SET state = $1,
                            started_at = COALESCE($2, started_at),
                            heartbeat_at = $3,
                            paused_at = CASE WHEN $6 THEN NULL ELSE paused_at END,
                            error_message = CASE
                                WHEN $4 = 'running' AND state = 'running'
                                THEN 'Reclaimed by executor after stale heartbeat'
                                ELSE error_message
                            END,
                            version = version + 1
                        WHERE id = $5
                        "#,
                    )
                    .bind(new_state)
                    .bind(started_at)
                    .bind(now)
                    .bind(&job.state)
                    .bind(job.id)
                    .bind(clear_paused)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    // Return the updated job
                    let updated_job = sqlx::query_as::<_, BackfillJobModel>(
                        r#"
                        SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                               execution_strategy, partition_start, partition_end, partition_keys,
                               total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                               include_upstream, error_message, created_by, created_at, started_at,
                               completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                               estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                               max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                               sla_deadline_at, priority
                        FROM backfill_jobs
                        WHERE id = $1
                        "#,
                    )
                    .bind(job.id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    Ok(Some(updated_job))
                } else {
                    Ok(None)
                }
            })
        })
        .await
    }

    /// Claim a pending partition for execution within a job.
    /// Uses FOR UPDATE SKIP LOCKED to prevent multiple executors from claiming the same partition.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT + UPDATE",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn claim_pending_partition(
        &self,
        job_id: Uuid,
        max_retries: i32,
        tenant_id: &TenantId,
    ) -> Result<Option<BackfillPartitionModel>> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Claim a pending partition (or failed partition under retry limit)
                let partition = sqlx::query_as::<_, BackfillPartitionModel>(
                    r#"
                    SELECT id, backfill_job_id, partition_key, state, attempt_count,
                           execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                    FROM backfill_partitions
                    WHERE backfill_job_id = $1
                      AND ((state = 'pending')
                           OR (state = 'failed' AND attempt_count < $2))
                    ORDER BY
                        CASE WHEN state = 'pending' THEN 0 ELSE 1 END,
                        partition_key ASC
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                    "#,
                )
                .bind(job_id)
                .bind(max_retries)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if let Some(partition) = partition {
                    // Claim by transitioning to running
                    sqlx::query(
                        r#"
                        UPDATE backfill_partitions
                        SET state = 'running',
                            started_at = $1,
                            attempt_count = attempt_count + 1,
                            error_message = NULL
                        WHERE id = $2
                        "#,
                    )
                    .bind(now)
                    .bind(partition.id)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    // Return updated partition
                    let updated = sqlx::query_as::<_, BackfillPartitionModel>(
                        r#"
                        SELECT id, backfill_job_id, partition_key, state, attempt_count,
                               execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                        FROM backfill_partitions
                        WHERE id = $1
                        "#,
                    )
                    .bind(partition.id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(map_db_error)?;

                    Ok(Some(updated))
                } else {
                    Ok(None)
                }
            })
        })
        .await
    }

    /// Create a backfill partition
    #[instrument(
        skip(self, partition, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            partition_id = %partition.id
        )
    )]
    pub async fn create_backfill_partition(
        &self,
        partition: &BackfillPartitionModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_non_empty_string(&partition.partition_key, "Partition key")?;
        Self::validate_partition_state(&partition.state)?;

        let partition = partition.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    INSERT INTO backfill_partitions (
                        id, backfill_job_id, partition_key, state, attempt_count,
                        execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                    "#,
                )
                .bind(partition.id)
                .bind(partition.backfill_job_id)
                .bind(&partition.partition_key)
                .bind(&partition.state)
                .bind(partition.attempt_count)
                .bind(partition.execution_id)
                .bind(&partition.error_message)
                .bind(partition.duration_ms)
                .bind(partition.started_at)
                .bind(partition.completed_at)
                .bind(&partition.tenant_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Create multiple backfill partitions in a batch
    #[instrument(
        skip(self, partitions, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            batch_size = partitions.len()
        )
    )]
    pub async fn create_backfill_partitions_batch(
        &self,
        partitions: &[BackfillPartitionModel],
        tenant_id: &TenantId,
    ) -> Result<()> {
        if partitions.is_empty() {
            return Ok(());
        }

        for partition in partitions {
            Self::validate_non_empty_string(&partition.partition_key, "Partition key")?;
            Self::validate_partition_state(&partition.state)?;
        }

        let partitions = partitions.to_vec();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                for partition in partitions {
                    sqlx::query(
                        r#"
                        INSERT INTO backfill_partitions (
                            id, backfill_job_id, partition_key, state, attempt_count,
                            execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                        "#,
                    )
                    .bind(partition.id)
                    .bind(partition.backfill_job_id)
                    .bind(&partition.partition_key)
                    .bind(&partition.state)
                    .bind(partition.attempt_count)
                    .bind(partition.execution_id)
                    .bind(&partition.error_message)
                    .bind(partition.duration_ms)
                    .bind(partition.started_at)
                    .bind(partition.completed_at)
                    .bind(&partition.tenant_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(map_db_error)?;
                }

                Ok(())
            })
        })
        .await
    }

    /// Update a backfill partition
    #[instrument(
        skip(self, partition, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            partition_id = %partition.id
        )
    )]
    pub async fn update_backfill_partition(
        &self,
        partition: &BackfillPartitionModel,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_partition_state(&partition.state)?;

        let partition = partition.clone();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = $1, attempt_count = $2, execution_id = $3,
                        error_message = $4, duration_ms = $5, started_at = $6, completed_at = $7
                    WHERE id = $8
                    "#,
                )
                .bind(&partition.state)
                .bind(partition.attempt_count)
                .bind(partition.execution_id)
                .bind(&partition.error_message)
                .bind(partition.duration_ms)
                .bind(partition.started_at)
                .bind(partition.completed_at)
                .bind(partition.id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill partition {}",
                        partition.id
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// List partitions for a backfill job
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %backfill_job_id
        )
    )]
    pub async fn list_backfill_partitions(
        &self,
        backfill_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<BackfillPartitionModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let partitions = sqlx::query_as::<_, BackfillPartitionModel>(
                    r#"
                    SELECT id, backfill_job_id, partition_key, state, attempt_count,
                           execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                    FROM backfill_partitions
                    WHERE backfill_job_id = $1
                    ORDER BY partition_key ASC
                    "#,
                )
                .bind(backfill_job_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(partitions)
            })
        })
        .await
    }

    /// Get failed partitions for a backfill job
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %backfill_job_id
        )
    )]
    pub async fn get_failed_backfill_partitions(
        &self,
        backfill_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<BackfillPartitionModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let partitions = sqlx::query_as::<_, BackfillPartitionModel>(
                    r#"
                    SELECT id, backfill_job_id, partition_key, state, attempt_count,
                           execution_id, error_message, duration_ms, started_at, completed_at, tenant_id
                    FROM backfill_partitions
                    WHERE backfill_job_id = $1 AND state = 'failed'
                    ORDER BY partition_key ASC
                    "#,
                )
                .bind(backfill_job_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(partitions)
            })
        })
        .await
    }

    /// Find backfill job by idempotency key
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str()
        )
    )]
    pub async fn find_backfill_job_by_idempotency_key(
        &self,
        idempotency_key: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<BackfillJobModel>> {
        let idempotency_key = idempotency_key.to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let job = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE idempotency_key = $1
                    "#,
                )
                .bind(&idempotency_key)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(job)
            })
        })
        .await
    }

    /// Find an active (pending, running, paused, or resuming) backfill job for a specific asset/partition.
    /// Used to prevent duplicate backfills while one is still active.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset_id
        )
    )]
    pub async fn find_active_backfill_for_partition(
        &self,
        asset_id: Uuid,
        partition_key: &str,
        tenant_id: &TenantId,
    ) -> Result<Option<BackfillJobModel>> {
        let partition_key = partition_key.to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let job = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE asset_id = $1
                      AND state IN ('pending', 'waiting_upstream', 'running', 'paused', 'resuming')
                      AND partition_keys @> $2::jsonb
                    ORDER BY created_at DESC
                    LIMIT 1
                    "#,
                )
                .bind(asset_id)
                .bind(serde_json::json!([partition_key]))
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(job)
            })
        })
        .await
    }

    /// Validate state transition for backfill jobs.
    /// Allowed transitions:
    ///   pending -> running, waiting_upstream, cancelled
    ///   waiting_upstream -> pending, cancelled
    ///   running -> completed, failed, cancelled, paused
    ///   paused -> resuming, cancelled
    ///   resuming -> running, cancelled
    /// Returns error if transition is not allowed.
    fn validate_backfill_state_transition(from: &str, to: &str) -> Result<()> {
        let valid = matches!(
            (from, to),
            ("pending", "running")
                | ("pending", "waiting_upstream")
                | ("pending", "cancelled")
                | ("waiting_upstream", "pending")
                | ("waiting_upstream", "failed")
                | ("waiting_upstream", "cancelled")
                | ("running", "waiting_upstream")
                | ("running", "completed")
                | ("running", "failed")
                | ("running", "cancelled")
                | ("running", "paused")
                | ("paused", "resuming")
                | ("paused", "cancelled")
                | ("resuming", "running")
                | ("resuming", "cancelled")
        );

        if valid {
            Ok(())
        } else {
            Err(crate::Error::ValidationError(format!(
                "Invalid backfill state transition: {} -> {}",
                from, to
            )))
        }
    }

    /// Update backfill job state with transition validation
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            backfill_job_id = %job_id
        )
    )]
    pub async fn transition_backfill_job_state(
        &self,
        job_id: Uuid,
        from_state: &str,
        to_state: &str,
        error_message: Option<&str>,
        tenant_id: &TenantId,
    ) -> Result<()> {
        Self::validate_backfill_state(from_state)?;
        Self::validate_backfill_state(to_state)?;
        Self::validate_backfill_state_transition(from_state, to_state)?;

        let to_state = to_state.to_string();
        let from_state_owned = from_state.to_string();
        let error_message = error_message.map(|s| s.to_string());
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let completed_at =
                    if to_state == "completed" || to_state == "failed" || to_state == "cancelled" {
                        Some(now)
                    } else {
                        None
                    };

                let started_at = if to_state == "running" {
                    Some(now)
                } else {
                    None
                };

                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = $1,
                        error_message = COALESCE($2, error_message),
                        started_at = COALESCE($3, started_at),
                        completed_at = COALESCE($4, completed_at),
                        heartbeat_at = $5,
                        version = version + 1
                    WHERE id = $6 AND state = $7
                    "#,
                )
                .bind(&to_state)
                .bind(&error_message)
                .bind(started_at)
                .bind(completed_at)
                .bind(now)
                .bind(job_id)
                .bind(&from_state_owned)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill job {} not found in state '{}'",
                        job_id, from_state_owned
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// Cancel a backfill job
    ///
    /// Transitions the job to cancelled state. Only pending or running jobs can be cancelled.
    /// Also marks all pending partitions as skipped.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn cancel_backfill_job(
        &self,
        job_id: Uuid,
        reason: Option<&str>,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let reason = reason.map(|s| s.to_string());
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Get current job state
                let job = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE id = $1
                    FOR UPDATE
                    "#,
                )
                .bind(job_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?
                .ok_or_else(|| crate::Error::NotFound(format!("Backfill job {} not found", job_id)))?;

                // Validate transition - waiting_upstream jobs can also be cancelled
                if job.state != "pending" && job.state != "waiting_upstream" && job.state != "running" && job.state != "paused" && job.state != "resuming" {
                    return Err(crate::Error::ValidationError(format!(
                        "Cannot cancel job in state '{}'. Only pending, waiting_upstream, running, paused, or resuming jobs can be cancelled.",
                        job.state
                    )));
                }

                let error_message = reason.unwrap_or_else(|| "Cancelled by user".to_string());

                // Update job to cancelled
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'cancelled',
                        error_message = $1,
                        completed_at = $2,
                        version = version + 1
                    WHERE id = $3
                    "#,
                )
                .bind(&error_message)
                .bind(now)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                // Mark all pending partitions as skipped
                sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = 'skipped',
                        completed_at = $1,
                        error_message = 'Job cancelled'
                    WHERE backfill_job_id = $2 AND state = 'pending'
                    "#,
                )
                .bind(now)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Check if a backfill job has been cancelled
    ///
    /// Useful for executors to check before processing each partition
    pub async fn is_backfill_job_cancelled(
        &self,
        job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<bool> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let state: Option<(String,)> =
                    sqlx::query_as(r#"SELECT state FROM backfill_jobs WHERE id = $1"#)
                        .bind(job_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(map_db_error)?;

                Ok(state.map(|(s,)| s == "cancelled").unwrap_or(false))
            })
        })
        .await
    }

    /// Check if a backfill job has been paused
    ///
    /// Useful for executors to check before processing each partition
    pub async fn is_backfill_job_paused(&self, job_id: Uuid, tenant_id: &TenantId) -> Result<bool> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let state: Option<(String,)> =
                    sqlx::query_as(r#"SELECT state FROM backfill_jobs WHERE id = $1"#)
                        .bind(job_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(map_db_error)?;

                Ok(state.map(|(s,)| s == "paused").unwrap_or(false))
            })
        })
        .await
    }

    /// Pause a running backfill job
    ///
    /// Transitions the job to paused state. The job will stop processing new partitions
    /// at the next partition boundary. The checkpoint_partition_key is set to the last
    /// completed partition for resumption.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn pause_backfill_job(&self, job_id: Uuid, tenant_id: &TenantId) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Get current job state
                let state: Option<(String,)> =
                    sqlx::query_as(r#"SELECT state FROM backfill_jobs WHERE id = $1 FOR UPDATE"#)
                        .bind(job_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(map_db_error)?;

                let current_state = state.map(|(s,)| s).ok_or_else(|| {
                    crate::Error::NotFound(format!("Backfill job {} not found", job_id))
                })?;

                if current_state != "running" {
                    return Err(crate::Error::ValidationError(format!(
                        "Cannot pause job in state '{}'. Only running jobs can be paused.",
                        current_state
                    )));
                }

                // Get the last completed partition as checkpoint
                let last_completed: Option<(String,)> = sqlx::query_as(
                    r#"
                    SELECT partition_key
                    FROM backfill_partitions
                    WHERE backfill_job_id = $1 AND state = 'completed'
                    ORDER BY completed_at DESC
                    LIMIT 1
                    "#,
                )
                .bind(job_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(map_db_error)?;

                let checkpoint = last_completed.map(|(pk,)| pk);

                // Update job to paused
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'paused',
                        paused_at = $1,
                        checkpoint_partition_key = $2,
                        heartbeat_at = $1,
                        version = version + 1
                    WHERE id = $3
                    "#,
                )
                .bind(now)
                .bind(&checkpoint)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Resume a paused backfill job
    ///
    /// Transitions the job to resuming state. The executor will pick it up
    /// and continue from the checkpoint.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn resume_backfill_job(&self, job_id: Uuid, tenant_id: &TenantId) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Get current job state
                let state: Option<(String,)> =
                    sqlx::query_as(r#"SELECT state FROM backfill_jobs WHERE id = $1 FOR UPDATE"#)
                        .bind(job_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(map_db_error)?;

                let current_state = state.map(|(s,)| s).ok_or_else(|| {
                    crate::Error::NotFound(format!("Backfill job {} not found", job_id))
                })?;

                if current_state != "paused" {
                    return Err(crate::Error::ValidationError(format!(
                        "Cannot resume job in state '{}'. Only paused jobs can be resumed.",
                        current_state
                    )));
                }

                // Update job to resuming (executor will pick it up)
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'resuming',
                        heartbeat_at = $1,
                        version = version + 1
                    WHERE id = $2
                    "#,
                )
                .bind(now)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Update backfill job heartbeat timestamp
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn update_backfill_job_heartbeat(
        &self,
        job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET heartbeat_at = $1
                    WHERE id = $2 AND state = 'running'
                    "#,
                )
                .bind(now)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Update backfill job progress counters
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn update_backfill_job_progress(
        &self,
        job_id: Uuid,
        completed: i32,
        failed: i32,
        skipped: i32,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET completed_partitions = $1,
                        failed_partitions = $2,
                        skipped_partitions = $3,
                        heartbeat_at = $4
                    WHERE id = $5 AND state = 'running'
                    "#,
                )
                .bind(completed)
                .bind(failed)
                .bind(skipped)
                .bind(now)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Update backfill job progress counters with ETA calculation
    ///
    /// This extended version also updates ETA-related fields:
    /// - avg_partition_duration_ms: EWMA of partition execution time
    /// - estimated_completion_at: Calculated from remaining partitions and avg duration
    /// - checkpoint_partition_key: Last completed partition for resumption
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id
        )
    )]
    pub async fn update_backfill_job_progress_with_eta(
        &self,
        job_id: Uuid,
        progress: &BackfillProgressUpdate,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            let progress = progress.clone();
            Box::pin(async move {
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET completed_partitions = $1,
                        failed_partitions = $2,
                        skipped_partitions = $3,
                        heartbeat_at = $4,
                        avg_partition_duration_ms = COALESCE($5, avg_partition_duration_ms),
                        estimated_completion_at = $6,
                        checkpoint_partition_key = COALESCE($7, checkpoint_partition_key)
                    WHERE id = $8 AND state = 'running'
                    "#,
                )
                .bind(progress.completed)
                .bind(progress.failed)
                .bind(progress.skipped)
                .bind(now)
                .bind(progress.avg_partition_duration_ms)
                .bind(progress.estimated_completion_at)
                .bind(&progress.checkpoint_partition_key)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Transition a backfill partition to a new state
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            partition_id = %partition_id
        )
    )]
    pub async fn transition_backfill_partition_state(
        &self,
        partition_id: Uuid,
        from_state: &str,
        to_state: &str,
        error_message: Option<&str>,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let to_state = to_state.to_string();
        let from_state_owned = from_state.to_string();
        let error_message = error_message.map(|s| s.to_string());
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let started_at = if to_state == "running" {
                    Some(now)
                } else {
                    None
                };

                let result = sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = $1,
                        error_message = COALESCE($2, error_message),
                        started_at = COALESCE($3, started_at),
                        attempt_count = attempt_count + CASE WHEN $1 = 'running' THEN 1 ELSE 0 END
                    WHERE id = $4 AND state = $5
                    "#,
                )
                .bind(&to_state)
                .bind(&error_message)
                .bind(started_at)
                .bind(partition_id)
                .bind(&from_state_owned)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill partition {} not found in state '{}'",
                        partition_id, from_state_owned
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// Mark a backfill partition as completed
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            partition_id = %partition_id
        )
    )]
    pub async fn complete_backfill_partition(
        &self,
        partition_id: Uuid,
        execution_id: Option<Uuid>,
        duration_ms: i64,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = 'completed',
                        execution_id = $1,
                        duration_ms = $2,
                        completed_at = $3
                    WHERE id = $4 AND state = 'running'
                    "#,
                )
                .bind(execution_id)
                .bind(duration_ms)
                .bind(now)
                .bind(partition_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill partition {} not found in running state",
                        partition_id
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// Mark a backfill partition as failed
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_partitions",
            tenant_id = %tenant_id.as_str(),
            partition_id = %partition_id
        )
    )]
    pub async fn fail_backfill_partition(
        &self,
        partition_id: Uuid,
        error_message: &str,
        duration_ms: i64,
        tenant_id: &TenantId,
    ) -> Result<()> {
        let now = Utc::now();
        let error_message = error_message.to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = 'failed',
                        error_message = $1,
                        duration_ms = $2,
                        completed_at = $3
                    WHERE id = $4 AND state = 'running'
                    "#,
                )
                .bind(&error_message)
                .bind(duration_ms)
                .bind(now)
                .bind(partition_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Backfill partition {} not found in running state",
                        partition_id
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    // ========== Upstream Propagation Operations ==========

    /// Discover all upstream assets for a given asset within the specified depth limit.
    /// Returns a list of (asset_id, asset_name, depth) tuples in topological order
    /// (furthest upstream first, execution_order 0).
    ///
    /// Uses BFS traversal with cycle detection.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "asset_dependencies",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset_id,
            max_depth = max_depth
        )
    )]
    pub async fn discover_upstream_assets(
        &self,
        asset_id: Uuid,
        max_depth: i32,
        tenant_id: &TenantId,
    ) -> Result<Vec<(Uuid, String, i32)>> {
        use std::collections::{HashMap, HashSet, VecDeque};

        if max_depth == 0 {
            return Ok(Vec::new());
        }

        // Safety limit to prevent runaway discovery
        const MAX_ASSETS: usize = 100;

        // BFS to find all upstream assets within depth limit
        // The visited set prevents cycles from causing infinite loops
        let mut visited: HashSet<Uuid> = HashSet::new();
        let mut queue: VecDeque<(Uuid, i32)> = VecDeque::new();
        let mut upstream_assets: Vec<(Uuid, String, i32)> = Vec::new();
        let mut asset_names: HashMap<Uuid, String> = HashMap::new();

        // Mark the target asset as visited to detect self-cycles
        visited.insert(asset_id);

        // Get all assets for name lookup
        let assets = self.list_assets(tenant_id, 1000, 0).await?;
        for asset in assets {
            asset_names.insert(asset.id, asset.name);
        }

        // Start with the target asset's direct dependencies
        let deps = self.get_upstream_dependencies(asset_id, tenant_id).await?;
        for dep in deps {
            // Check for direct self-cycle (asset depends on itself)
            if dep.upstream_asset_id == asset_id {
                return Err(crate::Error::ValidationError(format!(
                    "Cycle detected: asset {} depends on itself",
                    asset_id
                )));
            }
            if !visited.contains(&dep.upstream_asset_id) {
                visited.insert(dep.upstream_asset_id);
                queue.push_back((dep.upstream_asset_id, 1));
            }
        }

        while let Some((current_asset_id, depth)) = queue.pop_front() {
            // Safety check: limit total discovered assets
            if upstream_assets.len() >= MAX_ASSETS {
                tracing::warn!(
                    asset_id = %asset_id,
                    discovered = upstream_assets.len(),
                    "Upstream discovery hit safety limit of {} assets",
                    MAX_ASSETS
                );
                break;
            }

            let name = asset_names
                .get(&current_asset_id)
                .cloned()
                .unwrap_or_else(|| format!("asset-{}", current_asset_id));
            upstream_assets.push((current_asset_id, name, depth));

            // Continue BFS if within depth limit
            if max_depth < 0 || depth < max_depth {
                let deps = self
                    .get_upstream_dependencies(current_asset_id, tenant_id)
                    .await?;
                for dep in deps {
                    // Note: if dep.upstream_asset_id is already in visited, it means
                    // we've seen this asset before - either the target or another upstream.
                    // This prevents cycles from causing duplicate work but doesn't error
                    // since the same asset appearing at multiple depths is valid in a DAG.
                    if !visited.contains(&dep.upstream_asset_id) {
                        visited.insert(dep.upstream_asset_id);
                        queue.push_back((dep.upstream_asset_id, depth + 1));
                    }
                }
            }
        }

        // Sort by depth descending (furthest upstream first for execution order)
        upstream_assets.sort_by(|a, b| b.2.cmp(&a.2));

        Ok(upstream_assets)
    }

    /// Get summary of upstream jobs for a parent backfill job.
    /// Returns counts of jobs in each state.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn get_upstream_jobs_summary(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<crate::models::UpstreamJobsSummary> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
                    r#"
                    SELECT
                        COUNT(*) as total,
                        COUNT(*) FILTER (WHERE state = 'completed') as completed,
                        COUNT(*) FILTER (WHERE state = 'failed') as failed,
                        COUNT(*) FILTER (WHERE state = 'running') as running,
                        COUNT(*) FILTER (WHERE state IN ('pending', 'waiting_upstream', 'paused', 'resuming')) as pending
                    FROM backfill_jobs
                    WHERE parent_job_id = $1
                    "#,
                )
                .bind(parent_job_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(crate::models::UpstreamJobsSummary {
                    total: row.0 as i32,
                    completed: row.1 as i32,
                    failed: row.2 as i32,
                    running: row.3 as i32,
                    pending: row.4 as i32,
                })
            })
        })
        .await
    }

    /// Check if all upstream jobs are complete for a parent job.
    /// Returns true if all child jobs are in 'completed' state.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn are_upstream_jobs_complete(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<bool> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let row: (i64, i64) = sqlx::query_as(
                    r#"
                    SELECT
                        COUNT(*) as total,
                        COUNT(*) FILTER (WHERE state = 'completed') as completed
                    FROM backfill_jobs
                    WHERE parent_job_id = $1
                    "#,
                )
                .bind(parent_job_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                // All upstream jobs complete if total == completed and total > 0
                // If no upstream jobs, return true (nothing to wait for)
                Ok(row.0 == 0 || row.0 == row.1)
            })
        })
        .await
    }

    /// Get all child jobs for a parent job
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn get_child_backfill_jobs(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<Vec<BackfillJobModel>> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let jobs = sqlx::query_as::<_, BackfillJobModel>(
                    r#"
                    SELECT id, tenant_id, asset_id, asset_name, idempotency_key, state,
                           execution_strategy, partition_start, partition_end, partition_keys,
                           total_partitions, completed_partitions, failed_partitions, skipped_partitions,
                           include_upstream, error_message, created_by, created_at, started_at,
                           completed_at, heartbeat_at, version, paused_at, checkpoint_partition_key,
                           estimated_completion_at, avg_partition_duration_ms, parent_job_id,
                           max_upstream_depth, upstream_job_count, completed_upstream_jobs, execution_order,
                           sla_deadline_at, priority
                    FROM backfill_jobs
                    WHERE parent_job_id = $1
                    ORDER BY execution_order ASC
                    "#,
                )
                .bind(parent_job_id)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(jobs)
            })
        })
        .await
    }

    /// Increment the completed_upstream_jobs counter for a parent job.
    /// Called when a child job completes successfully.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn increment_completed_upstream(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET completed_upstream_jobs = completed_upstream_jobs + 1,
                        version = version + 1
                    WHERE id = $1
                    "#,
                )
                .bind(parent_job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                if result.rows_affected() == 0 {
                    return Err(crate::Error::NotFound(format!(
                        "Parent backfill job {} not found",
                        parent_job_id
                    )));
                }

                Ok(())
            })
        })
        .await
    }

    /// Cancel all jobs in a job tree (parent and all children).
    /// Returns the count of cancelled jobs.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            root_job_id = %root_job_id
        )
    )]
    pub async fn cancel_job_tree(
        &self,
        root_job_id: Uuid,
        reason: &str,
        tenant_id: &TenantId,
    ) -> Result<i32> {
        let reason = reason.to_string();
        let now = Utc::now();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Cancel the root job and all its children in cancellable states
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'cancelled',
                        error_message = $1,
                        completed_at = $2,
                        version = version + 1
                    WHERE (id = $3 OR parent_job_id = $3)
                      AND state IN ('pending', 'waiting_upstream', 'running', 'paused', 'resuming')
                    "#,
                )
                .bind(&reason)
                .bind(now)
                .bind(root_job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                // Also mark all pending partitions as skipped for all affected jobs
                sqlx::query(
                    r#"
                    UPDATE backfill_partitions
                    SET state = 'skipped',
                        completed_at = $1,
                        error_message = 'Job tree cancelled'
                    WHERE backfill_job_id IN (
                        SELECT id FROM backfill_jobs
                        WHERE id = $2 OR parent_job_id = $2
                    )
                    AND state = 'pending'
                    "#,
                )
                .bind(now)
                .bind(root_job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(result.rows_affected() as i32)
            })
        })
        .await
    }

    /// Transition a parent job from waiting_upstream to pending when all upstream jobs complete.
    /// This is called after a child job completes to check if parent is ready to proceed.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn try_transition_parent_to_pending(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<bool> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Atomically check if all upstream are complete and transition parent
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'pending',
                        version = version + 1
                    WHERE id = $1
                      AND state = 'waiting_upstream'
                      AND upstream_job_count > 0
                      AND completed_upstream_jobs >= upstream_job_count
                    "#,
                )
                .bind(parent_job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(result.rows_affected() > 0)
            })
        })
        .await
    }

    /// Propagate child job failure to parent job.
    /// When a child job fails, the parent should be marked as failed since
    /// it cannot proceed without all upstream data.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn fail_parent_on_child_failure(
        &self,
        parent_job_id: Uuid,
        child_job_id: Uuid,
        child_asset_name: &str,
        tenant_id: &TenantId,
    ) -> Result<bool> {
        let error_message = format!(
            "Upstream job {} for asset '{}' failed",
            child_job_id, child_asset_name
        );

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                // Only transition if parent is in waiting_upstream state
                let result = sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET state = 'failed',
                        error_message = $1,
                        completed_at = NOW(),
                        version = version + 1
                    WHERE id = $2
                      AND state = 'waiting_upstream'
                    "#,
                )
                .bind(&error_message)
                .bind(parent_job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(result.rows_affected() > 0)
            })
        })
        .await
    }

    /// Check if any upstream jobs have failed for a parent job.
    /// Returns the count of failed upstream jobs.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %parent_job_id
        )
    )]
    pub async fn count_failed_upstream_jobs(
        &self,
        parent_job_id: Uuid,
        tenant_id: &TenantId,
    ) -> Result<i32> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let row: (i64,) = sqlx::query_as(
                    r#"
                    SELECT COUNT(*) as failed
                    FROM backfill_jobs
                    WHERE parent_job_id = $1
                      AND state IN ('failed', 'cancelled')
                    "#,
                )
                .bind(parent_job_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(row.0 as i32)
            })
        })
        .await
    }

    /// Update the upstream job count for a parent backfill job.
    /// Called after creating child jobs for upstream assets.
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "UPDATE",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            job_id = %job_id,
            count = count
        )
    )]
    pub async fn update_backfill_upstream_count(
        &self,
        job_id: Uuid,
        count: i32,
        tenant_id: &TenantId,
    ) -> Result<()> {
        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                sqlx::query(
                    r#"
                    UPDATE backfill_jobs
                    SET upstream_job_count = $1,
                        version = version + 1
                    WHERE id = $2
                    "#,
                )
                .bind(count)
                .bind(job_id)
                .execute(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(())
            })
        })
        .await
    }

    /// Create a child backfill job for an upstream asset.
    /// The child job will be processed before the parent due to topological ordering.
    #[instrument(
        skip(self, tenant_id, params),
        fields(
            db.system = "postgresql",
            db.operation = "INSERT",
            db.sql.table = "backfill_jobs",
            tenant_id = %tenant_id.as_str(),
            parent_job_id = %params.parent_job_id,
            asset_id = %params.asset_id,
            asset_name = %params.asset_name
        )
    )]
    pub async fn create_upstream_child_job(
        &self,
        params: CreateUpstreamChildJobParams,
        tenant_id: &TenantId,
    ) -> Result<Uuid> {
        let tenant_id_str = tenant_id.as_str().to_string();

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let row: (Uuid,) = sqlx::query_as(
                    r#"
                    INSERT INTO backfill_jobs (
                        tenant_id, asset_id, asset_name, idempotency_key, state,
                        partition_start, partition_end, partition_keys,
                        include_upstream, parent_job_id, execution_order,
                        max_upstream_depth
                    )
                    VALUES ($1, $2, $3, $4, 'pending', $5, $6, '[]'::jsonb, false, $7, $8, 0)
                    RETURNING id
                    "#,
                )
                .bind(&tenant_id_str)
                .bind(params.asset_id)
                .bind(&params.asset_name)
                .bind(&params.idempotency_key)
                .bind(&params.partition_start)
                .bind(&params.partition_end)
                .bind(params.parent_job_id)
                .bind(params.execution_order)
                .fetch_one(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(row.0)
            })
        })
        .await
    }

    /// List recent check results for an asset
    #[instrument(
        skip(self, tenant_id),
        fields(
            db.system = "postgresql",
            db.operation = "SELECT",
            db.sql.table = "check_results",
            tenant_id = %tenant_id.as_str(),
            asset_id = %asset_id
        )
    )]
    pub async fn list_recent_check_results_for_asset(
        &self,
        asset_id: Uuid,
        tenant_id: &TenantId,
        limit: i64,
    ) -> Result<Vec<CheckResultModel>> {
        Self::validate_pagination_params(limit, 0)?;

        self.with_tenant_context(tenant_id, |tx| {
            Box::pin(async move {
                let results = sqlx::query_as::<_, CheckResultModel>(
                    r#"
                    SELECT id, check_id, execution_id, asset_id, outcome, severity, blocking,
                           failed_row_count, total_row_count, error_message, failed_samples,
                           duration_ms, executed_at, metadata, tenant_id
                    FROM check_results
                    WHERE asset_id = $1
                    ORDER BY executed_at DESC
                    LIMIT $2
                    "#,
                )
                .bind(asset_id)
                .bind(limit)
                .fetch_all(&mut **tx)
                .await
                .map_err(map_db_error)?;

                Ok(results)
            })
        })
        .await
    }
}

fn map_db_error(err: sqlx::Error) -> crate::Error {
    // Map connection-related errors
    match &err {
        sqlx::Error::PoolTimedOut => {
            error!(error = %err, "Connection pool timed out");
            return crate::Error::PoolExhausted("Connection pool timed out".to_string());
        }
        sqlx::Error::PoolClosed => {
            error!(error = %err, "Connection pool closed");
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
                    warn!(
                        error_code = code,
                        message = db_err.message(),
                        "Unique constraint violation"
                    );
                    return crate::Error::AlreadyExists(db_err.message().to_string());
                }
                // foreign_key_violation - referenced record doesn't exist
                "23503" => {
                    warn!(
                        error_code = code,
                        message = db_err.message(),
                        "Foreign key violation"
                    );
                    return crate::Error::NotFound(db_err.message().to_string());
                }
                // not_null_violation - required field is null
                "23502" => {
                    warn!(
                        error_code = code,
                        message = db_err.message(),
                        "Not null violation"
                    );
                    return crate::Error::ValidationError(format!(
                        "Required field cannot be null: {}",
                        db_err.message()
                    ));
                }
                // check_violation - CHECK constraint failed
                "23514" => {
                    warn!(
                        error_code = code,
                        message = db_err.message(),
                        "CHECK constraint violation"
                    );
                    return crate::Error::ValidationError(format!(
                        "Constraint violation: {}",
                        db_err.message()
                    ));
                }
                // too_many_connections - connection pool exhausted
                "53300" => {
                    error!(
                        error_code = code,
                        message = db_err.message(),
                        "Database connection limit reached"
                    );
                    return crate::Error::PoolExhausted(db_err.message().to_string());
                }
                // connection_failure - database connection failed
                "08006" | "08001" | "08003" | "08004" => {
                    error!(
                        error_code = code,
                        message = db_err.message(),
                        "Database connection failed"
                    );
                    return crate::Error::ConnectionFailed(db_err.message().to_string());
                }
                _ => {
                    error!(
                        error_code = code,
                        message = db_err.message(),
                        "Unexpected database error"
                    );
                }
            }
        }
    }

    error!(error = %err, "Database error");
    crate::Error::Database(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sqlx::types::Json;

    /// Get test database URL from environment or use default (owner role)
    fn get_test_database_url() -> String {
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://servo:servo@localhost:5432/servo_test".to_string())
    }

    /// Get test application database URL (non-owner, RLS enforced)
    fn get_test_app_database_url() -> String {
        std::env::var("TEST_APP_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://servo_app:servo_app@localhost:5432/servo_test".to_string()
        })
    }

    /// Setup test database with migrations using owner role, then return storage connected as app role
    async fn setup_test_db() -> Result<PostgresStorage> {
        // Owner connection for DDL/migrations
        let owner_url = get_test_database_url();
        let owner_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&owner_url)
            .await?;

        // Drop and recreate schema to ensure clean state across CI runs
        sqlx::query("DROP SCHEMA public CASCADE")
            .execute(&owner_pool)
            .await?;
        sqlx::query("CREATE SCHEMA public")
            .execute(&owner_pool)
            .await?;
        sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
            .execute(&owner_pool)
            .await?;

        // Run migrations (creates RLS policies and servo_app role/grants)
        crate::migrations::run_migrations(&owner_pool).await?;

        // Connect as app role for tests (RLS enforced)
        let app_url = get_test_app_database_url();
        let storage = PostgresStorage::new(&app_url).await?;

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

    #[test]
    fn test_validate_backfill_state_transition_valid() {
        // Valid transitions
        assert!(PostgresStorage::validate_backfill_state_transition("pending", "running").is_ok());
        assert!(
            PostgresStorage::validate_backfill_state_transition("pending", "cancelled").is_ok()
        );
        assert!(
            PostgresStorage::validate_backfill_state_transition("running", "completed").is_ok()
        );
        assert!(PostgresStorage::validate_backfill_state_transition("running", "failed").is_ok());
        assert!(
            PostgresStorage::validate_backfill_state_transition("running", "cancelled").is_ok()
        );
    }

    #[test]
    fn test_validate_backfill_state_transition_invalid() {
        // Invalid transitions
        assert!(
            PostgresStorage::validate_backfill_state_transition("pending", "completed").is_err()
        );
        assert!(PostgresStorage::validate_backfill_state_transition("pending", "failed").is_err());
        assert!(PostgresStorage::validate_backfill_state_transition("running", "pending").is_err());
        assert!(
            PostgresStorage::validate_backfill_state_transition("completed", "running").is_err()
        );
        assert!(PostgresStorage::validate_backfill_state_transition("failed", "running").is_err());
        assert!(
            PostgresStorage::validate_backfill_state_transition("cancelled", "running").is_err()
        );
    }

    #[test]
    fn test_validate_backfill_state_transition_waiting_upstream() {
        // Valid transitions TO waiting_upstream
        // pending -> waiting_upstream: Direct transition for jobs created with include_upstream
        assert!(
            PostgresStorage::validate_backfill_state_transition("pending", "waiting_upstream")
                .is_ok()
        );
        // running -> waiting_upstream: After job is claimed and upstream discovery starts
        assert!(
            PostgresStorage::validate_backfill_state_transition("running", "waiting_upstream")
                .is_ok()
        );

        // Valid transitions FROM waiting_upstream
        // waiting_upstream -> pending: All upstream jobs complete
        assert!(
            PostgresStorage::validate_backfill_state_transition("waiting_upstream", "pending")
                .is_ok()
        );
        // waiting_upstream -> failed: Upstream job failed
        assert!(
            PostgresStorage::validate_backfill_state_transition("waiting_upstream", "failed")
                .is_ok()
        );
        // waiting_upstream -> cancelled: User cancellation
        assert!(PostgresStorage::validate_backfill_state_transition(
            "waiting_upstream",
            "cancelled"
        )
        .is_ok());

        // Invalid transitions involving waiting_upstream
        // Cannot go from waiting_upstream directly to running (must go through pending first)
        assert!(
            PostgresStorage::validate_backfill_state_transition("waiting_upstream", "running")
                .is_err()
        );
        // Cannot complete from waiting_upstream (must complete from running)
        assert!(PostgresStorage::validate_backfill_state_transition(
            "waiting_upstream",
            "completed"
        )
        .is_err());
        // Cannot go back to waiting_upstream from terminal states
        assert!(PostgresStorage::validate_backfill_state_transition(
            "completed",
            "waiting_upstream"
        )
        .is_err());
        assert!(
            PostgresStorage::validate_backfill_state_transition("failed", "waiting_upstream")
                .is_err()
        );
        assert!(PostgresStorage::validate_backfill_state_transition(
            "cancelled",
            "waiting_upstream"
        )
        .is_err());
    }

    #[test]
    fn test_validate_backfill_state() {
        // Valid states
        assert!(PostgresStorage::validate_backfill_state("pending").is_ok());
        assert!(PostgresStorage::validate_backfill_state("running").is_ok());
        assert!(PostgresStorage::validate_backfill_state("waiting_upstream").is_ok());
        assert!(PostgresStorage::validate_backfill_state("paused").is_ok());
        assert!(PostgresStorage::validate_backfill_state("resuming").is_ok());
        assert!(PostgresStorage::validate_backfill_state("completed").is_ok());
        assert!(PostgresStorage::validate_backfill_state("failed").is_ok());
        assert!(PostgresStorage::validate_backfill_state("cancelled").is_ok());

        // Invalid states
        assert!(PostgresStorage::validate_backfill_state("bogus").is_err());
        assert!(PostgresStorage::validate_backfill_state("").is_err());
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
            idempotency_key: None,
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
