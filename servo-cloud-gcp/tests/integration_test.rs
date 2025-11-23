//! Integration tests for Cloud Tasks enqueue (requires GCP credentials).
//!
//! These tests are marked #[ignore] because they require:
//! - A GCP project with Cloud Tasks API enabled
//! - A service account with roles/cloudtasks.enqueuer
//! - An existing Cloud Tasks queue
//! - Env vars: GCP_PROJECT_ID, GCP_LOCATION, GCP_QUEUE_NAME, GCP_WORKER_URL,
//!   GCP_SERVICE_ACCOUNT_KEY_JSON, SERVO_HMAC_SECRET
//!
//! Run with:
//! ```bash
//! export GCP_PROJECT_ID=your-project
//! export GCP_LOCATION=us-central1
//! export GCP_QUEUE_NAME=servo-tasks
//! export GCP_WORKER_URL=https://your-worker-url
//! export GCP_SERVICE_ACCOUNT_KEY_JSON="$(cat key.json)"
//! export SERVO_HMAC_SECRET="$(openssl rand -base64 32)"
//! cargo test -p servo-cloud-gcp --test integration_test -- --ignored
//! ```

use servo_cloud_gcp::auth::GcpAuth;
use servo_cloud_gcp::config::GcpConfig;
use servo_cloud_gcp::queue::CloudTasksQueue;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
#[ignore]
async fn test_enqueue_task_to_cloud_tasks() {
    let cfg = GcpConfig::from_env().expect("GCP env vars not set");
    let auth = Arc::new(
        GcpAuth::from_service_account_json(&cfg.service_account_key_json)
            .expect("Invalid service account json"),
    );
    let queue = CloudTasksQueue::from_config(cfg, auth).expect("Failed to build CloudTasksQueue");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let tenant_id = "test-tenant";
    let execution_plan = vec![Uuid::new_v4(), Uuid::new_v4()]; // Mock execution plan

    let task_name = queue
        .enqueue_task(execution_id, workflow_id, tenant_id, None, execution_plan)
        .await
        .expect("Failed to enqueue task");

    assert!(
        task_name.contains("/tasks/"),
        "Expected task name to include /tasks/, got {}",
        task_name
    );
}
