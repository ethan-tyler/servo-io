//! Integration tests for LocalExecutor
//!
//! These tests validate the LocalExecutor against a real PostgreSQL database
//! with Row-Level Security (RLS) enforcement using the servo_app role.
//!
//! # Setup
//!
//! Tests require two environment variables:
//! - `TEST_DATABASE_URL`: Connection string for database owner role (for setup/teardown)
//! - `TEST_APP_DATABASE_URL`: Connection string for servo_app role (for RLS-enforced operations)
//!
//! # Running Tests
//!
//! ```bash
//! export TEST_DATABASE_URL="postgresql://servo:servo@localhost:5432/servo_test"
//! export TEST_APP_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//! cargo test --package servo-runtime --test local_executor_integration -- --ignored
//! ```

use servo_runtime::executor::ExecutionId;
use servo_runtime::local_executor::states;
use servo_runtime::{Executor, LocalExecutor, LocalExecutorConfig};
use servo_storage::{AssetModel, PostgresStorage, TenantId, WorkflowModel};
use std::sync::Arc;
use uuid::Uuid;

/// Get the app role database URL for RLS-enforced operations
fn get_app_database_url() -> String {
    std::env::var("TEST_APP_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://servo_app:servo_app@localhost:5432/servo_test".to_string()
    })
}

/// Generate a unique tenant ID for test isolation
fn unique_tenant() -> TenantId {
    TenantId::new(format!("test_{}", Uuid::new_v4()))
}

/// Get the owner database URL (for migrations)
fn get_owner_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://servo:servo@localhost:5432/servo_test".to_string())
}

/// Setup storage connected with servo_app role (RLS enforced) and ensure migrations are applied
async fn setup_app_role_storage() -> Arc<PostgresStorage> {
    // Apply migrations using owner role to ensure schema exists
    let owner_url = get_owner_database_url();
    let owner_storage = PostgresStorage::new(&owner_url)
        .await
        .expect("Failed to connect to database with owner role");
    servo_storage::migrations::run_migrations(owner_storage.pool())
        .await
        .expect("Failed to run migrations with owner role");

    // Connect as app role for tests (RLS enforced)
    let url = get_app_database_url();
    Arc::new(
        PostgresStorage::new(&url)
            .await
            .expect("Failed to connect to database with app role"),
    )
}

/// Create a test workflow for execution
async fn create_test_workflow(storage: &PostgresStorage, tenant: &TenantId) -> Uuid {
    let workflow = WorkflowModel {
        id: Uuid::new_v4(),
        name: format!("test_workflow_{}", Uuid::new_v4()),
        description: Some("Test workflow for LocalExecutor integration tests".to_string()),
        owner: Some("test_user".to_string()),
        tags: sqlx::types::Json(vec![]),
        tenant_id: Some(tenant.as_str().to_string()),
        version: 1,
        schedule_cron: None,
        schedule_timezone: None,
        schedule_enabled: None,
        scheduler_job_name: None,
        last_scheduled_run: None,
        next_scheduled_run: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    storage
        .create_workflow(&workflow, tenant)
        .await
        .expect("Failed to create test workflow");

    workflow.id
}

#[tokio::test]
#[ignore]
async fn test_local_executor_execute() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Create a test workflow
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute the workflow
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    // Verify result
    assert!(result.success, "Simulated execution should succeed");
    assert!(result.error.is_none(), "Should have no error");

    // Verify execution record was created and completed
    let execution = storage
        .get_execution(result.execution_id.0, &tenant)
        .await
        .expect("Failed to get execution");

    assert_eq!(execution.state, states::SUCCEEDED);
    assert!(execution.started_at.is_some());
    assert!(execution.completed_at.is_some());
    assert!(execution.error_message.is_none());
}

#[tokio::test]
#[ignore]
async fn test_local_executor_with_custom_config() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    config.timeout_seconds = 60;
    config
        .env_vars
        .insert("TEST_VAR".to_string(), "test_value".to_string());

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(result.success);
}

