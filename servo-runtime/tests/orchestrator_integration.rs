//! Integration tests for ExecutionOrchestrator
//!
//! These tests validate the orchestrator against a real PostgreSQL database
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
//! cargo test --package servo-runtime --test orchestrator_integration -- --ignored
//! ```

use servo_runtime::{ExecutionOrchestrator, ExecutionState, RetryPolicy};
use servo_storage::{PostgresStorage, TenantId, WorkflowModel};
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
        description: Some("Test workflow for integration tests".to_string()),
        owner: Some("test_user".to_string()),
        tags: sqlx::types::Json(vec![]),
        tenant_id: Some(tenant.as_str().to_string()),
        version: 1,
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
async fn test_execution_lifecycle_with_rls() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    // Create a test workflow
    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Start execution
    let exec_id = orchestrator
        .start_execution(workflow_id, &tenant, None)
        .await
        .expect("Failed to start execution");

    // Verify initial state is Pending
    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "pending");
    assert!(execution.started_at.is_none());
    assert!(execution.completed_at.is_none());

    // Transition to Running
    orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Pending,
            ExecutionState::Running,
            &tenant,
        )
        .await
        .expect("Failed to transition to Running");

    // Verify state persisted and started_at is set
    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "running");
    assert!(execution.started_at.is_some());
    assert!(execution.completed_at.is_none());

    // Complete successfully
    orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Running,
            ExecutionState::Succeeded,
            &tenant,
        )
        .await
        .expect("Failed to transition to Succeeded");

    // Verify final state
    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "succeeded");
    assert!(execution.completed_at.is_some());
    assert!(execution.error_message.is_none());
}

#[tokio::test]
#[ignore]
async fn test_tenant_isolation_enforced_by_rls() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());

    let tenant1 = unique_tenant();
    let tenant2 = unique_tenant();

    // Create workflow and execution for tenant1
    let wf_id = create_test_workflow(&storage, &tenant1).await;
    let exec_id = orchestrator
        .start_execution(wf_id, &tenant1, None)
        .await
        .expect("Failed to start execution for tenant1");

    // Tenant2 CANNOT access tenant1's execution (RLS blocks it)
    let result = storage.get_execution(exec_id, &tenant2).await;
    assert!(
        result.is_err(),
        "Tenant2 should not be able to access tenant1's execution due to RLS"
    );

    // Tenant1 CAN access their own execution
    let result = storage.get_execution(exec_id, &tenant1).await;
    assert!(
        result.is_ok(),
        "Tenant1 should be able to access their own execution"
    );

    // Tenant2 cannot transition tenant1's execution
    let result = orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Pending,
            ExecutionState::Running,
            &tenant2,
        )
        .await;
    assert!(
        result.is_err(),
        "Tenant2 should not be able to transition tenant1's execution"
    );
}

#[tokio::test]
#[ignore]
async fn test_invalid_state_transition_rejected() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    let wf_id = create_test_workflow(&storage, &tenant).await;
    let exec_id = orchestrator
        .start_execution(wf_id, &tenant, None)
        .await
        .expect("Failed to start execution");

    // Try invalid transition: Pending -> Succeeded (must go through Running)
    let result = orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Pending,
            ExecutionState::Succeeded,
            &tenant,
        )
        .await;

    assert!(
        result.is_err(),
        "Invalid state transition should be rejected"
    );

    // Verify state hasn't changed
    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "pending");
}

#[tokio::test]
#[ignore]
async fn test_state_mismatch_rejected() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    let wf_id = create_test_workflow(&storage, &tenant).await;
    let exec_id = orchestrator
        .start_execution(wf_id, &tenant, None)
        .await
        .expect("Failed to start execution");

    // Execution is in Pending state, but we claim it's Running
    let result = orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Running, // Wrong!
            ExecutionState::Succeeded,
            &tenant,
        )
        .await;

    assert!(result.is_err(), "State mismatch should be rejected");
}

#[tokio::test]
#[ignore]
async fn test_record_failure() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    let wf_id = create_test_workflow(&storage, &tenant).await;
    let exec_id = orchestrator
        .start_execution(wf_id, &tenant, None)
        .await
        .expect("Failed to start execution");

    // Transition to Running
    orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Pending,
            ExecutionState::Running,
            &tenant,
        )
        .await
        .expect("Failed to transition to Running");

    // Record failure
    let error_message = "Database connection timeout";
    orchestrator
        .record_failure(exec_id, error_message, &tenant)
        .await
        .expect("Failed to record failure");

    // Verify execution marked as failed with error message
    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "failed");
    assert_eq!(execution.error_message, Some(error_message.to_string()));
    assert!(execution.completed_at.is_some());
}

