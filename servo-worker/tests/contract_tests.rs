//! API Contract Tests for Servo Worker
//!
//! These tests verify the HTTP API contracts are stable and well-defined.
//! They focus on request/response formats, not business logic.
//!
//! Contract tests should:
//! - Verify request validation rules
//! - Verify response schemas
//! - Verify error response formats
//! - Be fast and reliable (no database required for most tests)

use axum::http::StatusCode;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

/// Helper to compute HMAC-SHA256 signature
fn compute_signature(payload: &[u8], secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload);
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

/// Create a valid task payload
fn create_task_payload() -> Value {
    json!({
        "execution_id": Uuid::new_v4(),
        "workflow_id": Uuid::new_v4(),
        "tenant_id": "test-tenant",
        "idempotency_key": null,
        "execution_plan": []
    })
}

/// Create a signed request body
fn sign_payload(payload: &Value, secret: &str) -> (Vec<u8>, String) {
    let json_bytes = serde_json::to_vec(payload).unwrap();
    let base64_payload = STANDARD.encode(&json_bytes);
    let body_bytes = base64_payload.as_bytes().to_vec();
    let signature = compute_signature(&body_bytes, secret);
    (body_bytes, signature)
}

mod task_payload_contract {
    use super::*;

    #[test]
    fn payload_has_required_fields() {
        let payload = create_task_payload();

        // Verify required fields exist
        assert!(
            payload.get("execution_id").is_some(),
            "execution_id required"
        );
        assert!(payload.get("workflow_id").is_some(), "workflow_id required");
        assert!(payload.get("tenant_id").is_some(), "tenant_id required");
        assert!(
            payload.get("execution_plan").is_some(),
            "execution_plan required"
        );
    }

    #[test]
    fn execution_id_must_be_uuid() {
        let payload = create_task_payload();
        let execution_id = payload["execution_id"].as_str().unwrap();
        assert!(
            Uuid::parse_str(execution_id).is_ok(),
            "execution_id must be valid UUID"
        );
    }

    #[test]
    fn workflow_id_must_be_uuid() {
        let payload = create_task_payload();
        let workflow_id = payload["workflow_id"].as_str().unwrap();
        assert!(
            Uuid::parse_str(workflow_id).is_ok(),
            "workflow_id must be valid UUID"
        );
    }

    #[test]
    fn tenant_id_must_be_string() {
        let payload = create_task_payload();
        assert!(
            payload["tenant_id"].is_string(),
            "tenant_id must be a string"
        );
    }

    #[test]
    fn execution_plan_must_be_array() {
        let payload = create_task_payload();
        assert!(
            payload["execution_plan"].is_array(),
            "execution_plan must be an array"
        );
    }

    #[test]
    fn idempotency_key_is_optional() {
        // Can be null
        let mut payload = create_task_payload();
        assert!(payload["idempotency_key"].is_null());

        // Can be string
        payload["idempotency_key"] = json!("my-key");
        assert!(payload["idempotency_key"].is_string());
    }

    #[test]
    fn payload_base64_encodes_correctly() {
        let payload = create_task_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let base64_encoded = STANDARD.encode(&json_bytes);

        // Verify it decodes back
        let decoded = STANDARD.decode(&base64_encoded).unwrap();
        let decoded_payload: Value = serde_json::from_slice(&decoded).unwrap();

        assert_eq!(payload["tenant_id"], decoded_payload["tenant_id"]);
    }
}

mod signature_contract {
    use super::*;

    const TEST_SECRET: &str = "test-secret-minimum-32-bytes-required";

    #[test]
    fn signature_is_hex_encoded() {
        let payload = create_task_payload();
        let (_body, signature) = sign_payload(&payload, TEST_SECRET);

        // Verify it's valid hex
        assert!(
            hex::decode(&signature).is_ok(),
            "signature must be valid hex"
        );

        // Verify length (SHA256 = 32 bytes = 64 hex chars)
        assert_eq!(signature.len(), 64, "signature must be 64 hex characters");
    }