#[tokio::test]
#[ignore]
async fn test_local_executor_idempotency() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    let workflow_id = create_test_workflow(&storage, &tenant).await;
    let idempotency_key = format!("test-key-{}", Uuid::new_v4());

    // First execution with idempotency key
    let result1 = executor
        .execute_with_idempotency(
            servo_core::WorkflowId(workflow_id),
            Some(idempotency_key.clone()),
        )
        .await
        .expect("First execution should succeed");

    // Second execution with same idempotency key
    let result2 = executor
        .execute_with_idempotency(
            servo_core::WorkflowId(workflow_id),
            Some(idempotency_key.clone()),
        )
        .await
        .expect("Second execution should return existing");

    // Should return same execution ID
    assert_eq!(
        result1.execution_id.0, result2.execution_id.0,
        "Same idempotency key should return same execution ID"
    );

    // Verify only one execution exists
    let executions = storage
        .list_workflow_executions(workflow_id, &tenant, 100, 0)
        .await
        .expect("Failed to list executions");

    assert_eq!(
        executions.len(),
        1,
        "Should have only one execution with idempotency key"
    );
}

#[tokio::test]
#[ignore]
async fn test_local_executor_cancel() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute workflow first (not used directly but shows the flow)
    let _completed_result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    // Since LocalExecutor executes synchronously, the execution is already
    // completed by the time execute() returns. Test cancellation on a new
    // execution that hasn't been executed yet.

    // Create a pending execution via direct DB insert for cancel testing
    let pending_exec_id = Uuid::new_v4();
    let execution = servo_storage::models::ExecutionModel {
        id: pending_exec_id,
        workflow_id,
        state: states::PENDING.to_string(),
        tenant_id: Some(tenant.as_str().to_string()),
        idempotency_key: None,
        started_at: None,
        completed_at: None,
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    storage
        .create_execution(&execution, &tenant)
        .await
        .expect("Failed to create pending execution");

    // Cancel the pending execution
    executor
        .cancel(ExecutionId(pending_exec_id))
        .await
        .expect("Cancel should succeed");

    // Verify execution was cancelled
    let cancelled_exec = storage
        .get_execution(pending_exec_id, &tenant)
        .await
        .expect("Failed to get cancelled execution");

    assert_eq!(cancelled_exec.state, states::CANCELLED);
    assert!(cancelled_exec.completed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn test_local_executor_cancel_already_completed() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute workflow (completes synchronously)
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    // Try to cancel completed execution
    executor
        .cancel(result.execution_id)
        .await
        .expect("Cancel should succeed (no-op for completed)");

    // Verify execution is still in SUCCEEDED state (not changed to cancelled)
    let execution = storage
        .get_execution(result.execution_id.0, &tenant)
        .await
        .expect("Failed to get execution");

    assert_eq!(
        execution.state,
        states::SUCCEEDED,
        "Completed execution should remain in SUCCEEDED state"
    );
}

#[tokio::test]
#[ignore]
async fn test_local_executor_status() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute workflow
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    // Check status
    let status = executor
        .status(result.execution_id)
        .await
        .expect("Status check should succeed");

    assert_eq!(status.execution_id.0, result.execution_id.0);
    assert!(status.success);
    assert!(status.error.is_none());
}

#[tokio::test]
#[ignore]
async fn test_local_executor_get_execution_details() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    // Get detailed execution info
    let details = executor
        .get_execution_details(result.execution_id)
        .await
        .expect("Should get execution details");

    assert_eq!(details.id, result.execution_id.0);
    assert_eq!(details.workflow_id, workflow_id);
    assert_eq!(details.state, states::SUCCEEDED);
    assert!(details.started_at.is_some());
    assert!(details.completed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn test_local_executor_tenant_isolation() {
    let storage = setup_app_role_storage().await;

    let tenant1 = unique_tenant();
    let tenant2 = unique_tenant();

    let executor1 = LocalExecutor::new(storage.clone(), tenant1.clone());
    let executor2 = LocalExecutor::new(storage.clone(), tenant2.clone());

    // Create workflow and execute for tenant1
    let workflow_id = create_test_workflow(&storage, &tenant1).await;
    let result1 = executor1
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed for tenant1");

    // Tenant2 should NOT be able to see tenant1's execution
    let status_result = executor2.status(result1.execution_id).await;
    assert!(
        status_result.is_err(),
        "Tenant2 should not be able to access tenant1's execution due to RLS"
    );

    // Tenant1 CAN access their own execution
    let status = executor1
        .status(result1.execution_id)
        .await
        .expect("Tenant1 should access their own execution");
    assert!(status.success);
}

#[tokio::test]
#[ignore]
async fn test_local_executor_workflow_not_found() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Try to execute non-existent workflow
    let result = executor
        .execute(servo_core::WorkflowId(Uuid::new_v4()))
        .await;

    // The execution record gets created, but run_workflow_locally fails
    // when it tries to get the workflow
    assert!(result.is_err(), "Should fail for non-existent workflow");
}

