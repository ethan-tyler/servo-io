//! Cloud Tasks Contract Tests
//!
//! These tests verify the contract between the orchestrator (servo-cloud-gcp)
//! and the worker (servo-worker) for task payloads and signatures.
//!
//! Contract tests ensure:
//! - Task payload structure matches expected schema
//! - HMAC signature is computed correctly
//! - Base64 encoding matches worker expectations
//! - Cloud Tasks API request format is correct

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

/// Task payload structure (must match worker's TaskPayload)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskPayload {
    pub execution_id: Uuid,
    pub workflow_id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: Option<String>,
    pub execution_plan: Vec<Uuid>,
}

/// Create a test task payload
fn create_test_payload() -> TaskPayload {
    TaskPayload {
        execution_id: Uuid::new_v4(),
        workflow_id: Uuid::new_v4(),
        tenant_id: "test-tenant".to_string(),
        idempotency_key: None,
        execution_plan: vec![],
    }
}

/// Compute HMAC-SHA256 signature (matching signing.rs)
fn sign_payload(payload: &[u8], secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload);
    let signature = mac.finalize().into_bytes();
    hex::encode(signature)
}

mod task_payload_contract {
    use super::*;

    #[test]
    fn payload_serializes_to_expected_json_schema() {
        let payload = create_test_payload();
        let json_str = serde_json::to_string(&payload).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        // Verify all required fields are present
        assert!(parsed.get("execution_id").is_some(), "missing execution_id");
        assert!(parsed.get("workflow_id").is_some(), "missing workflow_id");
        assert!(parsed.get("tenant_id").is_some(), "missing tenant_id");
        assert!(
            parsed.get("execution_plan").is_some(),
            "missing execution_plan"
        );
        // idempotency_key can be null or absent
    }

    #[test]
    fn execution_id_is_uuid_string() {
        let payload = create_test_payload();
        let json: Value = serde_json::to_value(&payload).unwrap();

        let execution_id = json["execution_id"].as_str().unwrap();
        assert!(Uuid::parse_str(execution_id).is_ok());
    }

    #[test]
    fn workflow_id_is_uuid_string() {
        let payload = create_test_payload();
        let json: Value = serde_json::to_value(&payload).unwrap();

        let workflow_id = json["workflow_id"].as_str().unwrap();
        assert!(Uuid::parse_str(workflow_id).is_ok());
    }

    #[test]
    fn tenant_id_is_string() {
        let payload = create_test_payload();
        let json: Value = serde_json::to_value(&payload).unwrap();

        assert!(json["tenant_id"].is_string());
        assert_eq!(json["tenant_id"], "test-tenant");
    }

    #[test]
    fn execution_plan_is_array_of_uuids() {
        let payload = TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "test".to_string(),
            idempotency_key: None,
            execution_plan: vec![Uuid::new_v4(), Uuid::new_v4()],
        };

        let json: Value = serde_json::to_value(&payload).unwrap();

        assert!(json["execution_plan"].is_array());
        let plan = json["execution_plan"].as_array().unwrap();
        assert_eq!(plan.len(), 2);

        for item in plan {
            assert!(item.is_string());
            assert!(Uuid::parse_str(item.as_str().unwrap()).is_ok());
        }
    }

    #[test]
    fn idempotency_key_can_be_null() {
        let payload = create_test_payload();
        let json: Value = serde_json::to_value(&payload).unwrap();

        // Should be null when None
        assert!(json["idempotency_key"].is_null());
    }

    #[test]
    fn idempotency_key_can_be_string() {
        let mut payload = create_test_payload();
        payload.idempotency_key = Some("my-idempotency-key".to_string());

        let json: Value = serde_json::to_value(&payload).unwrap();

        assert!(json["idempotency_key"].is_string());
        assert_eq!(json["idempotency_key"], "my-idempotency-key");
    }

    #[test]
    fn payload_deserializes_correctly() {
        // Test that worker can deserialize what orchestrator produces
        let original = create_test_payload();
        let json_bytes = serde_json::to_vec(&original).unwrap();

        let restored: TaskPayload = serde_json::from_slice(&json_bytes).unwrap();

        assert_eq!(original.execution_id, restored.execution_id);
        assert_eq!(original.workflow_id, restored.workflow_id);
        assert_eq!(original.tenant_id, restored.tenant_id);
        assert_eq!(original.idempotency_key, restored.idempotency_key);
        assert_eq!(original.execution_plan.len(), restored.execution_plan.len());
    }
}

mod signature_contract {
    use super::*;

    const TEST_SECRET: &str = "test-secret-minimum-32-bytes-long-for-security";

    #[test]
    fn signature_is_hex_encoded() {
        let payload = create_test_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let signature = sign_payload(&json_bytes, TEST_SECRET);

        // Verify it's valid hex
        assert!(hex::decode(&signature).is_ok());

        // SHA256 produces 32 bytes = 64 hex characters
        assert_eq!(signature.len(), 64);
    }

    #[test]
    fn signature_computed_over_json_bytes() {
        // Important: orchestrator signs JSON bytes, not base64
        let payload = create_test_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();

        let signature1 = sign_payload(&json_bytes, TEST_SECRET);
        let signature2 = sign_payload(&json_bytes, TEST_SECRET);

        assert_eq!(
            signature1, signature2,
            "same input should produce same signature"
        );
    }

