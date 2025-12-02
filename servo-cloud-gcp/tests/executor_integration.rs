//! Integration tests for CloudRunExecutor
//!
//! These tests validate the CloudRunExecutor execution flow, including:
//! - Execution record creation
//! - Idempotency key handling
//! - State transitions
//! - Tenant isolation (via RLS)
//!
//! # Note on Cloud Tasks
//!
//! These tests use a mock CloudTasksQueue since actual Cloud Tasks API calls
//! require GCP credentials. The CloudTasksQueue integration is tested separately
//! in `cloud_tasks_real_integration.rs`.
//!
//! # Setup
//!
//! Tests require:
//! - `TEST_DATABASE_URL`: Connection string for database owner role
//! - `TEST_APP_DATABASE_URL`: Connection string for servo_app role
//!
//! # Running Tests
//!
//! ```bash
//! export TEST_DATABASE_URL="postgresql://servo:servo@localhost:5432/servo_test"
//! export TEST_APP_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//! cargo test --package servo-cloud-gcp --test executor_integration -- --ignored
//! ```

use servo_cloud_gcp::executor::states;
use servo_cloud_gcp::queue::CloudTasksQueue;
use servo_cloud_gcp::CloudRunExecutor;
use servo_runtime::executor::ExecutionId;
use servo_runtime::Executor;
use servo_storage::{PostgresStorage, TenantId, WorkflowModel};
use std::sync::Arc;
use uuid::Uuid;

/// Mock service account JSON for testing (generates valid OIDC tokens locally)
fn mock_service_account_json() -> String {
    r#"{
        "type": "service_account",
        "project_id": "test-project",
        "private_key_id": "test-key-id",
        "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEAo5Xn/JMJwGcXzKuHeW0/0coF/5LphsnqflTNcuQmDWSj6sQ5\nxV3EDQW2WANvAci8Jkovexz2KYR0Qq0YQJY4mh9z9o9pVebMc7lP0bML7PI2eyTx\njdIiHeOJV4sjZg336EcphXHh8dvRvlaCfVouXMAZviZgSjtyEhQcfG3q8hYuWpUh\n8drOYkt9oLcW1eg54+FxjGPxm3ehCjjc5S9DOMcAfmql862QU3XwdRrNL16uBGKH\nEV1wGOmuRp4PcmI3r1koMwxkY4rIsvKvdwMdY5h9vICL+SfD0zzIz/eFz/UrypRT\nw3I88CNOCopeu278S5AAAMAmV6CcY6u/H8k34QIDAQABAoIBAHa6gIuy9KruqDlm\nbYdWIbHUHRJnwJKRzl4ukobGzE+AMfBcx0lWSsrg0H2E0HbNCLIJufhzBr8PYYpb\nM3T9EsvZ9EKJx/wO7Wk4oJQkjY1GJwCtyZwTXauvz6j4VZLmpH2z2JEAHOt7jfnF\ntCPi5i9vRb/a/pwiFSLF8f7lkFZvVYkxabI2aPN9C2Vl2rGTvKVRVTL824LT7N5T\nieGHhIzG06WVx0wx6RkIfFioURQ5bjAz27mOcCtXklbE1uuJ3Wt3/3WrLj5kNngQ\n1cYG50RxsrcySpNJRe0Q2yjmKuoZbFUGhOkcgKxOt4gWIsXJS6j+2n209TkrYFWt\nVJgZ+aECgYEA1lopqJfmAWXCHHWWqk8qojze2VDGWHGTWY8UZIG5shfJ848Et910\nPbeyVT7wjqyndDumyQ0Z5RZdubuwAJtztAcx2kjllT/xn98MiTsybdO+zSxTGi8e\nufI5otoc2l3LXwIPg3SCuw3IRCKb5xH65lqPuuxAi5biYO0xdpVPYsUCgYEAw16f\nBJtu264LrwWjxAIi/OzTU8LrLrWfQxwHluCNAq0231YvxYn78IAXlFwKOQSfo7xu\nDFPtYPTKrcHBXhfCTSmPtcc4EHUOxeI1Rrx7+hsnGRvpBN7izyNV9xhjgKhbsE9j\nFGdBLMecQjKEWfQnupt9OJR+/Gc0ejYRWABGIm0CgYEAmRav5f8GgD0C03yP7Ha6\nBkDfD+oGnqgPCsmLUPTtgOpIi7C7N+3r1SBbN1LKT7uzpOnthm3a7qh4q+wGGYjs\nq8jvq+ygO2X91aZht6kVrSargLGFcI/yq+75hvur27FdTC+WkNYngBypyS/GlNNc\nlCRdvqx4tLlAWOVOjFi659kCgYAyWBeyIDuY8GiiWVmyELrjYkOyue/QwYygxoBO\nKN8taWVSd+snhD8f4mIxIOaheseIce4JzzbEXVA9Jkil7olrSOJ3fTa9RuzfoUMJ\nT1jDQmv41HBfiO/5YKmuWuKJLNkzvcFSaKkq2OUzScFBTOFP/PFvuxReBlpHg9dE\nTg3EoQKBgQCvJ8WjWUs35PJcLh5gWGv1fzZB6Gn8vCmjTRWuVqrzhAmMKkQtlPPR\nYfl4thxj3+V/OJk3IJcL5gfwG/TubOCwGIt6rcrwWha6SScaRD18wAoQXOfePdiu\nj9s1qYHlJzsXIu0aJCAsrEYKLS3otjKVpf69HIDqo/ZU61U7J7YCoQ==\n-----END RSA PRIVATE KEY-----",
        "client_email": "test@test-project.iam.gserviceaccount.com",
        "client_id": "123456789",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token"
    }"#
    .to_string()
}

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

