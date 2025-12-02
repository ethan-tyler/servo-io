//! Type-safe builders for constructing test objects
//!
//! Builders provide a fluent API for constructing complex test objects
//! with clear, readable code.

use axum::body::Body;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use http::{header, Request};
use serde_json::json;
use uuid::Uuid;

/// Builder for constructing HTTP requests to the /execute endpoint
pub struct ExecutionRequestBuilder {
    execution_id: Uuid,
    workflow_id: Uuid,
    tenant_id: String,
    idempotency_key: Option<String>,
    execution_plan: Vec<Uuid>,
    hmac_secret: Option<String>,
    oidc_token: Option<String>,
    content_type: Option<String>,
    custom_headers: Vec<(String, String)>,
}

impl ExecutionRequestBuilder {
    /// Create a new builder with random IDs
    pub fn new() -> Self {
        Self {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "test-tenant".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
            hmac_secret: None,
            oidc_token: None,
            content_type: Some("application/json".to_string()),
            custom_headers: vec![],
        }
    }

    /// Set the execution ID
    pub fn execution_id(mut self, id: Uuid) -> Self {
        self.execution_id = id;
        self
    }

    /// Set the workflow ID
    pub fn workflow_id(mut self, id: Uuid) -> Self {
        self.workflow_id = id;
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, id: &str) -> Self {
        self.tenant_id = id.to_string();
        self
    }

    /// Set an idempotency key
    pub fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    /// Set the execution plan (list of asset IDs)
    pub fn execution_plan(mut self, assets: Vec<Uuid>) -> Self {
        self.execution_plan = assets;
        self
    }

    /// Sign the request with HMAC
    pub fn signed(mut self, secret: &str) -> Self {
        self.hmac_secret = Some(secret.to_string());
        self
    }

    /// Add OIDC Bearer token
    pub fn with_oidc_token(mut self, token: &str) -> Self {
        self.oidc_token = Some(token.to_string());
        self
    }

    /// Set a custom content type (or None to omit)
    pub fn content_type(mut self, ct: Option<&str>) -> Self {
        self.content_type = ct.map(|s| s.to_string());
        self
    }

    /// Add a custom header
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.custom_headers
            .push((name.to_string(), value.to_string()));
        self
    }

    /// Build the request payload as JSON
    fn build_payload(&self) -> serde_json::Value {
        json!({
            "execution_id": self.execution_id,
            "workflow_id": self.workflow_id,
            "tenant_id": self.tenant_id,
            "idempotency_key": self.idempotency_key,
            "execution_plan": self.execution_plan
        })
    }

    /// Compute HMAC-SHA256 signature
    fn compute_signature(payload_bytes: &[u8], secret: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(payload_bytes);
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Build the HTTP request
    pub fn build(self) -> Request<Body> {
        let payload = self.build_payload();
        let json_bytes = serde_json::to_vec(&payload).expect("Failed to serialize payload");
        let base64_payload = STANDARD.encode(&json_bytes);
        let body_bytes = base64_payload.as_bytes().to_vec();

        let mut builder = Request::builder().method("POST").uri("/execute");

        // Add content type if set
        if let Some(ct) = &self.content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }

        // Add HMAC signature if secret provided
        if let Some(secret) = &self.hmac_secret {
            let signature = Self::compute_signature(&body_bytes, secret);
            builder = builder.header("x-servo-signature", signature);
        }

        // Add OIDC token if provided
        if let Some(token) = &self.oidc_token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {}", token));
        }

        // Add custom headers
        for (name, value) in &self.custom_headers {
            builder = builder.header(name.as_str(), value.as_str());
        }

        builder
            .body(Body::from(body_bytes))
            .expect("Failed to build request")
    }

    /// Build and return the body bytes along with the signature
    /// Useful for tests that need to verify signature computation
    pub fn build_parts(self) -> (Vec<u8>, Option<String>) {
        let payload = self.build_payload();
        let json_bytes = serde_json::to_vec(&payload).expect("Failed to serialize payload");
        let base64_payload = STANDARD.encode(&json_bytes);
        let body_bytes = base64_payload.as_bytes().to_vec();

        let signature = self
            .hmac_secret
            .as_ref()
            .map(|secret| Self::compute_signature(&body_bytes, secret));

        (body_bytes, signature)
    }
}

impl Default for ExecutionRequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing workflow models in tests
pub struct WorkflowBuilder {
    id: Uuid,
    name: String,
    description: Option<String>,
    owner: Option<String>,
    tags: Vec<String>,
    tenant_id: Option<String>,
    version: i32,
}

