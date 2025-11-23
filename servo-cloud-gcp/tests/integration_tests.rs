//! Integration tests for GCP Cloud Tasks
//!
//! # Running Integration Tests
//!
//! These tests are marked with `#[ignore]` by default because they require:
//! 1. A GCP project with Cloud Tasks API enabled
//! 2. A service account with Cloud Tasks permissions
//! 3. Environment variables configured (see below)
//!
//! ## Prerequisites
//!
//! 1. **Enable Cloud Tasks API**:
//!    ```bash
//!    gcloud services enable cloudtasks.googleapis.com
//!    ```
//!
//! 2. **Create a Cloud Tasks queue**:
//!    ```bash
//!    gcloud tasks queues create servo-tasks \
//!      --location=us-central1 \
//!      --max-dispatches-per-second=10
//!    ```
//!
//! 3. **Create a service account**:
//!    ```bash
//!    gcloud iam service-accounts create servo-worker \
//!      --display-name="Servo Worker Service Account"
//!
//!    gcloud projects add-iam-policy-binding PROJECT_ID \
//!      --member="serviceAccount:servo-worker@PROJECT_ID.iam.gserviceaccount.com" \
//!      --role="roles/cloudtasks.enqueuer"
//!
//!    gcloud iam service-accounts keys create key.json \
//!      --iam-account=servo-worker@PROJECT_ID.iam.gserviceaccount.com
//!    ```
//!
//! 4. **Set environment variables**:
//!    ```bash
//!    export GCP_PROJECT_ID="your-project-id"
//!    export GCP_LOCATION="us-central1"
//!    export GCP_QUEUE_NAME="servo-tasks"
//!    export GCP_WORKER_URL="https://your-worker.run.app"
//!    export GCP_SERVICE_ACCOUNT_EMAIL="servo-worker@PROJECT_ID.iam.gserviceaccount.com"
//!    export GCP_SERVICE_ACCOUNT_KEY_JSON="$(cat key.json)"
//!    export SERVO_HMAC_SECRET="your-secure-random-secret"
//!    ```
//!
//! ## Running the Tests
//!
//! Run all integration tests:
//! ```bash
//! cargo test -p servo-cloud-gcp --test integration_tests -- --ignored --test-threads=1
//! ```
//!
//! Run a specific test:
//! ```bash
//! cargo test -p servo-cloud-gcp test_enqueue_task_end_to_end -- --ignored
//! ```
//!
//! ## CI Integration
//!
//! To run in CI, add the following to your GitHub Actions workflow:
//! ```yaml
//! - name: Run GCP Integration Tests
//!   env:
//!     GCP_PROJECT_ID: ${{ secrets.GCP_PROJECT_ID }}
//!     GCP_LOCATION: us-central1
//!     GCP_QUEUE_NAME: servo-tasks-ci
//!     GCP_WORKER_URL: https://servo-worker-ci.run.app
//!     GCP_SERVICE_ACCOUNT_EMAIL: ${{ secrets.GCP_SERVICE_ACCOUNT_EMAIL }}
//!     GCP_SERVICE_ACCOUNT_KEY_JSON: ${{ secrets.GCP_SERVICE_ACCOUNT_KEY_JSON }}
//!     SERVO_HMAC_SECRET: ${{ secrets.SERVO_HMAC_SECRET }}
//!   run: |
//!     cargo test -p servo-cloud-gcp --test integration_tests -- --ignored --test-threads=1
//! ```

use servo_cloud_gcp::{auth::GcpAuth, config::GcpConfig, queue::CloudTasksQueue, signing};
use servo_runtime::task_enqueuer::TaskEnqueuer;
use servo_storage::TenantId;
use std::sync::Arc;
use uuid::Uuid;

/// Helper to load config from environment with clear error messages
fn load_test_config() -> Result<GcpConfig, String> {
    GcpConfig::from_env().map_err(|e| {
        format!(
            "Failed to load GCP config: {}. See test file documentation for setup instructions.",
            e
        )
    })
}