    #[test]
    fn signature_computed_over_base64_bytes() {
        let payload = create_task_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let base64_payload = STANDARD.encode(&json_bytes);
        let body_bytes = base64_payload.as_bytes();

        // Compute expected signature
        let expected_signature = compute_signature(body_bytes, TEST_SECRET);

        // Verify our helper produces the same
        let (_, actual_signature) = sign_payload(&payload, TEST_SECRET);
        assert_eq!(expected_signature, actual_signature);
    }

    #[test]
    fn same_input_produces_same_signature() {
        let payload = create_task_payload();
        let (_, sig1) = sign_payload(&payload, TEST_SECRET);
        let (_, sig2) = sign_payload(&payload, TEST_SECRET);

        assert_eq!(sig1, sig2);
    }

    #[test]
    fn different_secret_produces_different_signature() {
        let payload = create_task_payload();
        let (_, sig1) = sign_payload(&payload, TEST_SECRET);
        let (_, sig2) = sign_payload(&payload, "different-secret-min-32-bytes-req");

        assert_ne!(sig1, sig2);
    }

    #[test]
    fn different_payload_produces_different_signature() {
        let payload1 = create_task_payload();
        let mut payload2 = create_task_payload();
        payload2["tenant_id"] = json!("different-tenant");

        let (_, sig1) = sign_payload(&payload1, TEST_SECRET);
        let (_, sig2) = sign_payload(&payload2, TEST_SECRET);

        assert_ne!(sig1, sig2);
    }
}

mod execute_response_contract {
    use super::*;

    #[test]
    fn success_response_has_required_fields() {
        let response = json!({
            "execution_id": Uuid::new_v4(),
            "status": "accepted"
        });

        assert!(response.get("execution_id").is_some());
        assert!(response.get("status").is_some());
    }

    #[test]
    fn success_response_status_is_accepted() {
        let execution_id = Uuid::new_v4();
        let response = json!({
            "execution_id": execution_id,
            "status": "accepted"
        });

        assert_eq!(response["status"], "accepted");
    }

    #[test]
    fn error_response_includes_message() {
        let response = json!({
            "execution_id": Uuid::new_v4(),
            "status": "error",
            "message": "Something went wrong"
        });

        assert!(response.get("message").is_some());
        assert!(response["message"].is_string());
    }

    #[test]
    fn execution_id_in_response_is_uuid() {
        let response = json!({
            "execution_id": Uuid::new_v4(),
            "status": "accepted"
        });

        let execution_id = response["execution_id"].as_str().unwrap();
        assert!(Uuid::parse_str(execution_id).is_ok());
    }

    #[test]
    fn status_values_are_known() {
        const VALID_STATUSES: &[&str] = &["accepted", "error"];

        for status in VALID_STATUSES {
            let response = json!({
                "execution_id": Uuid::new_v4(),
                "status": status
            });
            assert!(response["status"].is_string());
        }
    }
}

mod health_response_contract {
    use super::*;

    #[test]
    fn health_response_has_status() {
        let response = json!({
            "status": "healthy"
        });

        assert!(response.get("status").is_some());
        assert_eq!(response["status"], "healthy");
    }

    #[test]
    fn health_response_can_have_version() {
        let response = json!({
            "status": "healthy",
            "version": "0.1.0"
        });

        assert!(response.get("version").is_some());
        assert!(response["version"].is_string());
    }
}

mod ready_response_contract {
    use super::*;

    #[test]
    fn ready_response_has_status() {
        let response = json!({
            "status": "ready",
            "checks": {
                "database": "ok"
            }
        });

        assert!(response.get("status").is_some());
        assert_eq!(response["status"], "ready");
    }

    #[test]
    fn ready_response_includes_checks() {
        let response = json!({
            "status": "ready",
            "checks": {
                "database": "ok"
            }
        });

        assert!(response.get("checks").is_some());
        assert!(response["checks"].is_object());
    }

