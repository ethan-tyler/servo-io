//! Mock implementations for external services
//!
//! Provides mock servers and implementations for testing without real dependencies.

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Mock JWKS server for OIDC token validation testing
///
/// This mock server provides a JWKS endpoint with a test RSA public key,
/// and utilities for generating signed JWT tokens.
///
/// # Example
///
/// ```ignore
/// use servo_tests::mocks::MockJwksServer;
///
/// #[tokio::test]
/// async fn test_oidc_validation() {
///     let jwks = MockJwksServer::start().await;
///     let token = jwks.valid_token("https://my-service.com", "user@example.com");
///
///     // Use token in your test...
/// }
/// ```
pub struct MockJwksServer {
    server: MockServer,
    encoding_key: EncodingKey,
}

impl MockJwksServer {
    /// Start a new mock JWKS server
    pub async fn start() -> Self {
        let server = MockServer::start().await;

        // Mount JWKS endpoint
        Mock::given(method("GET"))
            .and(path("/oauth2/v3/certs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Self::test_jwks()))
            .mount(&server)
            .await;

        Self {
            server,
            encoding_key: Self::test_encoding_key(),
        }
    }

    /// Get the base URL of the mock server
    pub fn uri(&self) -> String {
        self.server.uri()
    }

    /// Get the JWKS URL
    pub fn jwks_url(&self) -> String {
        format!("{}/oauth2/v3/certs", self.server.uri())
    }

    /// Generate a valid OIDC token
    pub fn valid_token(&self, audience: &str, email: &str) -> String {
        let claims = json!({
            "iss": "https://accounts.google.com",
            "aud": audience,
            "sub": email,
            "email": email,
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            "iat": chrono::Utc::now().timestamp(),
        });

        let mut header = Header::new(Self::algorithm());
        header.kid = Some("test-key-id".to_string());

        encode(&header, &claims, &self.encoding_key).expect("Failed to encode token")
    }

    /// Generate an expired OIDC token
    pub fn expired_token(&self, audience: &str, email: &str) -> String {
        let claims = json!({
            "iss": "https://accounts.google.com",
            "aud": audience,
            "sub": email,
            "email": email,
            "exp": (chrono::Utc::now() - chrono::Duration::hours(1)).timestamp(),
            "iat": (chrono::Utc::now() - chrono::Duration::hours(2)).timestamp(),
        });

        let mut header = Header::new(Self::algorithm());
        header.kid = Some("test-key-id".to_string());

        encode(&header, &claims, &self.encoding_key).expect("Failed to encode token")
    }

    /// Generate a token with wrong audience
    pub fn wrong_audience_token(&self, email: &str) -> String {
        let claims = json!({
            "iss": "https://accounts.google.com",
            "aud": "https://wrong-audience.example.com",
            "sub": email,
            "email": email,
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            "iat": chrono::Utc::now().timestamp(),
        });

        let mut header = Header::new(Self::algorithm());
        header.kid = Some("test-key-id".to_string());

        encode(&header, &claims, &self.encoding_key).expect("Failed to encode token")
    }

    /// Generate a token with wrong issuer
    pub fn wrong_issuer_token(&self, audience: &str, email: &str) -> String {
        let claims = json!({
            "iss": "https://wrong-issuer.example.com",
            "aud": audience,
            "sub": email,
            "email": email,
            "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
            "iat": chrono::Utc::now().timestamp(),
        });

        let mut header = Header::new(Self::algorithm());
        header.kid = Some("test-key-id".to_string());

        encode(&header, &claims, &self.encoding_key).expect("Failed to encode token")
    }

    /// Generate a token with custom claims
    pub fn token_with_claims(&self, claims: serde_json::Value) -> String {
        let mut header = Header::new(Self::algorithm());
        header.kid = Some("test-key-id".to_string());

        encode(&header, &claims, &self.encoding_key).expect("Failed to encode token")
    }

    /// Test JWKS (JSON Web Key Set) with a public key
    ///
    /// Note: This uses HS256 for simplicity in tests. In production,
    /// OIDC tokens use RS256 with Google's actual JWKS.
    fn test_jwks() -> serde_json::Value {
        // For test purposes, we return a placeholder JWKS
        // The actual token validation in tests uses the test secret
        json!({
            "keys": [
                {
                    "kty": "oct",
                    "use": "sig",
                    "kid": "test-key-id",
                    "alg": "HS256",
                    "k": "dGVzdC1zZWNyZXQta2V5LWZvci1qd3QtdG9rZW5z" // base64url("test-secret-key-for-jwt-tokens")
                }
            ]
        })
    }

    /// Test encoding key for signing JWTs (uses HS256 for simplicity)
    fn test_encoding_key() -> EncodingKey {
        EncodingKey::from_secret(b"test-secret-key-for-jwt-tokens")
    }

    /// Get the algorithm used by this mock server
    fn algorithm() -> Algorithm {
        Algorithm::HS256
    }
}

/// Helper to compute HMAC-SHA256 signature for test payloads
pub fn compute_hmac_signature(payload: &[u8], secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload);
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

/// Helper to create a signed base64-encoded payload
pub fn create_signed_payload(payload: &serde_json::Value, secret: &str) -> (Vec<u8>, String) {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let json_bytes = serde_json::to_vec(payload).expect("Failed to serialize payload");
    let base64_payload = STANDARD.encode(&json_bytes);
    let body_bytes = base64_payload.as_bytes().to_vec();
    let signature = compute_hmac_signature(&body_bytes, secret);

    (body_bytes, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_jwks_server_starts() {
        let server = MockJwksServer::start().await;
        assert!(!server.uri().is_empty());
        assert!(server.jwks_url().contains("/oauth2/v3/certs"));
    }

    #[tokio::test]
    async fn test_mock_jwks_server_generates_tokens() {
        let server = MockJwksServer::start().await;

        let valid = server.valid_token("https://test.com", "user@test.com");
        assert!(!valid.is_empty());
        assert!(valid.contains('.'));

        let expired = server.expired_token("https://test.com", "user@test.com");
        assert!(!expired.is_empty());

        let wrong_aud = server.wrong_audience_token("user@test.com");
        assert!(!wrong_aud.is_empty());
    }

    #[test]
    fn test_hmac_signature_computation() {
        let payload = b"test payload";
        let secret = "test-secret";

        let sig1 = compute_hmac_signature(payload, secret);
        let sig2 = compute_hmac_signature(payload, secret);

        // Same input should produce same signature
        assert_eq!(sig1, sig2);

        // Different input should produce different signature
        let sig3 = compute_hmac_signature(b"different", secret);
        assert_ne!(sig1, sig3);
    }

    #[test]
    fn test_create_signed_payload() {
        let payload = serde_json::json!({
            "execution_id": "test-id",
            "workflow_id": "wf-id"
        });

        let (body, signature) = create_signed_payload(&payload, "test-secret-32-bytes-long-min");

        assert!(!body.is_empty());
        assert!(!signature.is_empty());
        assert_eq!(signature.len(), 64); // SHA256 hex-encoded
    }
}
