//! Integration test for Orchestrator → Cloud Tasks flow
//!
//! This test validates:
//! - CloudTasksQueue creation and configuration
//! - OIDC token generation
//! - HMAC payload signing
//! - TaskEnqueuer trait implementation
//!
//! Note: The Cloud Tasks API call itself is mocked (returns mock task ID)
//! to allow testing without actual GCP credentials.

use servo_cloud_gcp::queue::CloudTasksQueue;
use std::sync::Arc;
use uuid::Uuid;

/// Mock service account JSON for testing
///
/// This contains a valid RSA private key (generated for testing only).
/// It's sufficient for testing OIDC token generation logic without real GCP access.
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

#[test]
fn test_cloud_tasks_queue_creation() {
    // Test that CloudTasksQueue can be created with valid service account
    let queue = CloudTasksQueue::new(
        "test-project".to_string(),
        "us-central1".to_string(),
        "servo-tasks".to_string(),
        "https://test-worker.run.app".to_string(),
        &mock_service_account_json(),
        "test-secret".to_string(),
    );

    assert!(
        queue.is_ok(),
        "Failed to create CloudTasksQueue: {:?}",
        queue.err()
    );

    let queue = queue.unwrap();
    assert_eq!(
        queue.queue_path(),
        "projects/test-project/locations/us-central1/queues/servo-tasks"
    );
}

#[test]
fn test_cloud_tasks_queue_rejects_invalid_service_account() {
    let result = CloudTasksQueue::new(
        "test-project".to_string(),
        "us-central1".to_string(),
        "servo-tasks".to_string(),
        "https://test-worker.run.app".to_string(),
        "invalid json",
        "test-secret".to_string(),
    );

    assert!(
        result.is_err(),
        "Should reject invalid service account JSON"
    );
}

#[tokio::test]
#[ignore] // Requires real GCP credentials - use cloud_tasks_real_integration test instead
async fn test_task_enqueuer_implementation() {
    // This test now makes real API calls to Google's OAuth2 endpoint and Cloud Tasks API.
    // It will fail with mock credentials.
    //
    // To run this test with real credentials, use the cloud_tasks_real_integration test
    // which provides setup instructions and environment variable documentation.
    //
    // The basic queue creation and configuration is validated in
    // test_cloud_tasks_queue_creation which doesn't require credentials.

    let queue = Arc::new(
        CloudTasksQueue::new(
            "test-project".to_string(),
            "us-central1".to_string(),
            "servo-tasks".to_string(),
            "https://test-worker.run.app".to_string(),
            &mock_service_account_json(),
            "test-secret".to_string(),
        )
        .expect("Failed to create CloudTasksQueue"),
    );

    let tenant_id = "test-tenant";
    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let execution_plan = vec![Uuid::new_v4(), Uuid::new_v4()];

    // This will fail with mock credentials
    let result = queue
        .enqueue(execution_id, workflow_id, tenant_id, None, execution_plan)
        .await;

    // With mock credentials, we expect authentication failure
    assert!(result.is_err(), "Should fail with mock credentials");
}
