//! Custom assertions for common verification patterns
//!
//! Provides assertion helpers that give better error messages
//! and handle async contexts.

use axum::body::Body;
use http::Response;
use http_body_util::BodyExt;
use serde_json::Value;

/// Response assertion helpers
pub struct ResponseAssertions {
    response: Response<Body>,
    body_bytes: Option<Vec<u8>>,
}

impl ResponseAssertions {
    /// Create assertions for a response
    pub fn new(response: Response<Body>) -> Self {
        Self {
            response,
            body_bytes: None,
        }
    }

    /// Get the status code
    pub fn status(&self) -> http::StatusCode {
        self.response.status()
    }

    /// Assert the status code equals expected
    pub fn assert_status(self, expected: http::StatusCode) -> Self {
        assert_eq!(
            self.response.status(),
            expected,
            "Expected status {} but got {}",
            expected,
            self.response.status()
        );
        self
    }

    /// Assert status is 200 OK
    pub fn assert_ok(self) -> Self {
        self.assert_status(http::StatusCode::OK)
    }

    /// Assert status is 400 Bad Request
    pub fn assert_bad_request(self) -> Self {
        self.assert_status(http::StatusCode::BAD_REQUEST)
    }

    /// Assert status is 401 Unauthorized
    pub fn assert_unauthorized(self) -> Self {
        self.assert_status(http::StatusCode::UNAUTHORIZED)
    }

    /// Assert status is 404 Not Found
    pub fn assert_not_found(self) -> Self {
        self.assert_status(http::StatusCode::NOT_FOUND)
    }

    /// Assert status is 429 Too Many Requests
    pub fn assert_rate_limited(self) -> Self {
        self.assert_status(http::StatusCode::TOO_MANY_REQUESTS)
    }

    /// Assert status is 503 Service Unavailable
    pub fn assert_service_unavailable(self) -> Self {
        self.assert_status(http::StatusCode::SERVICE_UNAVAILABLE)
    }

    /// Assert a header exists
    pub fn assert_header_exists(self, name: &str) -> Self {
        assert!(
            self.response.headers().contains_key(name),
            "Expected header '{}' to exist",
            name
        );
        self
    }

    /// Assert a header has a specific value
    pub fn assert_header(self, name: &str, expected: &str) -> Self {
        let value = self
            .response
            .headers()
            .get(name)
            .unwrap_or_else(|| panic!("Header '{}' not found", name))
            .to_str()
            .unwrap_or_else(|_| panic!("Header '{}' is not valid UTF-8", name));

        assert_eq!(
            value, expected,
            "Expected header '{}' to be '{}' but got '{}'",
            name, expected, value
        );
        self
    }

    /// Assert content-type header
    pub fn assert_content_type(self, expected: &str) -> Self {
        self.assert_header("content-type", expected)
    }

    /// Assert JSON content type
    pub fn assert_json_content_type(self) -> Self {
        let ct = self
            .response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or(""))
            .unwrap_or("");

        assert!(
            ct.starts_with("application/json"),
            "Expected JSON content type, got '{}'",
            ct
        );
        self
    }

    /// Consume response and get body bytes
    pub async fn into_body_bytes(mut self) -> (Self, Vec<u8>) {
        if self.body_bytes.is_none() {
            let body = std::mem::replace(self.response.body_mut(), Body::empty());
            let bytes = body
                .collect()
                .await
                .expect("Failed to read response body")
                .to_bytes()
                .to_vec();
            self.body_bytes = Some(bytes.clone());
            (self, bytes)
        } else {
            let bytes = self.body_bytes.clone().unwrap();
            (self, bytes)
        }
    }

    /// Get body as JSON value
    pub async fn into_json(self) -> (Self, Value) {
        let (assertions, bytes) = self.into_body_bytes().await;
        let json: Value = serde_json::from_slice(&bytes).expect("Response body is not valid JSON");
        (assertions, json)
    }
}