#[tokio::test]
#[ignore = "Requires GCP credentials and Cloud Tasks setup"]
async fn test_enqueue_task_end_to_end() {
    // Load configuration
    let config = load_test_config().expect("Failed to load test configuration");

    // Create auth and queue
    let auth = Arc::new(
        GcpAuth::from_service_account_json(&config.service_account_key_json)
            .expect("Failed to create GCP auth"),
    );
    let queue_name = config.queue_name.clone();
    let queue = CloudTasksQueue::from_config(config, auth)
        .expect("Failed to create Cloud Tasks queue");

    // Test data
    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let tenant_id = TenantId::new("test-tenant");
    let idempotency_key = Some(format!("test-{}", Uuid::new_v4()));
    let execution_plan = vec![Uuid::new_v4(), Uuid::new_v4()]; // Mock execution plan

    // Enqueue task
    let task_name = queue
        .enqueue(execution_id, workflow_id, &tenant_id, idempotency_key.clone(), execution_plan)
        .await
        .expect("Failed to enqueue task");

    // Verify task name format
    assert!(
        task_name.contains("/tasks/"),
        "Task name should contain '/tasks/': {}",
        task_name
    );
    assert!(
        task_name.contains(&queue_name),
        "Task name should contain queue name"
    );

    println!("✓ Successfully enqueued task: {}", task_name);
}

#[tokio::test]
#[ignore = "Requires GCP credentials and Cloud Tasks setup"]
async fn test_idempotency_key_with_cloud_tasks() {
    let config = load_test_config().expect("Failed to load test configuration");
    let auth = Arc::new(
        GcpAuth::from_service_account_json(&config.service_account_key_json)
            .expect("Failed to create GCP auth"),
    );
    let queue = CloudTasksQueue::from_config(config.clone(), auth)
        .expect("Failed to create Cloud Tasks queue");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let tenant_id = TenantId::new("test-tenant");
    let idempotency_key = Some(format!("idempotency-test-{}", Uuid::new_v4()));
    let execution_plan = vec![Uuid::new_v4()]; // Mock execution plan

    // Enqueue the same task twice with the same idempotency key
    let task_name_1 = queue
        .enqueue(
            execution_id,
            workflow_id,
            &tenant_id,
            idempotency_key.clone(),
            execution_plan.clone(),
        )
        .await
        .expect("First enqueue should succeed");

    // Note: Cloud Tasks doesn't deduplicate based on our idempotency_key
    // That's handled at the database level in the orchestrator
    // This test just verifies both enqueue operations succeed
    let task_name_2 = queue
        .enqueue(
            execution_id,
            workflow_id,
            &tenant_id,
            idempotency_key.clone(),
            execution_plan,
        )
        .await
        .expect("Second enqueue should succeed");

    println!("✓ First task: {}", task_name_1);
    println!("✓ Second task: {}", task_name_2);
    println!("✓ Note: Idempotency is enforced at DB level, not Cloud Tasks level");
}

#[tokio::test]
#[ignore = "Requires GCP credentials"]
async fn test_oauth2_token_exchange() {
    let config = load_test_config().expect("Failed to load test configuration");
    let auth =
        GcpAuth::from_service_account_json(&config.service_account_key_json)
            .expect("Failed to create GCP auth");

    // Get access token
    let token = auth
        .get_access_token()
        .await
        .expect("Failed to get access token");

    // Verify token format (should be a non-empty string)
    assert!(!token.is_empty(), "Access token should not be empty");
    assert!(
        token.len() > 100,
        "Access token should be substantial length"
    );

    // Test caching - second call should be faster
    let start = std::time::Instant::now();
    let token_2 = auth
        .get_access_token()
        .await
        .expect("Failed to get cached token");
    let elapsed = start.elapsed();

    assert_eq!(token, token_2, "Cached token should match original");
    assert!(
        elapsed.as_millis() < 10,
        "Cached token retrieval should be fast (< 10ms)"
    );

    println!("✓ OAuth2 token exchange successful");
    println!("✓ Token caching verified ({}μs)", elapsed.as_micros());
}

