//! Task enqueuer abstraction for cloud-agnostic task queuing

use async_trait::async_trait;
use servo_storage::TenantId;
use uuid::Uuid;

/// Result type for task enqueueing operations
pub type EnqueueResult<T> = std::result::Result<T, EnqueueError>;

/// Errors that can occur during task enqueueing
#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Queue API error: {0}")]
    QueueApi(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Abstraction for enqueueing workflow execution tasks.
///
/// This trait allows the orchestrator to trigger execution without
/// depending on specific cloud provider implementations (GCP, AWS, Azure, etc.).
///
/// Implementations should:
/// - Authenticate with the cloud provider
/// - Sign payloads for security
/// - Submit tasks to the appropriate queue/service
/// - Return a task identifier for tracking
#[async_trait]
pub trait TaskEnqueuer: Send + Sync {
    /// Enqueue a workflow execution task
    ///
    /// # Arguments
    ///
    /// * `execution_id` - UUID of the execution record in storage
    /// * `workflow_id` - UUID of the workflow to execute
    /// * `tenant_id` - Tenant identifier for multi-tenant isolation
    /// * `idempotency_key` - Optional idempotency key for deduplication
    /// * `execution_plan` - Pre-compiled execution plan (topologically sorted asset IDs)
    ///
    /// # Returns
    ///
    /// A task identifier (e.g., Cloud Tasks task name) for tracking
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Authentication fails
    /// - The queue is unavailable
    /// - The request is malformed
    async fn enqueue(
        &self,
        execution_id: Uuid,
        workflow_id: Uuid,
        tenant_id: &TenantId,
        idempotency_key: Option<String>,
        execution_plan: Vec<Uuid>,
    ) -> EnqueueResult<String>;
}
