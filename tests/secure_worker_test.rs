//! End-to-end tests for the secure worker with OIDC + HMAC validation
//!
//! These tests validate the complete worker HTTP endpoint including:
//! - OIDC token validation (caller identity)
//! - HMAC signature verification (payload integrity)
//! - Full workflow execution lifecycle
//!
//! ## Test Infrastructure
//! - Ephemeral PostgreSQL via testcontainers
//! - Mock JWKS server for OIDC validation
//! - Real Axum HTTP server with complete middleware stack
//! - Test JWT token generation with matching keys
//!
//! ## Running these tests
//! ```bash
//! cargo test --test secure_worker_test -- --ignored
//! ```

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use servo_storage::{postgres::PostgresStorage, TenantId};
use servo_worker::{
    config::OidcConfig,
    executor::WorkflowExecutor,
    handler::{execute_handler, AppState},
    oidc::OidcValidator,
};
use std::sync::Arc;
use std::time::Duration;
use testcontainers::{clients::Cli, core::WaitFor, Container, GenericImage};
use tower::Service;
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

// Helper trait to add oneshot to Router
trait RouterExt {
    async fn oneshot_call(self, req: Request<Body>) -> Result<axum::response::Response, axum::Error>;
}

impl RouterExt for Router {
    async fn oneshot_call(mut self, req: Request<Body>) -> Result<axum::response::Response, axum::Error> {
        self.call(req).await
    }
}

/// Test JWT signing key (must match JWKS public key)
fn test_signing_key() -> EncodingKey {
    // Test-only RSA private key (DO NOT use in production)
    let pem = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDS/HtqCh5sZxBK
64+IsldmO032ed2tCZtcSmzZqIAVtaEzvwuFbHhxtt8AC1VPzrPC7VErto8UXG6E
NHUvq1Khz8EkQI95tYpFeMFkKIVXifeiSeOEyy2fri1n/Zb7kmwZjgdzmf3IFcCv
CX3eWq3v9E3nDoJ/SHhDJDm/7rlgaNBHT8UNbZC/Opjfrr5BAwM5x5Kqs7PCiWYJ
2G/XO3dM4HQGR87uqjEL0S+Ya456593UJsy6lsg9PKjS8q3ZLfmxU1oQCOLzEBYe
lnmbtHzOJRXdjXEcK91z5LCDR2kPhv8QZ4iKm8NC7NYxeOPnLBQrq/pBIFPGsGqS
cyp6gyoMAgMBAAECggEAJR5bY5D6TcIZ6okSiaDOKLjhcFqVaw7rNawRokvnZ8A1
sMmDQ8qkf4i7lI/5r24Z7f6qMVpGCNWS8fRQQzG1BL8E6ggc8xnHmPGh5KVc6W4t
EL3fQzWjB8xD9VL7vCu3cLt0hAJLxRQJU8L3NvqCfL3nCfL7hKL6hcC5K9v8rBdC
hKL7vLgCxMqBdCT4rFsQvCxJqRxKuL5vDxL5rJL6xCqBvQxDhLvBcK4nKL8vNqBo
L9vQxDhL6vBcKL5nK8vNqBoL9vQxDvL6vBcKL5nK8vNqBoL9vQxDvL6vBcKL5nK8
vNqBoL9vQxDvL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoQKBgQDv7dJ6r5e
vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcK
L5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoQKBgQDg
vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcK
L5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoQKBgQCv
NqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcK
L5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoQKBgCv
NqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcK
L5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoQKBgHv
NqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcK
L5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBoL9vQxDhL6vBcKL5nK8vNqBo
-----END PRIVATE KEY-----"#;

    EncodingKey::from_rsa_pem(pem.as_bytes()).expect("Failed to create test signing key")
}

/// Test JWKS public key (must match signing key)
fn test_jwks() -> serde_json::Value {
    json!({
        "keys": [
            {
                "kty": "RSA",
                "use": "sig",
                "kid": "e2e-test-key",
                "alg": "RS256",
                "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
                "e": "AQAB"
            }
        ]
    })
}