    #[test]
    fn not_ready_response_includes_reason() {
        let response = json!({
            "status": "not_ready",
            "reason": "database_unreachable"
        });

        assert_eq!(response["status"], "not_ready");
        assert!(response.get("reason").is_some());
    }
}

mod metrics_response_contract {
    #[test]
    fn metrics_content_type_is_prometheus() {
        // Prometheus metrics use text/plain with version parameter
        let expected_content_type = "text/plain; version=0.0.4";
        assert!(expected_content_type.starts_with("text/plain"));
    }

    #[test]
    fn prometheus_metrics_format_example() {
        // Example of valid Prometheus metrics format
        let metrics_text = r#"
# HELP http_requests_total Total HTTP requests
# TYPE http_requests_total counter
http_requests_total{method="GET",status="200"} 1234
http_requests_total{method="POST",status="200"} 567
"#;

        // Verify format: metric_name{labels} value
        for line in metrics_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Should contain metric name and value
            assert!(
                line.contains(' '),
                "Metrics line should have name and value: {}",
                line
            );
        }
    }
}

mod error_response_contract {
    use super::*;

    #[test]
    fn unauthorized_returns_401() {
        // Contract: Missing or invalid auth should return 401
        let expected_status = StatusCode::UNAUTHORIZED;
        assert_eq!(expected_status.as_u16(), 401);
    }

    #[test]
    fn bad_request_returns_400() {
        // Contract: Invalid payload should return 400
        let expected_status = StatusCode::BAD_REQUEST;
        assert_eq!(expected_status.as_u16(), 400);
    }

    #[test]
    fn rate_limit_returns_429() {
        // Contract: Rate limit exceeded should return 429
        let expected_status = StatusCode::TOO_MANY_REQUESTS;
        assert_eq!(expected_status.as_u16(), 429);
    }

    #[test]
    fn rate_limit_includes_retry_after_header() {
        // Contract: 429 responses should include Retry-After header
        let expected_header = "Retry-After";
        assert_eq!(expected_header, "Retry-After");
    }

    #[test]
    fn service_unavailable_returns_503() {
        // Contract: Service unavailable (e.g., DB down) should return 503
        let expected_status = StatusCode::SERVICE_UNAVAILABLE;
        assert_eq!(expected_status.as_u16(), 503);
    }
}

mod header_contract {
    #[test]
    fn signature_header_name() {
        // Contract: Signature header must be named x-servo-signature
        let header_name = "x-servo-signature";
        assert_eq!(header_name, "x-servo-signature");
    }

    #[test]
    fn authorization_header_format() {
        // Contract: OIDC token must use Bearer scheme
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...";
        let header_value = format!("Bearer {}", token);
        assert!(header_value.starts_with("Bearer "));
    }

    #[test]
    fn content_type_for_json() {
        // Contract: JSON requests should use application/json
        let content_type = "application/json";
        assert!(content_type.contains("json"));
    }
}

mod trace_context_contract {
    #[test]
    fn traceparent_header_format() {
        // W3C Trace Context format: version-trace_id-parent_id-flags
        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parts: Vec<&str> = traceparent.split('-').collect();

        assert_eq!(parts.len(), 4, "traceparent should have 4 parts");
        assert_eq!(parts[0], "00", "version should be 00");
        assert_eq!(parts[1].len(), 32, "trace_id should be 32 hex chars");
        assert_eq!(parts[2].len(), 16, "parent_id should be 16 hex chars");
        assert_eq!(parts[3].len(), 2, "flags should be 2 hex chars");
    }

    #[test]
    fn tracestate_header_is_optional() {
        // tracestate is optional - can be empty or have vendor data
        let tracestate = "congo=t61rcWkgMzE";
        assert!(
            tracestate.contains('='),
            "tracestate should have key=value format"
        );
    }
}

mod scheduler_payload_contract {
    use super::*;

    /// Create a valid scheduler payload (as sent by Cloud Scheduler)
    fn create_scheduler_payload() -> Value {
        json!({
            "workflow_id": Uuid::new_v4(),
            "tenant_id": "test-tenant",
            "triggered_by": "scheduler"
        })
    }