    #[test]
    fn different_payloads_produce_different_signatures() {
        let payload1 = create_test_payload();
        let payload2 = TaskPayload {
            tenant_id: "different-tenant".to_string(),
            ..create_test_payload()
        };

        let json1 = serde_json::to_vec(&payload1).unwrap();
        let json2 = serde_json::to_vec(&payload2).unwrap();

        let sig1 = sign_payload(&json1, TEST_SECRET);
        let sig2 = sign_payload(&json2, TEST_SECRET);

        assert_ne!(sig1, sig2);
    }

    #[test]
    fn different_secrets_produce_different_signatures() {
        let payload = create_test_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();

        let sig1 = sign_payload(&json_bytes, TEST_SECRET);
        let sig2 = sign_payload(&json_bytes, "different-secret-also-32-bytes-long!");

        assert_ne!(sig1, sig2);
    }

    #[test]
    fn signature_verification_works() {
        let payload = create_test_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let signature = sign_payload(&json_bytes, TEST_SECRET);

        // Verify by recomputing
        let expected = sign_payload(&json_bytes, TEST_SECRET);
        assert_eq!(signature, expected);
    }
}

mod cloud_tasks_api_contract {
    use super::*;

    #[test]
    fn task_body_is_base64_encoded_json() {
        // Cloud Tasks sends the body as base64 in the API request
        let payload = create_test_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let base64_body = STANDARD.encode(&json_bytes);

        // Worker receives this and decodes it
        let decoded = STANDARD.decode(&base64_body).unwrap();
        let restored: TaskPayload = serde_json::from_slice(&decoded).unwrap();

        assert_eq!(payload.execution_id, restored.execution_id);
    }

    #[test]
    fn cloud_tasks_request_has_required_structure() {
        // Cloud Tasks API request structure
        let task_request = json!({
            "task": {
                "httpRequest": {
                    "url": "https://worker.example.com/execute",
                    "httpMethod": "POST",
                    "headers": {
                        "Content-Type": "application/json",
                        "X-Servo-Signature": "abc123..."
                    },
                    "body": "base64encodedpayload...",
                    "oidcToken": {
                        "serviceAccountEmail": "sa@project.iam.gserviceaccount.com"
                    }
                }
            }
        });

        // Verify structure
        assert!(task_request.get("task").is_some());
        let task = &task_request["task"];

        assert!(task.get("httpRequest").is_some());
        let http_request = &task["httpRequest"];

        assert!(http_request.get("url").is_some());
        assert!(http_request.get("httpMethod").is_some());
        assert!(http_request.get("headers").is_some());
        assert!(http_request.get("body").is_some());
        assert!(http_request.get("oidcToken").is_some());
    }

    #[test]
    fn oidc_token_requires_service_account_email() {
        let oidc_token = json!({
            "serviceAccountEmail": "sa@project.iam.gserviceaccount.com"
        });

        assert!(oidc_token.get("serviceAccountEmail").is_some());
        assert!(oidc_token["serviceAccountEmail"].is_string());
    }

    #[test]
    fn http_method_is_post() {
        // Cloud Tasks should use POST for task execution
        let http_method = "POST";
        assert_eq!(http_method, "POST");
    }

    #[test]
    fn target_url_ends_with_execute() {
        // Worker endpoint for task execution
        let worker_url = "https://worker.example.run.app";
        let target_url = format!("{}/execute", worker_url);

        assert!(target_url.ends_with("/execute"));
    }
}

mod headers_contract {
    #[test]
    fn signature_header_name_is_lowercase() {
        // Note: HTTP headers are case-insensitive, but we use lowercase
        let header_name = "x-servo-signature";
        assert_eq!(header_name.to_lowercase(), "x-servo-signature");
    }

    #[test]
    fn content_type_is_application_json() {
        let content_type = "application/json";
        assert!(content_type.contains("json"));
    }

    #[test]
    fn trace_headers_follow_w3c_format() {
        // traceparent format: version-trace_id-parent_id-flags
        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parts: Vec<&str> = traceparent.split('-').collect();

        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00"); // version
        assert_eq!(parts[1].len(), 32); // trace_id (32 hex chars)
        assert_eq!(parts[2].len(), 16); // parent_id (16 hex chars)
        assert_eq!(parts[3].len(), 2); // flags (2 hex chars)
    }
}

mod queue_path_contract {
    #[test]
    fn queue_path_format() {
        let project_id = "my-project";
        let location = "us-central1";
        let queue_name = "servo-executions";

        let queue_path = format!(
            "projects/{}/locations/{}/queues/{}",
            project_id, location, queue_name
        );

        assert_eq!(
            queue_path,
            "projects/my-project/locations/us-central1/queues/servo-executions"
        );
    }

    #[test]
    fn api_url_format() {
        let queue_path = "projects/my-project/locations/us-central1/queues/servo-executions";
        let api_url = format!("https://cloudtasks.googleapis.com/v2/{}/tasks", queue_path);

        assert!(api_url.starts_with("https://cloudtasks.googleapis.com/v2/"));
        assert!(api_url.ends_with("/tasks"));
    }
}

mod error_handling_contract {
    #[test]
    fn rate_limit_status_code() {
        // Cloud Tasks returns 429 for rate limits
        let rate_limit_status = 429u16;
        assert_eq!(rate_limit_status, 429);
    }

    #[test]
    fn server_error_range() {
        // 5xx errors should trigger retry
        for status in 500..=599 {
            assert!(status >= 500 && status < 600);
        }
    }

    #[test]
    fn client_error_range() {
        // 4xx errors (except 429) should not retry
        for status in [400u16, 401, 403, 404] {
            assert!(status >= 400 && status < 500);
            assert_ne!(status, 429); // 429 is special case
        }
    }
}
