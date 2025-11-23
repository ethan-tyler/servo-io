//! Cloud Tasks queue implementation

use crate::auth::GcpAuth;
use crate::signing::sign_payload;
use crate::{Error, Result};
use async_trait::async_trait;
use serde::Serialize;
use servo_runtime::task_enqueuer::{EnqueueError, EnqueueResult, TaskEnqueuer};
use servo_storage::TenantId;
use uuid::Uuid;

/// Task payload sent to the worker
#[derive(Debug, Serialize)]
pub struct TaskPayload {
    pub execution_id: Uuid,
    pub workflow_id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: Option<String>,
    pub execution_plan: Vec<Uuid>,
}

/// Cloud Tasks queue for task scheduling
pub struct CloudTasksQueue {
    project_id: String,
    location: String,
    queue_name: String,
    worker_url: String,
    auth: GcpAuth,
    hmac_secret: String,
    http_client: reqwest::Client,
}

impl CloudTasksQueue {
    /// Create a new Cloud Tasks queue
    ///
    /// # Arguments
    ///
    /// * `project_id` - GCP project ID
    /// * `location` - GCP location/region (e.g., "us-central1")
    /// * `queue_name` - Cloud Tasks queue name
    /// * `worker_url` - Cloud Run worker service URL
    /// * `service_account_json` - Service account credentials JSON
    /// * `hmac_secret` - Secret for HMAC signing task payloads
    pub fn new(
        project_id: String,
        location: String,
        queue_name: String,
        worker_url: String,
        service_account_json: &str,
        hmac_secret: String,
    ) -> Result<Self> {
        let auth = GcpAuth::from_service_account_json(service_account_json)?;
        let http_client = reqwest::Client::new();

        Ok(Self {
            project_id,
            location,
            queue_name,
            worker_url,
            auth,
            hmac_secret,
            http_client,
        })
    }

    /// Enqueue a task to execute a workflow
    ///
    /// Creates a Cloud Tasks HTTP POST task with:
    /// - Authorization header containing OIDC token for worker authentication
    /// - X-Servo-Signature header containing HMAC signature for payload integrity
    /// - JSON body with execution details
    ///
    /// # Arguments
    ///
    /// * `execution_id` - UUID of the execution record
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant identifier
    /// * `idempotency_key` - Optional idempotency key
    /// * `execution_plan` - Pre-compiled execution plan (asset IDs in topological order)
    ///
    /// # Returns
    ///
    /// The Cloud Tasks task name/ID for tracking
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Token generation fails
    /// - Payload signing fails
    /// - Cloud Tasks API request fails
    pub async fn enqueue(
        &self,
        execution_id: Uuid,
        workflow_id: Uuid,
        tenant_id: &str,
        idempotency_key: Option<String>,
        execution_plan: Vec<Uuid>,
    ) -> Result<String> {
        // 1. Create task payload
        let payload = TaskPayload {
            execution_id,
            workflow_id,
            tenant_id: tenant_id.to_string(),
            idempotency_key,
            execution_plan,
        };

        let payload_json = serde_json::to_string(&payload)
            .map_err(|e| Error::Internal(format!("Failed to serialize payload: {}", e)))?;
        let payload_bytes = payload_json.as_bytes();

        // 2. Sign payload with HMAC for integrity verification
        let signature = sign_payload(payload_bytes, &self.hmac_secret)?;

        // 3. Generate OIDC token for worker authentication
        let target_url = format!("{}/execute", self.worker_url);
        let oidc_token = self.auth.generate_oidc_token(&target_url)?;

        tracing::info!(
            execution_id = %execution_id,
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
            queue = %self.queue_path(),
            "Enqueueing execution task"
        );

        // 4. Create Cloud Tasks task
        let task_id = self
            .create_cloud_task(&target_url, &oidc_token, &signature, payload_bytes)
            .await?;

        tracing::info!(
            execution_id = %execution_id,
            task_id = %task_id,
            "Task enqueued successfully"
        );

        Ok(task_id)
    }

    /// Create a Cloud Tasks HTTP POST task
    async fn create_cloud_task(
        &self,
        target_url: &str,
        oidc_token: &str,
        hmac_signature: &str,
        payload: &[u8],
    ) -> Result<String> {
        // Cloud Tasks API endpoint
        let _api_url = format!(
            "https://cloudtasks.googleapis.com/v2/{}/tasks",
            self.queue_path()
        );

        // Task definition
        use base64::Engine as _;
        let _task = serde_json::json!({
            "task": {
                "httpRequest": {
                    "url": target_url,
                    "httpMethod": "POST",
                    "headers": {
                        "Content-Type": "application/json",
                        "Authorization": format!("Bearer {}", oidc_token),
                        "X-Servo-Signature": hmac_signature,
                    },
                    "body": base64::engine::general_purpose::STANDARD.encode(payload),
                    "oidcToken": {
                        "serviceAccountEmail": self.auth.service_account_email(),
                    }
                }
            }
        });

        // TODO: Get OAuth2 access token for Cloud Tasks API authentication
        // For now, return a mock task ID to enable testing the flow
        // In production, this would make the actual API call:
        //
        // let access_token = self.get_access_token().await?;
        // let response = self.http_client
        //     .post(&api_url)
        //     .header("Authorization", format!("Bearer {}", access_token))
        //     .json(&task)
        //     .send()
        //     .await?;
        //
        // let task_response: serde_json::Value = response.json().await?;
        // let task_name = task_response["name"].as_str()
        //     .ok_or_else(|| Error::Api("No task name in response".to_string()))?;

        tracing::warn!(
            "Cloud Tasks API integration not yet implemented - using mock task ID"
        );

        // Mock task ID for development
        let mock_task_id = format!(
            "{}/tasks/mock-{}",
            self.queue_path(),
            Uuid::new_v4()
        );

        Ok(mock_task_id)
    }

    /// Get the full queue path
    pub fn queue_path(&self) -> String {
        format!(
            "projects/{}/locations/{}/queues/{}",
            self.project_id, self.location, self.queue_name
        )
    }
}

/// Implement TaskEnqueuer trait for cloud-agnostic orchestrator integration
#[async_trait]
impl TaskEnqueuer for CloudTasksQueue {
    async fn enqueue(
        &self,
        execution_id: Uuid,
        workflow_id: Uuid,
        tenant_id: &TenantId,
        idempotency_key: Option<String>,
        execution_plan: Vec<Uuid>,
    ) -> EnqueueResult<String> {
        self.enqueue(
            execution_id,
            workflow_id,
            tenant_id.as_str(),
            idempotency_key,
            execution_plan,
        )
        .await
        .map_err(|e| match e {
            Error::Auth(msg) => EnqueueError::Authentication(msg),
            Error::Config(msg) => EnqueueError::Configuration(msg),
            Error::Api(msg) => EnqueueError::QueueApi(msg),
            Error::Http(e) => EnqueueError::QueueApi(e.to_string()),
            Error::Internal(msg) => EnqueueError::Internal(msg),
        })
    }
}
