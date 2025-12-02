//! Real Cloud Tasks integration test (requires GCP project setup)
//!
//! This test validates the end-to-end Cloud Tasks integration with a real GCP project.
//!
//! # Prerequisites
//!
//! 1. **GCP Project Setup:**
//!    ```bash
//!    export GCP_PROJECT_ID="your-project-id"
//!    export GCP_LOCATION="us-central1"  # or your preferred region
//!    ```
//!
//! 2. **Create Cloud Tasks Queue:**
//!    ```bash
//!    gcloud tasks queues create servo-tasks \
//!      --location=$GCP_LOCATION \
//!      --max-dispatches-per-second=100 \
//!      --max-concurrent-dispatches=10
//!    ```
//!
//! 3. **Create Service Account:**
//!    ```bash
//!    gcloud iam service-accounts create servo-cloud-tasks \
//!      --display-name="Servo Cloud Tasks Service Account"
//!
//!    # Grant Cloud Tasks Enqueuer role
//!    gcloud projects add-iam-policy-binding $GCP_PROJECT_ID \
//!      --member="serviceAccount:servo-cloud-tasks@${GCP_PROJECT_ID}.iam.gserviceaccount.com" \
//!      --role="roles/cloudtasks.enqueuer"
//!
//!    # Download service account key
//!    gcloud iam service-accounts keys create key.json \
//!      --iam-account=servo-cloud-tasks@${GCP_PROJECT_ID}.iam.gserviceaccount.com
//!    ```
//!
//! 4. **Deploy Worker (or use mock URL):**
//!    ```bash
//!    # For testing, you can use a mock URL like:
//!    export GCP_WORKER_URL="https://mock-worker.example.com"
//!
//!    # Or deploy the actual servo-worker to Cloud Run:
//!    # cd servo-worker && gcloud run deploy ...
//!    ```
//!
//! 5. **Set Environment Variables:**
//!    ```bash
//!    export GCP_SERVICE_ACCOUNT_KEY_JSON="$(cat key.json)"
//!    export SERVO_HMAC_SECRET="$(openssl rand -base64 32)"
//!    export GCP_QUEUE_NAME="servo-tasks"
//!    ```
//!
//! 6. **Run Test:**
//!    ```bash
//!    cargo test --package servo-cloud-gcp --test cloud_tasks_real_integration -- --ignored --nocapture
//!    ```
//!
//! # What This Test Validates
//!
//! - OAuth2 access token acquisition from service account
//! - OIDC token generation for worker authentication
//! - HMAC payload signing
//! - Real Cloud Tasks API call (creates actual task in GCP)
//! - Response parsing (extracts task name)
//! - Error handling for API failures
//!
//! # Cleanup
//!
//! After testing, you may want to:
//! - Delete the test queue: `gcloud tasks queues delete servo-tasks --location=$GCP_LOCATION`
//! - View created tasks: `gcloud tasks list --queue=servo-tasks --location=$GCP_LOCATION`
//! - Delete tasks if needed (they expire automatically after 30 days)

use servo_cloud_gcp::queue::CloudTasksQueue;
use std::env;
use uuid::Uuid;

/// Helper to load required environment variables
fn load_test_config() -> anyhow::Result<(String, String, String, String, String, String)> {
    let project_id =
        env::var("GCP_PROJECT_ID").map_err(|_| anyhow::anyhow!("GCP_PROJECT_ID not set"))?;
    let location = env::var("GCP_LOCATION").map_err(|_| anyhow::anyhow!("GCP_LOCATION not set"))?;
    let queue_name =
        env::var("GCP_QUEUE_NAME").map_err(|_| anyhow::anyhow!("GCP_QUEUE_NAME not set"))?;
    let worker_url =
        env::var("GCP_WORKER_URL").map_err(|_| anyhow::anyhow!("GCP_WORKER_URL not set"))?;
    let service_account_json = env::var("GCP_SERVICE_ACCOUNT_KEY_JSON")
        .map_err(|_| anyhow::anyhow!("GCP_SERVICE_ACCOUNT_KEY_JSON not set"))?;
    let hmac_secret =
        env::var("SERVO_HMAC_SECRET").map_err(|_| anyhow::anyhow!("SERVO_HMAC_SECRET not set"))?;

    Ok((
        project_id,
        location,
        queue_name,
        worker_url,
        service_account_json,
        hmac_secret,
    ))
}

