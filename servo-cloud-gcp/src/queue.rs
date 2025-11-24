//! Cloud Tasks queue implementation

use crate::auth::GcpAuth;
use crate::metrics::{ENQUEUE_DURATION, ENQUEUE_RETRIES, ENQUEUE_TOTAL};
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

        // Configure HTTP client with 30-second timeout to prevent hangs
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("Failed to create HTTP client: {}", e)))?;

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
    /// - OIDC authentication automatically injected by Cloud Tasks via service account
    /// - X-Servo-Signature header containing HMAC signature for payload integrity
    /// - JSON body with execution details
    ///
    /// # Arguments
    ///
    /// * `execution_id` - UUID of the execution record
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant identifier
    /// * `idempotency_key` - Optional idempotency key (not enforced by Cloud Tasks, passed through to worker)
    /// * `execution_plan` - Pre-compiled execution plan (currently empty, worker compiles from workflow)
    ///
    /// # Returns
    ///
    /// The Cloud Tasks task name (e.g., "projects/.../locations/.../queues/.../tasks/...")
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - OAuth2 access token acquisition fails
    /// - OIDC token generation fails
    /// - Payload signing fails
    /// - Cloud Tasks API request fails (4xx/5xx)
    pub async fn enqueue(
        &self,
        execution_id: Uuid,
        workflow_id: Uuid,
        tenant_id: &str,
        idempotency_key: Option<String>,
        execution_plan: Vec<Uuid>,
    ) -> Result<String> {
        // Start total duration timer
        let _total_timer = ENQUEUE_DURATION.with_label_values(&["total"]).start_timer();

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

        // 3. Build target URL for Cloud Run worker
        let target_url = format!("{}/execute", self.worker_url);

        tracing::info!(
            execution_id = %execution_id,
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
            queue = %self.queue_path(),
            "Enqueueing execution task"
        );

        // 4. Create Cloud Tasks task (OIDC authentication handled by Cloud Tasks)
        let result = self
            .create_cloud_task(&target_url, &signature, payload_bytes)
            .await;

        // Record success or failure metrics
        match &result {
            Ok(task_id) => {
                ENQUEUE_TOTAL
                    .with_label_values(&["success", tenant_id])
                    .inc();
                tracing::info!(
                    execution_id = %execution_id,
                    task_id = %task_id,
                    "Task enqueued successfully"
                );
            }
            Err(e) => {
                // Determine if it's a rate limit or generic failure
                let status = if matches!(e, Error::Api(msg) if msg.contains("429")) {
                    "rate_limit"
                } else {
                    "failure"
                };
                ENQUEUE_TOTAL
                    .with_label_values(&[status, tenant_id])
                    .inc();
                tracing::error!(
                    execution_id = %execution_id,
                    error = %e,
                    "Task enqueue failed"
                );
            }
        }

        result
    }

    /// Create a Cloud Tasks HTTP POST task
    ///
    /// Calls the Cloud Tasks API with retries and proper error handling.
    /// Cloud Tasks will automatically inject OIDC authentication using the service account.
    async fn create_cloud_task(
        &self,
        target_url: &str,
        hmac_signature: &str,
        payload: &[u8],
    ) -> Result<String> {
        // Cloud Tasks API endpoint
        let api_url = format!(
            "https://cloudtasks.googleapis.com/v2/{}/tasks",
            self.queue_path()
        );

        // Task definition
        use base64::Engine as _;
        let task = serde_json::json!({
            "task": {
                "httpRequest": {
                    "url": target_url,
                    "httpMethod": "POST",
                    "headers": {
                        "Content-Type": "application/json",
                        "X-Servo-Signature": hmac_signature,
                    },
                    "body": base64::engine::general_purpose::STANDARD.encode(payload),
                    "oidcToken": {
                        "serviceAccountEmail": self.auth.service_account_email(),
                    }
                }
            }
        });

        // Get OAuth2 access token for Cloud Tasks API authentication
        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        // Call Cloud Tasks API with retries
        let task_name = self
            .call_cloud_tasks_api(&api_url, &access_token, &task)
            .await?;

        Ok(task_name)
    }

    /// Call Cloud Tasks API with exponential backoff retries
    async fn call_cloud_tasks_api(
        &self,
        api_url: &str,
        access_token: &str,
        task: &serde_json::Value,
    ) -> Result<String> {
        const MAX_RETRIES: u32 = 3;
        const INITIAL_BACKOFF_MS: u64 = 100;

        let mut attempt = 0;
        let mut backoff_ms = INITIAL_BACKOFF_MS;

        loop {
            attempt += 1;

            // Start API call timer
            let api_timer = ENQUEUE_DURATION.with_label_values(&["api"]).start_timer();

            let response = self
                .http_client
                .post(api_url)
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .json(task)
                .send()
                .await
                .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

            let status = response.status();
            drop(api_timer); // Stop timer

            // Success
            if status.is_success() {
                let task_response: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| Error::Api(format!("Failed to parse response: {}", e)))?;

                let task_name = task_response["name"]
                    .as_str()
                    .ok_or_else(|| Error::Api("No task name in response".to_string()))?
                    .to_string();

                tracing::info!(
                    task_name = %task_name,
                    attempt = attempt,
                    "Cloud Tasks API call succeeded"
                );

                return Ok(task_name);
            }

            // Rate limit (429) - retry with Retry-After header if available
            if status.as_u16() == 429 {
                ENQUEUE_RETRIES.with_label_values(&["429"]).inc();
                // Read Retry-After header before consuming response body
                let retry_after_ms = if let Some(retry_after) = response.headers().get("retry-after") {
                    retry_after
                        .to_str()
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|secs| secs * 1000) // Convert seconds to milliseconds
                } else {
                    None
                };

                let body = response.text().await.unwrap_or_default();

                if attempt >= MAX_RETRIES {
                    return Err(Error::Api(format!(
                        "Cloud Tasks API failed after {} attempts due to rate limiting. Last response: {} - {}",
                        MAX_RETRIES, status, body
                    )));
                }

                let delay_ms = retry_after_ms.unwrap_or(backoff_ms);

                tracing::warn!(
                    status = %status,
                    attempt = attempt,
                    max_retries = MAX_RETRIES,
                    delay_ms = delay_ms,
                    retry_after = ?retry_after_ms,
                    "Cloud Tasks API rate limit exceeded, retrying after delay"
                );

                // Respect Retry-After header or use exponential backoff
                tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms))
                    .await;

                // Continue exponential backoff for next attempt if no Retry-After header
                if retry_after_ms.is_none() {
                    backoff_ms *= 2;
                }

                continue;
            }

            // Other client errors (4xx) - don't retry
            if status.is_client_error() {
                let body = response.text().await.unwrap_or_default();
                return Err(Error::Api(format!(
                    "Cloud Tasks API returned {}: {}",
                    status, body
                )));
            }

            // Server error (5xx) - retry with backoff
            if status.is_server_error() {
                ENQUEUE_RETRIES.with_label_values(&["5xx"]).inc();
                let body = response.text().await.unwrap_or_default();

                if attempt >= MAX_RETRIES {
                    return Err(Error::Api(format!(
                        "Cloud Tasks API failed after {} attempts. Last response: {} - {}",
                        MAX_RETRIES, status, body
                    )));
                }

                tracing::warn!(
                    status = %status,
                    attempt = attempt,
                    max_retries = MAX_RETRIES,
                    backoff_ms = backoff_ms,
                    "Cloud Tasks API returned server error, retrying"
                );

                // Exponential backoff with jitter
                let jitter = rand::random::<u64>() % (backoff_ms / 2);
                tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms + jitter))
                    .await;
                backoff_ms *= 2;

                continue;
            }

            // Unexpected status code
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Api(format!(
                "Unexpected status from Cloud Tasks API: {} - {}",
                status, body
            )));
        }
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