#[tokio::test]
#[ignore]
async fn test_retry_transition_allowed() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    let wf_id = create_test_workflow(&storage, &tenant).await;
    let exec_id = orchestrator
        .start_execution(wf_id, &tenant, None)
        .await
        .expect("Failed to start execution");

    // Go to Running
    orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Pending,
            ExecutionState::Running,
            &tenant,
        )
        .await
        .expect("Failed to transition to Running");

    // Fail
    orchestrator
        .record_failure(exec_id, "Transient error", &tenant)
        .await
        .expect("Failed to record failure");

    // Retry: Failed -> Running should be allowed
    let result = orchestrator
        .transition_state(
            exec_id,
            ExecutionState::Failed,
            ExecutionState::Running,
            &tenant,
        )
        .await;

    assert!(
        result.is_ok(),
        "Retry transition (Failed -> Running) should be allowed"
    );

    let execution = storage
        .get_execution(exec_id, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "running");
}

#[tokio::test]
#[ignore]
async fn test_concurrent_executions() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    // Create workflow
    let wf_id = create_test_workflow(&storage, &tenant).await;

    // Start 10 executions concurrently
    let mut handles = vec![];
    for _ in 0..10 {
        let orch = orchestrator.clone();
        let wf = wf_id;
        let t = tenant.clone();
        let handle = tokio::spawn(async move { orch.start_execution(wf, &t, None).await });
        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles).await;

    // Verify all succeeded and got unique execution IDs
    let mut exec_ids = std::collections::HashSet::new();
    for result in results {
        let exec_id = result
            .expect("Task panicked")
            .expect("Failed to start execution");
        assert!(exec_ids.insert(exec_id), "Duplicate execution ID detected");
    }

    assert_eq!(exec_ids.len(), 10, "Should have 10 unique executions");

    // Verify all executions are in database
    for exec_id in exec_ids {
        let execution = storage
            .get_execution(exec_id, &tenant)
            .await
            .expect("Failed to get execution");
        assert_eq!(execution.state, "pending");
    }
}

#[tokio::test]
#[ignore]
async fn test_concurrent_state_transitions() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    // Create 5 executions
    let wf_id = create_test_workflow(&storage, &tenant).await;
    let mut exec_ids = vec![];
    for _ in 0..5 {
        let exec_id = orchestrator
            .start_execution(wf_id, &tenant, None)
            .await
            .expect("Failed to start execution");
        exec_ids.push(exec_id);
    }

    // Transition all to Running concurrently
    let mut handles: Vec<tokio::task::JoinHandle<servo_runtime::Result<()>>> = vec![];
    for exec_id in exec_ids.iter() {
        let orch = orchestrator.clone();
        let id = *exec_id;
        let t = tenant.clone();
        let handle = tokio::spawn(async move {
            orch.transition_state(id, ExecutionState::Pending, ExecutionState::Running, &t)
                .await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles).await;

    // Verify all transitions succeeded
    for (idx, result) in results.iter().enumerate() {
        assert!(
            result.is_ok() && result.as_ref().unwrap().is_ok(),
            "Transition {} failed",
            idx
        );
    }

    // Verify all are now in Running state
    for exec_id in exec_ids {
        let execution = storage
            .get_execution(exec_id, &tenant)
            .await
            .expect("Failed to get execution");
        assert_eq!(execution.state, "running");
        assert!(execution.started_at.is_some());
    }
}

#[tokio::test]
#[ignore]
async fn test_idempotency_enforcement() {
    let storage = setup_app_role_storage().await;
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = unique_tenant();

    let wf_id = create_test_workflow(&storage, &tenant).await;

    // Start execution with idempotency key
    let exec_id_1 = orchestrator
        .start_execution(wf_id, &tenant, Some("test-key-123".to_string()))
        .await
        .expect("Should succeed with idempotency key");

    // Verify execution was created
    let execution = storage
        .get_execution(exec_id_1, &tenant)
        .await
        .expect("Failed to get execution");
    assert_eq!(execution.state, "pending");
    assert_eq!(execution.idempotency_key, Some("test-key-123".to_string()));

    // Try to create another execution with the same idempotency key
    // Should return the existing execution_id instead of creating a new one
    let exec_id_2 = orchestrator
        .start_execution(wf_id, &tenant, Some("test-key-123".to_string()))
        .await
        .expect("Should return existing execution for duplicate idempotency key");

    // Verify both IDs are the same (idempotency enforced)
    assert_eq!(
        exec_id_1, exec_id_2,
        "Duplicate idempotency key should return same execution ID"
    );

    // Verify only one execution exists
    let executions = storage
        .list_workflow_executions(wf_id, &tenant, 100, 0)
        .await
        .expect("Failed to list executions");
    assert_eq!(
        executions.len(),
        1,
        "Should have only one execution despite duplicate key"
    );

    // Different idempotency key should create a new execution
    let exec_id_3 = orchestrator
        .start_execution(wf_id, &tenant, Some("different-key-456".to_string()))
        .await
        .expect("Should succeed with different idempotency key");

    assert_ne!(
        exec_id_1, exec_id_3,
        "Different idempotency key should create new execution"
    );

    // No idempotency key should also create a new execution
    let exec_id_4 = orchestrator
        .start_execution(wf_id, &tenant, None)
        .await
        .expect("Should succeed without idempotency key");

    assert_ne!(
        exec_id_1, exec_id_4,
        "No idempotency key should create new execution"
    );
}