#[tokio::test]
#[ignore] // Requires GCP project and credentials
async fn test_real_cloud_tasks_enqueue() {
    // Load configuration from environment
    let (project_id, location, queue_name, worker_url, service_account_json, hmac_secret) =
        load_test_config().expect("Failed to load test configuration - see test file docs");

    // Create CloudTasksQueue
    let queue = CloudTasksQueue::new(
        project_id.clone(),
        location.clone(),
        queue_name.clone(),
        worker_url.clone(),
        &service_account_json,
        hmac_secret.clone(),
    )
    .expect("Failed to create CloudTasksQueue");

    // Generate test execution data
    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();
    let tenant_id = "test-tenant-real";
    let execution_plan = vec![Uuid::new_v4(), Uuid::new_v4()];

    println!("\n=== Cloud Tasks Real Integration Test ===");
    println!("Project: {}", project_id);
    println!("Location: {}", location);
    println!("Queue: {}", queue_name);
    println!("Worker URL: {}", worker_url);
    println!("Execution ID: {}", execution_id);
    println!("Workflow ID: {}", workflow_id);
    println!("=========================================\n");

    // Enqueue task (makes real API call to Cloud Tasks)
    let task_name = queue
        .enqueue(
            execution_id,
            workflow_id,
            tenant_id,
            None,
            execution_plan,
            None,
        )
        .await
        .expect("Failed to enqueue task to Cloud Tasks");

    println!("✅ Task enqueued successfully!");
    println!("Task name: {}", task_name);

    // Validate task name format
    assert!(
        task_name.starts_with(&format!(
            "projects/{}/locations/{}/queues/{}",
            project_id, location, queue_name
        )),
        "Task name should start with queue path"
    );
    assert!(
        task_name.contains("/tasks/"),
        "Task name should contain /tasks/"
    );

    println!("\n=== Test Passed ===");
    println!("You can view the task in GCP Console:");
    println!(
        "https://console.cloud.google.com/cloudtasks/queue/{}/{}/{}",
        location, queue_name, project_id
    );
    println!("\nOr via gcloud:");
    println!(
        "gcloud tasks list --queue={} --location={}",
        queue_name, location
    );
}

#[tokio::test]
#[ignore] // Requires GCP project and credentials
async fn test_real_cloud_tasks_error_handling() {
    // This test validates error handling by using an invalid queue name
    let (project_id, location, _queue_name, worker_url, service_account_json, hmac_secret) =
        load_test_config().expect("Failed to load test configuration");

    // Create CloudTasksQueue with non-existent queue
    let queue = CloudTasksQueue::new(
        project_id,
        location,
        "non-existent-queue".to_string(),
        worker_url,
        &service_account_json,
        hmac_secret,
    )
    .expect("Failed to create CloudTasksQueue");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Attempt to enqueue should fail with 404 or 403
    let result = queue
        .enqueue(
            execution_id,
            workflow_id,
            "test-tenant",
            None,
            Vec::new(),
            None,
        )
        .await;

    assert!(
        result.is_err(),
        "Should fail when enqueueing to non-existent queue"
    );

    let error = result.unwrap_err();
    println!("Expected error: {:?}", error);

    // Error should be Api error with 404 or 403
    match error {
        servo_cloud_gcp::Error::Api(msg) => {
            assert!(
                msg.contains("404") || msg.contains("403"),
                "Error should mention 404 or 403: {}",
                msg
            );
        }
        _ => panic!("Expected Api error, got: {:?}", error),
    }

    println!("✅ Error handling validated");
}