impl WorkflowBuilder {
    /// Create a new workflow builder
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "test-workflow".to_string(),
            description: None,
            owner: None,
            tags: vec![],
            tenant_id: None,
            version: 1,
        }
    }

    /// Set the workflow ID
    pub fn id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    /// Set the workflow name
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Set the description
    pub fn description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }

    /// Set the owner
    pub fn owner(mut self, owner: &str) -> Self {
        self.owner = Some(owner.to_string());
        self
    }

    /// Add a tag
    pub fn tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, tenant_id: &str) -> Self {
        self.tenant_id = Some(tenant_id.to_string());
        self
    }

    /// Set the version
    pub fn version(mut self, version: i32) -> Self {
        self.version = version;
        self
    }

    /// Build the workflow model
    pub fn build(self) -> servo_storage::models::WorkflowModel {
        use chrono::Utc;
        use sqlx::types::Json;

        servo_storage::models::WorkflowModel {
            id: self.id,
            name: self.name,
            description: self.description,
            owner: self.owner,
            tags: Json(self.tags),
            tenant_id: self.tenant_id,
            version: self.version,
            schedule_cron: None,
            schedule_timezone: None,
            schedule_enabled: None,
            scheduler_job_name: None,
            last_scheduled_run: None,
            next_scheduled_run: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

impl Default for WorkflowBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for constructing execution models in tests
pub struct ExecutionBuilder {
    id: Uuid,
    workflow_id: Uuid,
    state: String,
    tenant_id: Option<String>,
    idempotency_key: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    error_message: Option<String>,
}

impl ExecutionBuilder {
    /// Create a new execution builder
    pub fn new(workflow_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            workflow_id,
            state: "pending".to_string(),
            tenant_id: None,
            idempotency_key: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        }
    }

    /// Set the execution ID
    pub fn id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    /// Set the state
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Set to pending state
    pub fn pending(self) -> Self {
        self.state("pending")
    }

    /// Set to running state
    pub fn running(mut self) -> Self {
        self.state = "running".to_string();
        self.started_at = Some(chrono::Utc::now());
        self
    }

    /// Set to succeeded state
    pub fn succeeded(mut self) -> Self {
        self = self.running();
        self.state = "succeeded".to_string();
        self.completed_at = Some(chrono::Utc::now());
        self
    }

    /// Set to failed state
    pub fn failed(mut self, error: &str) -> Self {
        self = self.running();
        self.state = "failed".to_string();
        self.completed_at = Some(chrono::Utc::now());
        self.error_message = Some(error.to_string());
        self
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, tenant_id: &str) -> Self {
        self.tenant_id = Some(tenant_id.to_string());
        self
    }

    /// Set an idempotency key
    pub fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    /// Build the execution model
    pub fn build(self) -> servo_storage::models::ExecutionModel {
        use chrono::Utc;

        servo_storage::models::ExecutionModel {
            id: self.id,
            workflow_id: self.workflow_id,
            state: self.state,
            tenant_id: self.tenant_id,
            idempotency_key: self.idempotency_key,
            started_at: self.started_at,
            completed_at: self.completed_at,
            error_message: self.error_message,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_request_builder_basic() {
        let request = ExecutionRequestBuilder::new()
            .tenant_id("my-tenant")
            .signed("test-secret-minimum-32-bytes-long")
            .build();

        assert_eq!(request.method(), "POST");
        assert_eq!(request.uri(), "/execute");
        assert!(request.headers().contains_key("x-servo-signature"));
    }

    #[test]
    fn test_execution_request_builder_with_oidc() {
        let request = ExecutionRequestBuilder::new()
            .signed("test-secret-minimum-32-bytes-long")
            .with_oidc_token("test-token")
            .build();

        let auth = request
            .headers()
            .get(header::AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(auth, "Bearer test-token");
    }

    #[test]
    fn test_workflow_builder() {
        let wf = WorkflowBuilder::new()
            .name("my-workflow")
            .description("Test workflow")
            .tag("test")
            .tag("ci")
            .tenant_id("tenant-1")
            .build();

        assert_eq!(wf.name, "my-workflow");
        assert_eq!(wf.description, Some("Test workflow".to_string()));
        assert_eq!(wf.tags.0.len(), 2);
        assert_eq!(wf.tenant_id, Some("tenant-1".to_string()));
    }

    #[test]
    fn test_execution_builder_states() {
        let wf_id = Uuid::new_v4();

        let pending = ExecutionBuilder::new(wf_id).pending().build();
        assert_eq!(pending.state, "pending");
        assert!(pending.started_at.is_none());

        let running = ExecutionBuilder::new(wf_id).running().build();
        assert_eq!(running.state, "running");
        assert!(running.started_at.is_some());

        let succeeded = ExecutionBuilder::new(wf_id).succeeded().build();
        assert_eq!(succeeded.state, "succeeded");
        assert!(succeeded.completed_at.is_some());

        let failed = ExecutionBuilder::new(wf_id).failed("test error").build();
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.error_message, Some("test error".to_string()));
    }
}