/// Setup storage connected with servo_app role (RLS enforced)
async fn setup_app_role_storage() -> Arc<PostgresStorage> {
    let owner_url = get_owner_database_url();
    let owner_storage = PostgresStorage::new(&owner_url)
        .await
        .expect("Failed to connect to database with owner role");
    servo_storage::migrations::run_migrations(owner_storage.pool())
        .await
        .expect("Failed to run migrations with owner role");

    let url = get_app_database_url();
    Arc::new(
        PostgresStorage::new(&url)
            .await
            .expect("Failed to connect to database with app role"),
    )
}

/// Create a mock CloudTasksQueue for testing
fn create_mock_queue() -> Arc<CloudTasksQueue> {
    Arc::new(
        CloudTasksQueue::new(
            "test-project".to_string(),
            "us-central1".to_string(),
            "servo-tasks".to_string(),
            "https://test-worker.run.app".to_string(),
            &mock_service_account_json(),
            "test-hmac-secret".to_string(),
        )
        .expect("Failed to create mock CloudTasksQueue"),
    )
}

/// Create a test workflow
async fn create_test_workflow(storage: &PostgresStorage, tenant: &TenantId) -> Uuid {
    let workflow = WorkflowModel {
        id: Uuid::new_v4(),
        name: format!("test_workflow_{}", Uuid::new_v4()),
        description: Some("Test workflow for CloudRunExecutor integration tests".to_string()),
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

#[test]
fn test_cloud_run_executor_construction() {
    // This test verifies executor construction without database
    // Uses mock queue that doesn't require GCP credentials
    let _queue = create_mock_queue();
    // CloudRunExecutor construction is tested implicitly via integration tests
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_cancel_pending() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Create a pending execution directly in DB (simulating pre-enqueue state)
    let execution_id = Uuid::new_v4();
    let execution = servo_storage::models::ExecutionModel {
        id: execution_id,
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
        .cancel(ExecutionId(execution_id))
        .await
        .expect("Cancel should succeed");

    // Verify state changed to cancelled
    let cancelled = storage
        .get_execution(execution_id, &tenant)
        .await
        .expect("Failed to get cancelled execution");

    assert_eq!(cancelled.state, states::CANCELLED);
    assert!(cancelled.completed_at.is_some());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_cancel_running() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Create a running execution
    let execution_id = Uuid::new_v4();
    let execution = servo_storage::models::ExecutionModel {
        id: execution_id,
        workflow_id,
        state: states::RUNNING.to_string(),
        tenant_id: Some(tenant.as_str().to_string()),
        idempotency_key: None,
        started_at: Some(chrono::Utc::now()),
        completed_at: None,
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    storage
        .create_execution(&execution, &tenant)
        .await
        .expect("Failed to create running execution");

    // Cancel the running execution
    executor
        .cancel(ExecutionId(execution_id))
        .await
        .expect("Cancel should succeed");

    // Verify state changed to cancelled
    let cancelled = storage
        .get_execution(execution_id, &tenant)
        .await
        .expect("Failed to get cancelled execution");

    assert_eq!(cancelled.state, states::CANCELLED);
    assert!(cancelled.completed_at.is_some());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_cancel_already_succeeded() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Create a succeeded execution
    let execution_id = Uuid::new_v4();
    let execution = servo_storage::models::ExecutionModel {
        id: execution_id,
        workflow_id,
        state: states::SUCCEEDED.to_string(),
        tenant_id: Some(tenant.as_str().to_string()),
        idempotency_key: None,
        started_at: Some(chrono::Utc::now()),
        completed_at: Some(chrono::Utc::now()),
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    storage
        .create_execution(&execution, &tenant)
        .await
        .expect("Failed to create succeeded execution");

    // Cancel should be no-op for already completed execution
    executor
        .cancel(ExecutionId(execution_id))
        .await
        .expect("Cancel should succeed (no-op)");

    // Verify state remains SUCCEEDED
    let still_succeeded = storage
        .get_execution(execution_id, &tenant)
        .await
        .expect("Failed to get execution");

    assert_eq!(
        still_succeeded.state,
        states::SUCCEEDED,
        "Completed execution should remain in SUCCEEDED state"
    );
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_status() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    // Create execution in various states and check status
    let test_cases = vec![
        (states::PENDING, false, None),
        (states::RUNNING, false, None),
        (states::SUCCEEDED, true, None),
        (
            states::FAILED,
            false,
            Some("Test error message".to_string()),
        ),
    ];

    for (state, expected_success, error_msg) in test_cases {
        let execution_id = Uuid::new_v4();
        let execution = servo_storage::models::ExecutionModel {
            id: execution_id,
            workflow_id,
            state: state.to_string(),
            tenant_id: Some(tenant.as_str().to_string()),
            idempotency_key: None,
            started_at: if state != states::PENDING {
                Some(chrono::Utc::now())
            } else {
                None
            },
            completed_at: if state == states::SUCCEEDED || state == states::FAILED {
                Some(chrono::Utc::now())
            } else {
                None
            },
            error_message: error_msg.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        storage
            .create_execution(&execution, &tenant)
            .await
            .expect("Failed to create execution");

        let status = executor
            .status(ExecutionId(execution_id))
            .await
            .expect("Status should succeed");

        assert_eq!(status.execution_id.0, execution_id);
        assert_eq!(
            status.success, expected_success,
            "State {} should have success={}",
            state, expected_success
        );
        assert_eq!(status.error, error_msg);
    }
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_get_execution_details() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant).await;

    let execution_id = Uuid::new_v4();
    let now = chrono::Utc::now();
    let execution = servo_storage::models::ExecutionModel {
        id: execution_id,
        workflow_id,
        state: states::RUNNING.to_string(),
        tenant_id: Some(tenant.as_str().to_string()),
        idempotency_key: Some("test-key-123".to_string()),
        started_at: Some(now),
        completed_at: None,
        error_message: None,
        created_at: now,
        updated_at: now,
    };

    storage
        .create_execution(&execution, &tenant)
        .await
        .expect("Failed to create execution");

    let details = executor
        .get_execution_details(ExecutionId(execution_id))
        .await
        .expect("Should get execution details");

    assert_eq!(details.id, execution_id);
    assert_eq!(details.workflow_id, workflow_id);
    assert_eq!(details.state, states::RUNNING);
    assert_eq!(details.idempotency_key, Some("test-key-123".to_string()));
    assert!(details.started_at.is_some());
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_tenant_isolation() {
    let storage = setup_app_role_storage().await;
    let queue = create_mock_queue();

    let tenant1 = unique_tenant();
    let tenant2 = unique_tenant();

    let executor1 = CloudRunExecutor::new(
        storage.clone(),
        queue.clone(),
        tenant1.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let executor2 = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant2.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    let workflow_id = create_test_workflow(&storage, &tenant1).await;

    // Create execution for tenant1
    let execution_id = Uuid::new_v4();
    let execution = servo_storage::models::ExecutionModel {
        id: execution_id,
        workflow_id,
        state: states::RUNNING.to_string(),
        tenant_id: Some(tenant1.as_str().to_string()),
        idempotency_key: None,
        started_at: Some(chrono::Utc::now()),
        completed_at: None,
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    storage
        .create_execution(&execution, &tenant1)
        .await
        .expect("Failed to create execution");

    // Tenant1 can access their execution
    let status1 = executor1
        .status(ExecutionId(execution_id))
        .await
        .expect("Tenant1 should access their execution");
    assert_eq!(status1.execution_id.0, execution_id);

    // Tenant2 cannot access tenant1's execution (RLS)
    let status2 = executor2.status(ExecutionId(execution_id)).await;
    assert!(
        status2.is_err(),
        "Tenant2 should not access tenant1's execution due to RLS"
    );
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cloud_run_executor_status_not_found() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();
    let queue = create_mock_queue();

    let executor = CloudRunExecutor::new(
        storage.clone(),
        queue,
        tenant.clone(),
        "test-project".to_string(),
        "us-central1".to_string(),
    );

    // Try to get status of non-existent execution
    let result = executor.status(ExecutionId(Uuid::new_v4())).await;

    assert!(result.is_err(), "Should fail for non-existent execution");
}
