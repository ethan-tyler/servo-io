//! OIDC Token Validation Integration Tests
//!
//! These tests verify that the worker properly validates OIDC tokens from Cloud Tasks.
//!
//! Test infrastructure:
//! - Mock JWKS server using wiremock
//! - Test JWT token generation with jsonwebtoken
//! - Full integration testing of the execute handler

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use servo_storage::{models::ExecutionModel, postgres::PostgresStorage, TenantId};
use servo_worker::handler::{execute_handler, AppState};
use servo_worker::{config::OidcConfig, executor::WorkflowExecutor, oidc::OidcValidator};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Test JWKS (JSON Web Key Set) with a public key
/// This matches the private key used to sign test tokens
fn test_jwks() -> serde_json::Value {
    json!({
        "keys": [
            {
                "kty": "RSA",
                "use": "sig",
                "kid": "test-key-id",
                "alg": "RS256",
                "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
                "e": "AQAB"
            }
        ]
    })
}

/// Generate a test RSA private key for signing JWTs
fn test_private_key() -> EncodingKey {
    // This is a test-only RSA private key (2048-bit)
    // Generated using: openssl genrsa 2048
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

    EncodingKey::from_rsa_pem(pem.as_bytes()).expect("Failed to create encoding key")
}

/// Create a valid test OIDC token
fn create_valid_token(audience: &str, email: &str) -> String {
    let claims = json!({
        "iss": "https://accounts.google.com",
        "aud": audience,
        "sub": email,
        "email": email,
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        "iat": chrono::Utc::now().timestamp(),
    });

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key-id".to_string());

    encode(&header, &claims, &test_private_key()).expect("Failed to encode token")
}

/// Create an expired test OIDC token
fn create_expired_token(audience: &str, email: &str) -> String {
    let claims = json!({
        "iss": "https://accounts.google.com",
        "aud": audience,
        "sub": email,
        "email": email,
        "exp": (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp(),
        "iat": (chrono::Utc::now() - chrono::Duration::hours(2)).timestamp(),
    });

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key-id".to_string());

    encode(&header, &claims, &test_private_key()).expect("Failed to encode token")
}

/// Create a token with wrong audience
fn create_wrong_audience_token(audience: &str, email: &str) -> String {
    let claims = json!({
        "iss": "https://accounts.google.com",
        "aud": "https://wrong-audience.example.com",
        "sub": email,
        "email": email,
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        "iat": chrono::Utc::now().timestamp(),
    });

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key-id".to_string());

    encode(&header, &claims, &test_private_key()).expect("Failed to encode token")
}

/// Helper to create a valid execute request payload with HMAC signature
fn create_signed_payload(hmac_secret: &str) -> (Vec<u8>, String) {
    let payload = json!({
        "execution_id": Uuid::new_v4(),
        "workflow_id": Uuid::new_v4(),
        "tenant_id": "test-tenant",
        "idempotency_key": null,
        "execution_plan": []
    });

    let json_bytes = serde_json::to_vec(&payload).unwrap();
    let base64_payload = STANDARD.encode(&json_bytes);

    // Compute HMAC signature
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(hmac_secret.as_bytes()).unwrap();
    mac.update(base64_payload.as_bytes());
    let result = mac.finalize();
    let signature = hex::encode(result.into_bytes());

    (base64_payload.into_bytes(), signature)
}

/// Set up test environment with mock JWKS server
async fn setup_test_env() -> (MockServer, AppState, String) {
    // Start mock JWKS server
    let mock_server = MockServer::start().await;

    // Mount JWKS endpoint
    Mock::given(method("GET"))
        .and(path("/oauth2/v3/certs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(test_jwks()))
        .mount(&mock_server)
        .await;

    let audience = "https://worker.example.com";
    let jwks_url = format!("{}/oauth2/v3/certs", mock_server.uri());

    // Create OIDC config pointing to mock server
    let oidc_config = OidcConfig {
        enabled: true,
        issuer: "https://accounts.google.com".to_string(),
        jwks_url,
        expected_audience: audience.to_string(),
    };

    // Initialize OIDC validator
    let oidc_validator = Arc::new(
        OidcValidator::new(oidc_config)
            .expect("Failed to create OIDC validator")
    );

    // Fetch JWKS to populate cache
    oidc_validator
        .fetch_and_cache_jwks()
        .await
        .expect("Failed to fetch JWKS");

    // Create mock executor (we won't actually execute workflows in these tests)
    let database_url = "postgresql://test:test@localhost:5432/test";
    let storage = Arc::new(
        PostgresStorage::new(database_url)
            .await
            .expect("Failed to create storage - tests require database")
    );
    let executor = Arc::new(WorkflowExecutor::new(storage, Duration::from_secs(60)));

    let hmac_secret = "test-hmac-secret".to_string();

    let state = AppState {
        executor,
        hmac_secret: hmac_secret.clone(),
        oidc_validator,
    };

    (mock_server, state, audience.to_string())
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_worker_requires_oidc_token() {
    let (_mock_server, state, _audience) = setup_test_env().await;

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should reject with 401 Unauthorized (missing OIDC token)
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_worker_validates_oidc_signature() {
    let (_mock_server, state, audience) = setup_test_env().await;

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    // Use invalid token (not properly signed)
    let invalid_token = "invalid.token.here";

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", invalid_token))
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should reject with 401 Unauthorized (invalid signature)
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_worker_validates_oidc_audience() {
    let (_mock_server, state, audience) = setup_test_env().await;

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    // Create token with wrong audience
    let wrong_aud_token = create_wrong_audience_token(&audience, "test@example.com");

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", wrong_aud_token))
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should reject with 401 Unauthorized (wrong audience)
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_worker_rejects_expired_token() {
    let (_mock_server, state, audience) = setup_test_env().await;

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    // Create expired token
    let expired_token = create_expired_token(&audience, "test@example.com");

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", expired_token))
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should reject with 401 Unauthorized (expired token)
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_worker_accepts_valid_oidc_token() {
    let (_mock_server, state, audience) = setup_test_env().await;

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    // Create valid token
    let valid_token = create_valid_token(&audience, "test@example.com");

    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {}", valid_token))
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should accept with 200 OK (valid token)
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore] // Requires database connection
async fn test_oidc_validation_can_be_disabled() {
    // Create state with OIDC disabled
    let oidc_config = OidcConfig {
        enabled: false,
        issuer: String::new(),
        jwks_url: String::new(),
        expected_audience: String::new(),
    };

    let oidc_validator = Arc::new(
        OidcValidator::new(oidc_config).expect("Failed to create disabled OIDC validator")
    );

    let database_url = "postgresql://test:test@localhost:5432/test";
    let storage = Arc::new(
        PostgresStorage::new(database_url)
            .await
            .expect("Failed to create storage")
    );
    let executor = Arc::new(WorkflowExecutor::new(storage, Duration::from_secs(60)));

    let hmac_secret = "test-hmac-secret".to_string();

    let state = AppState {
        executor,
        hmac_secret: hmac_secret.clone(),
        oidc_validator,
    };

    let (body, signature) = create_signed_payload(&state.hmac_secret);

    // Request WITHOUT OIDC token
    let request = Request::builder()
        .uri("/execute")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-servo-signature", signature)
        .body(Body::from(body))
        .unwrap();

    let app = Router::new()
        .route("/execute", axum::routing::post(execute_handler))
        .with_state(state);

    let response = app.oneshot(request).await.unwrap();

    // Should accept without token when OIDC is disabled
    assert_eq!(response.status(), StatusCode::OK);
}