// =============================================================================
// Python Execution Configuration Tests
// =============================================================================
//
// Note: Full Python execution integration tests require workflow-asset linking
// which is handled via the deploy command. These tests validate the executor
// configuration and behavior with workflows that have no assets (empty execution).

#[tokio::test]
#[ignore]
async fn test_executor_with_custom_timeout() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    config.timeout_seconds = 60;

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute should succeed (empty workflow)
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(
        result.success,
        "Execution with custom timeout should succeed"
    );
}

#[tokio::test]
#[ignore]
async fn test_executor_with_custom_python_path() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    config.python_path = std::path::PathBuf::from("python3");

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute should succeed (empty workflow)
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(
        result.success,
        "Execution with custom python_path should succeed"
    );
}

#[tokio::test]
#[ignore]
async fn test_executor_with_env_vars() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    config
        .env_vars
        .insert("MY_CUSTOM_VAR".to_string(), "custom_value".to_string());
    config
        .env_vars
        .insert("ANOTHER_VAR".to_string(), "another_value".to_string());

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute should succeed (empty workflow)
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(result.success, "Execution with env vars should succeed");
}

#[tokio::test]
#[ignore]
async fn test_executor_empty_workflow_succeeds() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Create workflow with no assets
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Execute should succeed (empty workflow = no work to do)
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(result.success, "Empty workflow execution should succeed");

    // Verify state in DB
    let execution = storage
        .get_execution(result.execution_id.0, &tenant)
        .await
        .expect("Should get execution");

    assert_eq!(execution.state, "succeeded");
}

// =============================================================================
// Python Execution Integration Tests
// =============================================================================
//
// These tests validate end-to-end Python execution via the LocalExecutor.
// They require:
// 1. PostgreSQL database with RLS (TEST_DATABASE_URL, TEST_APP_DATABASE_URL)
// 2. Python3 available in PATH
// 3. PYTHONPATH set to include the test fixtures directory
//
// Run with:
// ```bash
// export PYTHONPATH="$PWD/servo-runtime/tests/fixtures:$PYTHONPATH"
// cargo test --package servo-runtime --test local_executor_integration -- --ignored
// ```