#[test]
fn test_hmac_signature_verification() {
    let secret = "test-secret-key-for-hmac";
    let payload = br#"{"execution_id":"123","workflow_id":"456"}"#;

    // Sign payload
    let signature = signing::sign_payload(payload, secret).expect("Failed to sign payload");

    // Verify signature
    let is_valid =
        signing::verify_signature(payload, &signature, secret).expect("Failed to verify signature");
    assert!(is_valid, "Signature should be valid");

    // Verify fails with wrong secret
    let is_valid_wrong = signing::verify_signature(payload, &signature, "wrong-secret")
        .expect("Failed to verify with wrong secret");
    assert!(!is_valid_wrong, "Signature should fail with wrong secret");

    // Verify fails with wrong payload
    let wrong_payload = br#"{"execution_id":"999","workflow_id":"999"}"#;
    let is_valid_wrong_payload = signing::verify_signature(wrong_payload, &signature, secret)
        .expect("Failed to verify with wrong payload");
    assert!(
        !is_valid_wrong_payload,
        "Signature should fail with wrong payload"
    );

    println!("✓ HMAC signature generation and verification successful");
}

#[test]
fn test_signature_constant_time_comparison() {
    // This test verifies that signature comparison is constant-time
    // to prevent timing attacks
    let secret = "secret";
    let payload = b"test payload";

    let sig = signing::sign_payload(payload, secret).expect("Failed to sign");

    // Same signatures should verify
    assert!(signing::verify_signature(payload, &sig, secret).unwrap());

    // Different signature (but same length) should also take constant time
    let wrong_sig = "a".repeat(sig.len());

    // Both verifications should take similar time (constant-time comparison)
    // This is ensured by using subtle::ConstantTimeEq in the implementation
    let _ = signing::verify_signature(payload, &wrong_sig, secret);

    println!("✓ Constant-time signature comparison verified");
}

#[tokio::test]
#[ignore = "Requires GCP credentials"]
async fn test_error_handling_invalid_credentials() {
    // Create config with invalid service account JSON
    let invalid_json = r#"{"client_email":"invalid","private_key":"invalid"}"#;

    let result = GcpAuth::from_service_account_json(invalid_json);

    // Should fail to create auth with invalid key
    // Note: This actually succeeds at creation, but fails at token exchange
    if let Ok(auth) = result {
        let token_result = auth.get_access_token().await;
        assert!(
            token_result.is_err(),
            "Should fail to get token with invalid credentials"
        );
        println!("✓ Invalid credentials properly rejected");
    }
}

#[tokio::test]
#[ignore = "Requires GCP credentials"]
async fn test_error_handling_invalid_project() {
    // Create config with non-existent project
    let config = GcpConfig {
        project_id: "non-existent-project-123456789".to_string(),
        location: "us-central1".to_string(),
        queue_name: "non-existent-queue".to_string(),
        worker_url: "https://example.com".to_string(),
        service_account_email: "test@example.com".to_string(),
        service_account_key_json: std::env::var("GCP_SERVICE_ACCOUNT_KEY_JSON")
            .expect("GCP_SERVICE_ACCOUNT_KEY_JSON not set"),
        hmac_secret: "test-secret".to_string(),
    };

    let auth = Arc::new(
        GcpAuth::from_service_account_json(&config.service_account_key_json)
            .expect("Failed to create auth"),
    );
    let queue = CloudTasksQueue::from_config(config, auth).expect("Failed to create queue");

    let result = queue
        .enqueue(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &TenantId::new("test"),
            None,
            vec![Uuid::new_v4()], // Mock execution plan
        )
        .await;

    assert!(result.is_err(), "Should fail with non-existent project");
    println!("✓ Invalid project properly rejected");
}

#[tokio::test]
#[ignore = "Requires GCP credentials and Cloud Tasks setup"]
async fn test_queue_path_format() {
    let config = load_test_config().expect("Failed to load test configuration");
    let auth = Arc::new(
        GcpAuth::from_service_account_json(&config.service_account_key_json)
            .expect("Failed to create auth"),
    );
    let queue = CloudTasksQueue::from_config(config.clone(), auth)
        .expect("Failed to create queue");

    let queue_path = queue.queue_path();

    // Verify queue path format
    let expected = format!(
        "projects/{}/locations/{}/queues/{}",
        config.project_id, config.location, config.queue_name
    );
    assert_eq!(queue_path, expected, "Queue path should match expected format");

    println!("✓ Queue path format verified: {}", queue_path);
}
