//! Cloud Run executor implementation

use crate::Result;
use async_trait::async_trait;
use servo_runtime::{ExecutionResult, Executor};

/// Cloud Run executor for running workflows
pub struct CloudRunExecutor {
    project_id: String,
    region: String,
    service_name: String,
}

impl CloudRunExecutor {
    /// Create a new Cloud Run executor
    pub fn new(project_id: String, region: String, service_name: String) -> Self {
        Self {
            project_id,
            region,
            service_name,
        }
    }
}

#[async_trait]
impl Executor for CloudRunExecutor {
    async fn execute(
        &self,
        workflow_id: servo_core::WorkflowId,
    ) -> servo_runtime::Result<ExecutionResult> {
        // TODO: Implement Cloud Run execution
        // 1. Create execution record
        // 2. Enqueue task to Cloud Tasks
        // 3. Return execution ID

        tracing::info!(
            "Executing workflow {:?} on Cloud Run in project {}",
            workflow_id,
            self.project_id
        );

        Err(servo_runtime::Error::Internal(
            "Not yet implemented".to_string(),
        ))
    }

    async fn cancel(
        &self,
        execution_id: servo_runtime::executor::ExecutionId,
    ) -> servo_runtime::Result<()> {
        // TODO: Implement execution cancellation
        tracing::info!("Cancelling execution {:?}", execution_id);

        Err(servo_runtime::Error::Internal(
            "Not yet implemented".to_string(),
        ))
    }

    async fn status(
        &self,
        execution_id: servo_runtime::executor::ExecutionId,
    ) -> servo_runtime::Result<ExecutionResult> {
        // TODO: Implement status check
        tracing::info!("Checking status of execution {:?}", execution_id);

        Err(servo_runtime::Error::Internal(
            "Not yet implemented".to_string(),
        ))
    }
}