/// Create an asset linked to a workflow for testing
async fn create_linked_asset(
    storage: &PostgresStorage,
    tenant: &TenantId,
    workflow_id: Uuid,
    asset_name: &str,
    compute_fn_module: Option<&str>,
    compute_fn_function: Option<&str>,
    position: i32,
) -> Uuid {
    let asset_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    // Create the asset
    let asset = AssetModel {
        id: asset_id,
        name: asset_name.to_string(),
        description: Some(format!("Test asset for {}", asset_name)),
        asset_type: "table".to_string(),
        owner: Some("test_user".to_string()),
        tags: sqlx::types::Json(vec!["test".to_string()]),
        partition_config: None,
        compute_fn_module: compute_fn_module.map(|s| s.to_string()),
        compute_fn_function: compute_fn_function.map(|s| s.to_string()),
        tenant_id: Some(tenant.as_str().to_string()),
        created_at: now,
        updated_at: now,
    };

    storage
        .create_asset(&asset, tenant)
        .await
        .expect("Failed to create test asset");

    // Link asset to workflow via workflow_assets table
    // Use raw SQL since there's no storage method for this
    sqlx::query(
        r#"
        INSERT INTO workflow_assets (workflow_id, asset_id, position)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(workflow_id)
    .bind(asset_id)
    .bind(position)
    .execute(storage.pool())
    .await
    .expect("Failed to link asset to workflow");

    asset_id
}

#[tokio::test]
#[ignore]
async fn test_python_execution_success() {
    // This test requires PYTHONPATH to include servo-runtime/tests/fixtures
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Configure executor with PYTHONPATH
    let mut config = LocalExecutorConfig::default();
    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");

    let existing_pythonpath = std::env::var("PYTHONPATH").unwrap_or_default();
    let new_pythonpath = if existing_pythonpath.is_empty() {
        fixtures_path.to_string_lossy().to_string()
    } else {
        format!(
            "{}:{}",
            fixtures_path.to_string_lossy(),
            existing_pythonpath
        )
    };
    config
        .env_vars
        .insert("PYTHONPATH".to_string(), new_pythonpath);

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    // Create workflow with a Python asset
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "test_asset",
        Some("test_module"),
        Some("success_function"),
        0,
    )
    .await;

    // Execute the workflow
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(result.success, "Python execution should succeed");
    assert!(result.error.is_none(), "Should have no error");

    // Verify execution state in DB
    let execution = storage
        .get_execution(result.execution_id.0, &tenant)
        .await
        .expect("Failed to get execution");

    assert_eq!(execution.state, states::SUCCEEDED);
    assert!(execution.started_at.is_some());
    assert!(execution.completed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn test_python_execution_env_vars() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");

    let existing_pythonpath = std::env::var("PYTHONPATH").unwrap_or_default();
    let new_pythonpath = if existing_pythonpath.is_empty() {
        fixtures_path.to_string_lossy().to_string()
    } else {
        format!(
            "{}:{}",
            fixtures_path.to_string_lossy(),
            existing_pythonpath
        )
    };
    config
        .env_vars
        .insert("PYTHONPATH".to_string(), new_pythonpath);
    config
        .env_vars
        .insert("CUSTOM_TEST_VAR".to_string(), "custom_value".to_string());

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "env_test_asset",
        Some("test_module"),
        Some("env_vars_function"),
        0,
    )
    .await;

    // The function will succeed if it can access env vars
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(
        result.success,
        "Execution with env vars should succeed: {:?}",
        result.error
    );
}

#[tokio::test]
#[ignore]
async fn test_python_execution_missing_compute_fn_module() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Create workflow with an asset that has NO compute_fn_module
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "missing_module_asset",
        None, // No module!
        Some("some_function"),
        0,
    )
    .await;

    // Execute should fail with a clear error message
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    assert!(result.is_err(), "Should fail without compute_fn_module");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("no compute_fn_module") || err_msg.contains("compute_fn_module"),
        "Error should mention missing compute_fn_module: {}",
        err_msg
    );
}

#[tokio::test]
#[ignore]
async fn test_python_execution_missing_compute_fn_function() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Create workflow with an asset that has NO compute_fn_function
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "missing_function_asset",
        Some("test_module"),
        None, // No function!
        0,
    )
    .await;

    // Execute should fail with a clear error message
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    assert!(result.is_err(), "Should fail without compute_fn_function");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("no compute_fn_function") || err_msg.contains("compute_fn_function"),
        "Error should mention missing compute_fn_function: {}",
        err_msg
    );
}

#[tokio::test]
#[ignore]
async fn test_python_execution_invalid_module() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let executor = LocalExecutor::new(storage.clone(), tenant.clone());

    // Create workflow with an asset that references a non-existent module
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "invalid_module_asset",
        Some("nonexistent_module_xyz"),
        Some("some_function"),
        0,
    )
    .await;

    // Execute should fail because module cannot be imported
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    assert!(result.is_err(), "Should fail with invalid module");
    let err_msg = result.unwrap_err().to_string();
    // Python will fail to import the module
    assert!(
        err_msg.contains("failed") || err_msg.contains("exit code"),
        "Error should indicate execution failure: {}",
        err_msg
    );
}