/// Assert JSON response matches expected schema
pub fn assert_json_schema(json: &Value, schema: &[(&str, JsonFieldType)]) {
    for (field, expected_type) in schema {
        let value = json.get(*field);
        match expected_type {
            JsonFieldType::String => {
                assert!(
                    value.map(|v| v.is_string()).unwrap_or(false),
                    "Expected field '{}' to be a string",
                    field
                );
            }
            JsonFieldType::Number => {
                assert!(
                    value.map(|v| v.is_number()).unwrap_or(false),
                    "Expected field '{}' to be a number",
                    field
                );
            }
            JsonFieldType::Boolean => {
                assert!(
                    value.map(|v| v.is_boolean()).unwrap_or(false),
                    "Expected field '{}' to be a boolean",
                    field
                );
            }
            JsonFieldType::Array => {
                assert!(
                    value.map(|v| v.is_array()).unwrap_or(false),
                    "Expected field '{}' to be an array",
                    field
                );
            }
            JsonFieldType::Object => {
                assert!(
                    value.map(|v| v.is_object()).unwrap_or(false),
                    "Expected field '{}' to be an object",
                    field
                );
            }
            JsonFieldType::Null => {
                assert!(
                    value.map(|v| v.is_null()).unwrap_or(true),
                    "Expected field '{}' to be null",
                    field
                );
            }
            JsonFieldType::Optional(inner) => {
                if let Some(v) = value {
                    if !v.is_null() {
                        assert_json_schema(
                            &serde_json::json!({ (*field): v.clone() }),
                            &[(*field, *inner.clone())],
                        );
                    }
                }
            }
        }
    }
}

/// JSON field type for schema validation
#[derive(Clone)]
pub enum JsonFieldType {
    String,
    Number,
    Boolean,
    Array,
    Object,
    Null,
    Optional(Box<JsonFieldType>),
}

/// Assert that a UUID string is valid
pub fn assert_valid_uuid(s: &str) {
    uuid::Uuid::parse_str(s).unwrap_or_else(|_| panic!("'{}' is not a valid UUID", s));
}

/// Assert that a string matches a pattern
pub fn assert_matches_pattern(s: &str, pattern: &str) {
    let re =
        regex::Regex::new(pattern).unwrap_or_else(|_| panic!("Invalid regex pattern: {}", pattern));
    assert!(
        re.is_match(s),
        "String '{}' does not match pattern '{}'",
        s,
        pattern
    );
}

/// Assert that execution states are valid
pub fn assert_valid_execution_state(state: &str) {
    const VALID_STATES: &[&str] = &[
        "pending",
        "running",
        "succeeded",
        "failed",
        "cancelled",
        "timeout",
    ];

    assert!(
        VALID_STATES.contains(&state),
        "Invalid execution state '{}'. Valid states are: {:?}",
        state,
        VALID_STATES
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assert_valid_uuid() {
        assert_valid_uuid("550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    #[should_panic(expected = "not a valid UUID")]
    fn test_assert_invalid_uuid() {
        assert_valid_uuid("not-a-uuid");
    }

    #[test]
    fn test_assert_valid_execution_state() {
        assert_valid_execution_state("pending");
        assert_valid_execution_state("running");
        assert_valid_execution_state("succeeded");
        assert_valid_execution_state("failed");
    }

    #[test]
    #[should_panic(expected = "Invalid execution state")]
    fn test_assert_invalid_execution_state() {
        assert_valid_execution_state("invalid");
    }

    #[test]
    fn test_json_schema_validation() {
        let json = serde_json::json!({
            "id": "123",
            "count": 42,
            "active": true,
            "items": [],
            "meta": {}
        });

        assert_json_schema(
            &json,
            &[
                ("id", JsonFieldType::String),
                ("count", JsonFieldType::Number),
                ("active", JsonFieldType::Boolean),
                ("items", JsonFieldType::Array),
                ("meta", JsonFieldType::Object),
            ],
        );
    }
}
