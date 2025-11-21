//! Workflow executor

use crate::Result;
use async_trait::async_trait;
use uuid::Uuid;

/// Unique identifier for an execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionId(pub Uuid);

impl ExecutionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a workflow execution
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub execution_id: ExecutionId,
    pub success: bool,
    pub error: Option<String>,
}

/// Trait for workflow executors
#[async_trait]
pub trait Executor: Send + Sync {
    /// Execute a workflow
    async fn execute(
        &self,
        workflow_id: servo_core::WorkflowId,
    ) -> Result<ExecutionResult>;

    /// Cancel a running execution
    async fn cancel(&self, execution_id: ExecutionId) -> Result<()>;

    /// Get the status of an execution
    async fn status(&self, execution_id: ExecutionId) -> Result<ExecutionResult>;
}