#[tokio::test]
#[ignore]
async fn test_python_execution_function_error() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");

    config.env_vars.insert(
        "PYTHONPATH".to_string(),
        fixtures_path.to_string_lossy().to_string(),
    );

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    // Create workflow with an asset that raises an exception
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "error_asset",
        Some("test_module"),
        Some("failure_function"),
        0,
    )
    .await;

    // Execute should fail because function raises an exception
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    assert!(
        result.is_err(),
        "Should fail when function raises exception"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("failed") || err_msg.contains("Intentional test failure"),
        "Error should contain failure message: {}",
        err_msg
    );

    // Verify execution state shows failure
    // Note: The execution record is created, so we need to find it
    let executions = storage
        .list_workflow_executions(workflow_id, &tenant, 10, 0)
        .await
        .expect("Should list executions");

    assert_eq!(executions.len(), 1, "Should have one execution");
    assert_eq!(executions[0].state, states::FAILED);
    assert!(executions[0].error_message.is_some());
}

#[tokio::test]
#[ignore]
async fn test_python_execution_timeout() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Configure a very short timeout
    let mut config = LocalExecutorConfig::default();
    config.timeout_seconds = 1; // 1 second timeout

    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");
    config.env_vars.insert(
        "PYTHONPATH".to_string(),
        fixtures_path.to_string_lossy().to_string(),
    );

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    // Create workflow with a slow function (sleeps for 10s)
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "slow_asset",
        Some("test_module"),
        Some("slow_function"),
        0,
    )
    .await;

    // Execute should fail with timeout
    let start = std::time::Instant::now();
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    let elapsed = start.elapsed();

    assert!(result.is_err(), "Should fail with timeout");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.to_lowercase().contains("timeout") || err_msg.to_lowercase().contains("timed out"),
        "Error should mention timeout: {}",
        err_msg
    );

    // Verify it didn't wait the full 10 seconds
    assert!(
        elapsed.as_secs() < 5,
        "Should timeout quickly, not wait for slow function (elapsed: {:?})",
        elapsed
    );
}

#[tokio::test]
#[ignore]
async fn test_python_execution_multiple_assets_in_order() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");
    config.env_vars.insert(
        "PYTHONPATH".to_string(),
        fixtures_path.to_string_lossy().to_string(),
    );

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    // Create workflow with multiple assets
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "first_asset",
        Some("test_module"),
        Some("success_function"),
        0, // Position 0
    )
    .await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "second_asset",
        Some("test_module"),
        Some("output_function"),
        1, // Position 1
    )
    .await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "third_asset",
        Some("test_module"),
        Some("success_function"),
        2, // Position 2
    )
    .await;

    // Execute the workflow - all assets should succeed in order
    let result = executor
        .execute(servo_core::WorkflowId(workflow_id))
        .await
        .expect("Execution should succeed");

    assert!(result.success, "Multi-asset workflow should succeed");

    // Verify execution state
    let execution = storage
        .get_execution(result.execution_id.0, &tenant)
        .await
        .expect("Failed to get execution");

    assert_eq!(execution.state, states::SUCCEEDED);
}

#[tokio::test]
#[ignore]
async fn test_python_execution_stops_on_first_failure() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    let mut config = LocalExecutorConfig::default();
    let fixtures_path = std::env::current_dir()
        .expect("Failed to get current dir")
        .join("servo-runtime/tests/fixtures");
    config.env_vars.insert(
        "PYTHONPATH".to_string(),
        fixtures_path.to_string_lossy().to_string(),
    );

    let executor = LocalExecutor::with_config(storage.clone(), tenant.clone(), config);

    // Create workflow where second asset fails
    let workflow_id = create_test_workflow(&storage, &tenant).await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "first_success",
        Some("test_module"),
        Some("success_function"),
        0,
    )
    .await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "second_fails",
        Some("test_module"),
        Some("failure_function"),
        1,
    )
    .await;
    create_linked_asset(
        &storage,
        &tenant,
        workflow_id,
        "third_never_runs",
        Some("test_module"),
        Some("success_function"),
        2,
    )
    .await;

    // Execute should fail at second asset
    let result = executor.execute(servo_core::WorkflowId(workflow_id)).await;

    assert!(result.is_err(), "Workflow should fail when an asset fails");

    // Verify execution state shows failure
    let executions = storage
        .list_workflow_executions(workflow_id, &tenant, 10, 0)
        .await
        .expect("Should list executions");

    assert_eq!(executions.len(), 1);
    assert_eq!(executions[0].state, states::FAILED);
}