/// Generate a valid test OIDC token
fn create_test_token(audience: &str, service_account: &str) -> String {
    let claims = json!({
        "iss": "https://accounts.google.com",
        "aud": audience,
        "sub": service_account,
        "email": service_account,
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        "iat": chrono::Utc::now().timestamp(),
    });

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("e2e-test-key".to_string());

    encode(&header, &claims, &test_signing_key()).expect("Failed to encode test token")
}

/// Generate HMAC signature for payload
fn sign_payload(payload: &[u8], secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

/// E2E test environment with all components
struct SecureWorkerTestEnv<'a> {
    _docker: &'a Cli,
    _postgres_container: Container<'a, GenericImage>,
    _jwks_server: MockServer,
    storage: Arc<PostgresStorage>,
    tenant: TenantId,
    app: Router,
    hmac_secret: String,
    audience: String,
}

impl<'a> SecureWorkerTestEnv<'a> {
    /// Set up complete E2E test environment
    async fn setup(docker: &'a Cli) -> anyhow::Result<Self> {
        // 1. Start PostgreSQL container
        let postgres_image = GenericImage::new("postgres", "15-alpine")
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_DB", "servo_e2e")
            .with_env_var("POSTGRES_USER", "servo_e2e")
            .with_env_var("POSTGRES_PASSWORD", "servo_e2e");

        let postgres_container = docker.run(postgres_image);
        let postgres_port = postgres_container.get_host_port_ipv4(5432);

        let database_url = format!(
            "postgresql://servo_e2e:servo_e2e@localhost:{}/servo_e2e",
            postgres_port
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let storage = Arc::new(PostgresStorage::new(&database_url).await?);

        // 2. Start mock JWKS server
        let jwks_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth2/v3/certs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(test_jwks()))
            .mount(&jwks_server)
            .await;

        // 3. Configure OIDC validator
        let audience = "https://worker.servo.test".to_string();
        let jwks_url = format!("{}/oauth2/v3/certs", jwks_server.uri());

        let oidc_config = OidcConfig {
            enabled: true,
            issuer: "https://accounts.google.com".to_string(),
            jwks_url,
            expected_audience: audience.clone(),
        };

        let oidc_validator = Arc::new(OidcValidator::new(oidc_config)?);
        oidc_validator.fetch_and_cache_jwks().await?;

        // 4. Create worker components
        let executor = Arc::new(WorkflowExecutor::new(
            storage.clone(),
            Duration::from_secs(600),
        ));

        let hmac_secret = "e2e-test-hmac-secret-do-not-use-in-production".to_string();

        let state = AppState {
            executor,
            hmac_secret: hmac_secret.clone(),
            oidc_validator,
        };

        // 5. Build worker app
        let app = Router::new()
            .route("/execute", axum::routing::post(execute_handler))
            .with_state(state);

        let tenant = TenantId::new(format!("e2e-test-{}", Uuid::new_v4()));

        Ok(Self {
            _docker: docker,
            _postgres_container: postgres_container,
            _jwks_server: jwks_server,
            storage,
            tenant,
            app,
            hmac_secret,
            audience,
        })
    }

    /// Create an authenticated execute request
    fn create_execute_request(
        &self,
        execution_id: Uuid,
        workflow_id: Uuid,
        include_oidc: bool,
        include_hmac: bool,
    ) -> Request<Body> {
        let payload = json!({
            "execution_id": execution_id,
            "workflow_id": workflow_id,
            "tenant_id": self.tenant.to_string(),
            "idempotency_key": null,
            "execution_plan": []
        });

        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let base64_payload = STANDARD.encode(&json_bytes);

        let mut builder = Request::builder()
            .uri("/execute")
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");

        // Add OIDC token if requested
        if include_oidc {
            let token = create_test_token(&self.audience, "test-service@project.iam.gserviceaccount.com");
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {}", token));
        }

        // Add HMAC signature if requested
        if include_hmac {
            let signature = sign_payload(base64_payload.as_bytes(), &self.hmac_secret);
            builder = builder.header("x-servo-signature", signature);
        }

        builder.body(Body::from(base64_payload)).unwrap()
    }
}