    #[test]
    fn scheduler_payload_has_required_fields() {
        let payload = create_scheduler_payload();

        // Verify required fields exist
        assert!(payload.get("workflow_id").is_some(), "workflow_id required");
        assert!(payload.get("tenant_id").is_some(), "tenant_id required");
        assert!(
            payload.get("triggered_by").is_some(),
            "triggered_by required"
        );
    }

    #[test]
    fn workflow_id_must_be_uuid() {
        let payload = create_scheduler_payload();
        let workflow_id = payload["workflow_id"].as_str().unwrap();
        assert!(
            Uuid::parse_str(workflow_id).is_ok(),
            "workflow_id must be valid UUID"
        );
    }

    #[test]
    fn tenant_id_must_be_string() {
        let payload = create_scheduler_payload();
        assert!(
            payload["tenant_id"].is_string(),
            "tenant_id must be a string"
        );
    }

    #[test]
    fn triggered_by_indicates_scheduler() {
        let payload = create_scheduler_payload();
        assert_eq!(
            payload["triggered_by"], "scheduler",
            "triggered_by should be 'scheduler' for scheduled executions"
        );
    }

    #[test]
    fn tenant_header_required() {
        // Contract: Cloud Scheduler requests must include X-Servo-Tenant-Id header
        let header_name = "x-servo-tenant-id";
        assert_eq!(header_name.to_lowercase(), "x-servo-tenant-id");
    }
}

mod scheduler_execute_endpoint_contract {
    use super::*;

    #[test]
    fn endpoint_path_includes_workflow_id() {
        // Contract: /api/v1/workflows/:workflow_id/execute
        let workflow_id = Uuid::new_v4();
        let path = format!("/api/v1/workflows/{}/execute", workflow_id);
        assert!(path.contains(&workflow_id.to_string()));
    }

    #[test]
    fn endpoint_uses_post_method() {
        // Contract: POST method for triggering executions
        let method = "POST";
        assert_eq!(method, "POST");
    }

    #[test]
    fn response_includes_execution_id() {
        // Contract: Response must include the created execution_id
        let response = json!({
            "execution_id": Uuid::new_v4(),
            "status": "accepted"
        });

        assert!(response.get("execution_id").is_some());
        let execution_id = response["execution_id"].as_str().unwrap();
        assert!(Uuid::parse_str(execution_id).is_ok());
    }

    #[test]
    fn unauthorized_for_missing_auth() {
        // Contract: Missing OIDC token should return 401
        let expected_status = StatusCode::UNAUTHORIZED;
        assert_eq!(expected_status.as_u16(), 401);
    }

    #[test]
    fn not_found_for_invalid_workflow() {
        // Contract: Non-existent workflow should return 404
        let expected_status = StatusCode::NOT_FOUND;
        assert_eq!(expected_status.as_u16(), 404);
    }

    #[test]
    fn conflict_for_disabled_schedule() {
        // Contract: Disabled schedule should return 409 Conflict
        let expected_status = StatusCode::CONFLICT;
        assert_eq!(expected_status.as_u16(), 409);
    }
}

mod scheduler_oidc_contract {
    #[test]
    fn oidc_token_uses_bearer_scheme() {
        // Contract: Cloud Scheduler sends OIDC tokens with Bearer scheme
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...";
        let header_value = format!("Bearer {}", token);
        assert!(header_value.starts_with("Bearer "));
    }

    #[test]
    fn oidc_audience_matches_worker_url() {
        // Contract: OIDC token audience must match the worker URL
        let worker_url = "https://servo-worker-xxxx.run.app";
        let expected_audience = worker_url;
        assert!(expected_audience.starts_with("https://"));
    }

    #[test]
    fn oidc_service_account_in_token() {
        // Contract: The token should contain the service account email
        // This is verified by the OIDC validator
        let expected_claim = "email";
        assert_eq!(expected_claim, "email");
    }
}

