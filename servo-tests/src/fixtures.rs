//! Test data factories for Servo types
//!
//! Provides pre-built test data with sensible defaults.
//! Use these for quick test setup without needing to specify every field.

use chrono::Utc;
use servo_storage::models::{AssetModel, ExecutionModel, WorkflowModel};
use servo_storage::TenantId;
use sqlx::types::Json;
use uuid::Uuid;

/// Workflow fixture factories
pub mod workflow {
    use super::*;

    /// Create a simple workflow with minimal configuration
    pub fn simple() -> WorkflowModel {
        WorkflowModel {
            id: Uuid::new_v4(),
            name: "simple-test-workflow".to_string(),
            description: Some("A simple test workflow".to_string()),
            owner: Some("test-owner".to_string()),
            tags: Json(vec!["test".to_string()]),
            tenant_id: Some("test-tenant".to_string()),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Create a workflow with a specific name
    pub fn named(name: &str) -> WorkflowModel {
        let mut wf = simple();
        wf.name = name.to_string();
        wf
    }

    /// Create a workflow with specific tags
    pub fn with_tags(tags: Vec<String>) -> WorkflowModel {
        let mut wf = simple();
        wf.tags = Json(tags);
        wf
    }

    /// Create a workflow for a specific tenant
    pub fn for_tenant(tenant_id: &str) -> WorkflowModel {
        let mut wf = simple();
        wf.tenant_id = Some(tenant_id.to_string());
        wf
    }

    /// Create multiple workflows (useful for pagination tests)
    pub fn batch(count: usize) -> Vec<WorkflowModel> {
        (0..count)
            .map(|i| {
                let mut wf = simple();
                wf.name = format!("workflow-{}", i);
                wf.id = Uuid::new_v4();
                wf
            })
            .collect()
    }
}

/// Execution fixture factories
pub mod execution {
    use super::*;

    /// Create a pending execution
    pub fn pending(workflow_id: Uuid) -> ExecutionModel {
        ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id,
            state: "pending".to_string(),
            tenant_id: Some("test-tenant".to_string()),
            idempotency_key: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Create a running execution
    pub fn running(workflow_id: Uuid) -> ExecutionModel {
        let mut exec = pending(workflow_id);
        exec.state = "running".to_string();
        exec.started_at = Some(Utc::now());
        exec
    }

    /// Create a succeeded execution
    pub fn succeeded(workflow_id: Uuid) -> ExecutionModel {
        let mut exec = running(workflow_id);
        exec.state = "succeeded".to_string();
        exec.completed_at = Some(Utc::now());
        exec
    }

    /// Create a failed execution
    pub fn failed(workflow_id: Uuid, error: &str) -> ExecutionModel {
        let mut exec = running(workflow_id);
        exec.state = "failed".to_string();
        exec.completed_at = Some(Utc::now());
        exec.error_message = Some(error.to_string());
        exec
    }

    /// Create an execution with an idempotency key
    pub fn with_idempotency_key(workflow_id: Uuid, key: &str) -> ExecutionModel {
        let mut exec = pending(workflow_id);
        exec.idempotency_key = Some(key.to_string());
        exec
    }

    /// Create an execution for a specific tenant
    pub fn for_tenant(workflow_id: Uuid, tenant_id: &str) -> ExecutionModel {
        let mut exec = pending(workflow_id);
        exec.tenant_id = Some(tenant_id.to_string());
        exec
    }
}

/// Asset fixture factories
pub mod asset {
    use super::*;

    /// Create a simple asset
    pub fn simple() -> AssetModel {
        AssetModel {
            id: Uuid::new_v4(),
            name: "test-asset".to_string(),
            description: Some("A test asset".to_string()),
            asset_type: "table".to_string(),
            owner: Some("test-owner".to_string()),
            tags: Json(vec!["test".to_string()]),
            tenant_id: Some("test-tenant".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Create an asset with a specific name
    pub fn named(name: &str) -> AssetModel {
        let mut asset = simple();
        asset.name = name.to_string();
        asset
    }

    /// Create an asset of a specific type
    pub fn of_type(asset_type: &str) -> AssetModel {
        let mut asset = simple();
        asset.asset_type = asset_type.to_string();
        asset
    }

    /// Create multiple assets (useful for building DAGs)
    pub fn batch(count: usize) -> Vec<AssetModel> {
        (0..count)
            .map(|i| {
                let mut asset = simple();
                asset.name = format!("asset-{}", i);
                asset.id = Uuid::new_v4();
                asset
            })
            .collect()
    }
}

/// Tenant fixture factories
pub mod tenant {
    use super::*;

    /// Create a random tenant ID (for test isolation)
    pub fn random() -> TenantId {
        TenantId::new(format!("test-{}", Uuid::new_v4()))
    }

    /// Create a tenant ID with a specific name
    pub fn named(name: &str) -> TenantId {
        TenantId::new(name.to_string())
    }

    /// Default test tenant
    pub fn default() -> TenantId {
        TenantId::new("test-tenant".to_string())
    }
}

/// Task payload fixture factories
pub mod task_payload {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Task payload structure matching what Cloud Tasks sends
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct TaskPayload {
        pub execution_id: Uuid,
        pub workflow_id: Uuid,
        pub tenant_id: String,
        pub idempotency_key: Option<String>,
        pub execution_plan: Vec<Uuid>,
    }

    /// Create a simple task payload
    pub fn simple() -> TaskPayload {
        TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "test-tenant".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
        }
    }

    /// Create a task payload with specific IDs
    pub fn with_ids(execution_id: Uuid, workflow_id: Uuid) -> TaskPayload {
        TaskPayload {
            execution_id,
            workflow_id,
            tenant_id: "test-tenant".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
        }
    }

    /// Create a task payload with an execution plan
    pub fn with_plan(assets: Vec<Uuid>) -> TaskPayload {
        let mut payload = simple();
        payload.execution_plan = assets;
        payload
    }

    /// Convert to JSON bytes
    pub fn to_json_bytes(payload: &TaskPayload) -> Vec<u8> {
        serde_json::to_vec(payload).expect("Failed to serialize task payload")
    }

    /// Convert to base64-encoded JSON
    pub fn to_base64(payload: &TaskPayload) -> String {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let json_bytes = to_json_bytes(payload);
        STANDARD.encode(json_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_fixtures_have_unique_ids() {
        let wf1 = workflow::simple();
        let wf2 = workflow::simple();
        assert_ne!(wf1.id, wf2.id);
    }

    #[test]
    fn test_execution_fixtures_have_correct_states() {
        let wf_id = Uuid::new_v4();
        assert_eq!(execution::pending(wf_id).state, "pending");
        assert_eq!(execution::running(wf_id).state, "running");
        assert_eq!(execution::succeeded(wf_id).state, "succeeded");
        assert_eq!(execution::failed(wf_id, "error").state, "failed");
    }

    #[test]
    fn test_tenant_random_is_unique() {
        let t1 = tenant::random();
        let t2 = tenant::random();
        assert_ne!(t1.as_str(), t2.as_str());
    }

    #[test]
    fn test_batch_creates_correct_count() {
        let workflows = workflow::batch(5);
        assert_eq!(workflows.len(), 5);

        let assets = asset::batch(10);
        assert_eq!(assets.len(), 10);
    }
}