#[tokio::test]
#[ignore] // Requires Docker for testcontainers
async fn test_secure_worker_accepts_valid_request() {
    let docker = Cli::default();
    let env = SecureWorkerTestEnv::setup(&docker)
        .await
        .expect("Failed to setup test environment");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Create request with both OIDC and HMAC
    let request = env.create_execute_request(execution_id, workflow_id, true, true);

    // Send request to worker
    let response = env
        .app
        .clone()
        .oneshot_call(request)
        .await
        .expect("Failed to send request");

    // Should accept valid request
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Worker should accept request with valid OIDC + HMAC"
    );
}

#[tokio::test]
#[ignore] // Requires Docker for testcontainers
async fn test_secure_worker_rejects_missing_oidc_token() {
    let docker = Cli::default();
    let env = SecureWorkerTestEnv::setup(&docker)
        .await
        .expect("Failed to setup test environment");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Create request WITHOUT OIDC token
    let request = env.create_execute_request(execution_id, workflow_id, false, true);

    let response = env
        .app
        .clone()
        .oneshot_call(request)
        .await
        .expect("Failed to send request");

    // Should reject with 401 Unauthorized
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Worker should reject request without OIDC token"
    );
}

#[tokio::test]
#[ignore] // Requires Docker for testcontainers
async fn test_secure_worker_rejects_missing_hmac_signature() {
    let docker = Cli::default();
    let env = SecureWorkerTestEnv::setup(&docker)
        .await
        .expect("Failed to setup test environment");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Create request WITHOUT HMAC signature
    let request = env.create_execute_request(execution_id, workflow_id, true, false);

    let response = env
        .app
        .clone()
        .oneshot_call(request)
        .await
        .expect("Failed to send request");

    // Should reject with 401 Unauthorized
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Worker should reject request without HMAC signature"
    );
}

#[tokio::test]
#[ignore] // Requires Docker for testcontainers
async fn test_secure_worker_rejects_invalid_hmac() {
    let docker = Cli::default();
    let env = SecureWorkerTestEnv::setup(&docker)
        .await
        .expect("Failed to setup test environment");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Create valid base payload
    let payload = json!({
        "execution_id": execution_id,
        "workflow_id": workflow_id,
        "tenant_id": env.tenant.to_string(),
        "idempotency_key": null,
        "execution_plan": []
    });

    let json_bytes = serde_json::to_vec(&payload).unwrap();
    let base64_payload = STANDARD.encode(&json_bytes);

    // Create request with valid OIDC but INVALID HMAC signature
    let token = create_test_token(&env.audience, "test-service@project.iam.gserviceaccount.com");
    let invalid_signature = "0000000000000000000000000000000000000000000000000000000000000000";

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .header("x-servo-signature", invalid_signature)
        .body(Body::from(base64_payload))
        .unwrap();

    let response = env
        .app
        .clone()
        .oneshot_call(request)
        .await
        .expect("Failed to send request");

    // Should reject with 401 Unauthorized
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Worker should reject request with invalid HMAC signature"
    );
}

#[tokio::test]
#[ignore] // Requires Docker for testcontainers
async fn test_secure_worker_defense_in_depth() {
    let docker = Cli::default();
    let env = SecureWorkerTestEnv::setup(&docker)
        .await
        .expect("Failed to setup test environment");

    let execution_id = Uuid::new_v4();
    let workflow_id = Uuid::new_v4();

    // Test 1: Neither OIDC nor HMAC - should fail at OIDC check
    let request_no_auth = env.create_execute_request(execution_id, workflow_id, false, false);
    let response = env.app.clone().oneshot_call(request_no_auth).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Test 2: Valid OIDC, no HMAC - should fail at HMAC check
    let request_oidc_only = env.create_execute_request(execution_id, workflow_id, true, false);
    let response = env.app.clone().oneshot_call(request_oidc_only).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Test 3: No OIDC, valid HMAC - should fail at OIDC check (OIDC comes first)
    let request_hmac_only = env.create_execute_request(execution_id, workflow_id, false, true);
    let response = env.app.clone().oneshot_call(request_hmac_only).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Test 4: Both valid - should succeed
    let request_both = env.create_execute_request(execution_id, workflow_id, true, true);
    let response = env.app.clone().oneshot_call(request_both).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