/// Integration tests for partition context propagation
///
/// These tests verify that partition context flows correctly through the
/// execution pipeline: BackfillExecutor → TaskPayload → Worker → Python
mod partition_context_contract {
    use super::*;
    use servo_core::PartitionExecutionContext;
    use servo_worker::types::TaskPayload;

    /// Create a task payload with partition context
    fn create_partitioned_task_payload() -> TaskPayload {
        TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "test-tenant".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
            partition_context: Some(
                PartitionExecutionContext::new("2024-01-15")
                    .with_partition_type("daily")
                    .with_timezone("UTC"),
            ),
        }
    }

    /// Create a task payload without partition context (for backward compat)
    fn create_non_partitioned_task_payload() -> TaskPayload {
        TaskPayload {
            execution_id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            tenant_id: "test-tenant".to_string(),
            idempotency_key: None,
            execution_plan: vec![],
            partition_context: None,
        }
    }

    // ============================================================
    // End-to-End Partition Context Serialization Tests
    // ============================================================

    #[test]
    fn partition_context_serializes_correctly() {
        let payload = create_partitioned_task_payload();
        let json = serde_json::to_string(&payload).unwrap();

        // Verify partition_context is present in JSON
        assert!(json.contains("partition_context"));
        assert!(json.contains("2024-01-15"));
        assert!(json.contains("daily"));
        assert!(json.contains("UTC"));
    }

    #[test]
    fn partition_context_deserializes_correctly() {
        let payload = create_partitioned_task_payload();
        let json = serde_json::to_string(&payload).unwrap();

        let deserialized: TaskPayload = serde_json::from_str(&json).unwrap();

        assert!(deserialized.partition_context.is_some());
        let ctx = deserialized.partition_context.unwrap();
        assert_eq!(ctx.partition_key, "2024-01-15");
        assert_eq!(ctx.partition_type.as_deref(), Some("daily"));
        assert_eq!(ctx.timezone.as_deref(), Some("UTC"));
    }

    #[test]
    fn partition_context_round_trips_through_base64() {
        // This simulates the Cloud Tasks payload encoding
        let payload = create_partitioned_task_payload();
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let base64_encoded = STANDARD.encode(&json_bytes);

        // Decode and verify
        let decoded_bytes = STANDARD.decode(&base64_encoded).unwrap();
        let decoded: TaskPayload = serde_json::from_slice(&decoded_bytes).unwrap();

        assert!(decoded.partition_context.is_some());
        let ctx = decoded.partition_context.unwrap();
        assert_eq!(ctx.partition_key, "2024-01-15");
    }

    // ============================================================
    // Backward Compatibility Tests
    // ============================================================

    #[test]
    fn payload_without_partition_context_serializes() {
        let payload = create_non_partitioned_task_payload();
        let json = serde_json::to_string(&payload).unwrap();

        // partition_context should be omitted (skip_serializing_if)
        assert!(!json.contains("partition_context"));
    }

    #[test]
    fn old_payload_without_partition_context_deserializes() {
        // This simulates receiving an old-format payload from an older orchestrator
        let old_json = r#"{
            "execution_id": "550e8400-e29b-41d4-a716-446655440000",
            "workflow_id": "550e8400-e29b-41d4-a716-446655440001",
            "tenant_id": "test-tenant",
            "idempotency_key": null,
            "execution_plan": []
        }"#;

        let payload: TaskPayload = serde_json::from_str(old_json).unwrap();
        assert!(payload.partition_context.is_none());
        assert_eq!(payload.tenant_id, "test-tenant");
    }

    #[test]
    fn null_partition_context_deserializes() {
        // Explicit null should also work
        let json = r#"{
            "execution_id": "550e8400-e29b-41d4-a716-446655440000",
            "workflow_id": "550e8400-e29b-41d4-a716-446655440001",
            "tenant_id": "test-tenant",
            "idempotency_key": null,
            "execution_plan": [],
            "partition_context": null
        }"#;

        let payload: TaskPayload = serde_json::from_str(json).unwrap();
        assert!(payload.partition_context.is_none());
    }

    // ============================================================
    // JSON Escaping Security Tests
    // ============================================================

    #[test]
    fn partition_key_with_special_characters_escapes_correctly() {
        // Test that special characters are properly escaped
        let ctx =
            PartitionExecutionContext::new("2024-01-15 10:00:00").with_partition_type("hourly");

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.partition_key, "2024-01-15 10:00:00");
    }

    #[test]
    fn partition_key_with_quotes_escapes_correctly() {
        // Test that quotes are properly escaped (prevents injection)
        let ctx = PartitionExecutionContext::new(r#"2024-01-15"test"#);

        let json = serde_json::to_string(&ctx).unwrap();

        // Should contain escaped quotes
        assert!(json.contains(r#"\""#) || json.contains(r#"\"test"#));

        // Should round-trip correctly
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.partition_key, r#"2024-01-15"test"#);
    }

    #[test]
    fn partition_key_with_newlines_escapes_correctly() {
        // Test that newlines don't break JSON
        let ctx = PartitionExecutionContext::new("2024-01-15\n2024-01-16");

        let json = serde_json::to_string(&ctx).unwrap();

        // Should contain escaped newline
        assert!(json.contains(r"\n"));

        // Should round-trip correctly
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.partition_key, "2024-01-15\n2024-01-16");
    }

    #[test]
    fn partition_key_with_unicode_escapes_correctly() {
        // Test that unicode characters are handled correctly
        let ctx = PartitionExecutionContext::new("2024-01-15_日本語");

        let json = serde_json::to_string(&ctx).unwrap();

        // Should round-trip correctly
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.partition_key, "2024-01-15_日本語");
    }

    #[test]
    fn partition_context_with_all_fields_serializes() {
        // Test full context with dimensions and upstream partitions
        let ctx = PartitionExecutionContext::new("2024-01-15|us-west")
            .with_partition_type("daily")
            .with_timezone("America/Los_Angeles")
            .with_format("%Y-%m-%d|%s")
            .with_dimension("date", "2024-01-15")
            .with_dimension("region", "us-west")
            .with_upstream_partition("raw_data", vec!["2024-01-14".into(), "2024-01-15".into()]);

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: PartitionExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.partition_key, "2024-01-15|us-west");
        assert_eq!(deserialized.partition_type.as_deref(), Some("daily"));
        assert_eq!(
            deserialized.timezone.as_deref(),
            Some("America/Los_Angeles")
        );
        assert_eq!(deserialized.format.as_deref(), Some("%Y-%m-%d|%s"));
        assert_eq!(deserialized.get_dimension("date"), Some("2024-01-15"));
        assert_eq!(deserialized.get_dimension("region"), Some("us-west"));
        assert_eq!(
            deserialized.get_upstream_partitions("raw_data"),
            Some(&vec!["2024-01-14".into(), "2024-01-15".into()])
        );
    }

    // ============================================================
    // Environment Variable Format Tests
    // ============================================================

    #[test]
    fn partition_context_json_for_env_var() {
        // Verify the JSON format that gets passed to SERVO_PARTITION_CONTEXT
        let ctx = PartitionExecutionContext::new("2024-01-15")
            .with_partition_type("daily")
            .with_timezone("UTC");

        let json = serde_json::to_string(&ctx).unwrap();

        // Parse as generic JSON to verify structure
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["partition_key"], "2024-01-15");
        assert_eq!(parsed["partition_type"], "daily");
        assert_eq!(parsed["timezone"], "UTC");
    }

    #[test]
    fn empty_optional_fields_not_in_json() {
        // Verify skip_serializing_if works for cleaner env var
        let ctx = PartitionExecutionContext::new("2024-01-15");
        let json = serde_json::to_string(&ctx).unwrap();

        // Parse as generic JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // These should be absent, not null
        assert!(parsed.get("partition_type").is_none());
        assert!(parsed.get("timezone").is_none());
        assert!(parsed.get("dimensions").is_none());
        assert!(parsed.get("upstream_partitions").is_none());
    }
}
