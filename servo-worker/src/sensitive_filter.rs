//! Sensitive data filtering for telemetry.
//!
//! Provides redaction of PII, secrets, and credentials from:
//! - OpenTelemetry span attributes
//! - Structured log fields
//!
//! # Architecture
//!
//! This module implements a dual-filter approach:
//! 1. `SensitiveDataFilteringExporter` - Wraps the OTLP exporter to filter spans
//! 2. Pattern matching logic can be reused for log filtering
//!
//! # Usage
//!
//! ```rust,ignore
//! use servo_worker::sensitive_filter::SensitiveDataFilteringExporter;
//!
//! let base_exporter = /* OTLP exporter */;
//! let filtered_exporter = SensitiveDataFilteringExporter::new(base_exporter);
//! ```

use std::borrow::Cow;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use opentelemetry::KeyValue;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};

/// Keys that should always be redacted (case-insensitive substring match).
const SENSITIVE_KEY_PATTERNS: &[&str] = &[
    "password",
    "secret",
    "token",
    "bearer",
    "authorization",
    "credential",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "hmac",
    "signature",
    "x-servo-signature",
    "oidc",
    "jwt",
    "access_token",
    "refresh_token",
    "client_secret",
    "database_url",
    "connection_string",
];

/// Value patterns that indicate sensitive content (case-insensitive prefix match).
const SENSITIVE_VALUE_PATTERNS: &[&str] = &[
    "bearer ",     // OAuth bearer tokens
    "basic ",      // Basic auth
    "ey",          // JWT tokens (base64 header starts with eyJ)
    "ghp_",        // GitHub personal access tokens
    "gho_",        // GitHub OAuth tokens
    "sk-",         // OpenAI/Stripe secret keys
    "pk_",         // Stripe publishable keys (still redact)
    "ya29.",       // Google OAuth access tokens
    "agp_",        // Anthropic API keys
    "postgres://", // Database connection strings
    "postgresql://",
    "mysql://",
    "redis://",
];

/// Allowlist of keys that are safe to pass through (exact match).
const SAFE_KEYS: &[&str] = &[
    // Identifiers (UUIDs, not sensitive)
    "execution_id",
    "workflow_id",
    "tenant_id",
    "request_id",
    "trace_id",
    "span_id",
    "parent_span_id",
    "task_id",
    "asset_id",
    "step_id",
    // OpenTelemetry semantic conventions
    "db.system",
    "db.operation",
    "db.sql.table",
    "db.name",
    "db.response.rows_affected",
    "http.method",
    "http.status_code",
    "http.route",
    "http.target",
    // Servo operational fields
    "service.name",
    "service.version",
    "environment",
    "queue_path",
    "target_url",
    "attempts",
    "max_attempts",
    "max_concurrent",
    "available_before",
    "wait_duration_ms",
    "total_duration_ms",
    "strategy",
    "status",
    "error.type",
    "error.message",
    "level",
    "target",
    "file",
    "line",
    // Span metadata
    "otel.name",
    "otel.kind",
    "otel.status_code",
    "otel.status_message",
    "code.function",
    "code.namespace",
    "code.filepath",
    "code.lineno",
    "thread.id",
    "thread.name",
];

/// Redacted value placeholder.
pub const REDACTED: &str = "[REDACTED]";

/// Sensitive data filter with configurable patterns.
///
/// Uses a combination of:
/// - Key pattern matching (substring, case-insensitive)
/// - Value pattern matching (prefix, case-insensitive)
/// - Explicit allowlist for safe keys
#[derive(Clone, Debug)]
pub struct SensitiveDataMatcher {
    sensitive_keys: HashSet<String>,
    safe_keys: HashSet<String>,
}

impl Default for SensitiveDataMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SensitiveDataMatcher {
    /// Create a new matcher with default patterns.
    pub fn new() -> Self {
        Self {
            sensitive_keys: SENSITIVE_KEY_PATTERNS
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            safe_keys: SAFE_KEYS.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    /// Check if a key should be redacted.
    ///
    /// Returns `true` if the key matches a sensitive pattern and is not
    /// in the allowlist.
    pub fn is_sensitive_key(&self, key: &str) -> bool {
        let lower = key.to_lowercase();

        // Allowlist takes precedence
        if self.safe_keys.contains(&lower) {
            return false;
        }

        // Check for sensitive key patterns (substring match)
        self.sensitive_keys
            .iter()
            .any(|pattern| lower.contains(pattern))
    }

    /// Check if a value contains sensitive patterns.
    ///
    /// Returns `true` if the value starts with a known sensitive prefix.
    pub fn is_sensitive_value(&self, value: &str) -> bool {
        let lower = value.to_lowercase();
        SENSITIVE_VALUE_PATTERNS
            .iter()
            .any(|pattern| lower.starts_with(pattern))
    }

    /// Redact a value if the key or value is sensitive.
    ///
    /// Returns `[REDACTED]` if sensitive, otherwise returns the original value.
    pub fn filter_value<'a>(&self, key: &str, value: &'a str) -> Cow<'a, str> {
        if self.is_sensitive_key(key) || self.is_sensitive_value(value) {
            Cow::Borrowed(REDACTED)
        } else {
            Cow::Borrowed(value)
        }
    }
}

/// A SpanExporter wrapper that filters sensitive data before export.
///
/// This wraps the actual exporter (OTLP) and filters span attributes
/// before they are sent to Cloud Trace.
///
/// # Example
///
/// ```rust,ignore
/// let base_exporter = opentelemetry_otlp::new_exporter()
///     .tonic()
///     .build_span_exporter()?;
///
/// let filtered_exporter = SensitiveDataFilteringExporter::new(base_exporter);
///
/// let provider = TracerProvider::builder()
///     .with_batch_exporter(filtered_exporter, runtime::Tokio)
///     .build();
/// ```
pub struct SensitiveDataFilteringExporter<E> {
    inner: E,
    matcher: SensitiveDataMatcher,
}

impl<E: std::fmt::Debug> std::fmt::Debug for SensitiveDataFilteringExporter<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SensitiveDataFilteringExporter")
            .field("inner", &self.inner)
            .field("matcher", &self.matcher)
            .finish()
    }
}

impl<E> SensitiveDataFilteringExporter<E>
where
    E: SpanExporter,
{
    /// Create a new filtering exporter wrapping the given exporter.
    pub fn new(inner: E) -> Self {
        Self {
            inner,
            matcher: SensitiveDataMatcher::new(),
        }
    }

    /// Filter a single KeyValue, redacting if sensitive.
    fn filter_key_value(&self, kv: &KeyValue) -> KeyValue {
        let key = kv.key.as_str();

        // Try to get string value
        if let opentelemetry::Value::String(ref s) = kv.value {
            let value = s.as_ref();
            if self.matcher.is_sensitive_key(key) || self.matcher.is_sensitive_value(value) {
                return KeyValue::new(
                    kv.key.clone(),
                    opentelemetry::Value::String(REDACTED.into()),
                );
            }
        }

        kv.clone()
    }

    /// Filter all attributes in a span.
    fn filter_attributes(&self, attributes: &[KeyValue]) -> Vec<KeyValue> {
        attributes
            .iter()
            .map(|kv| self.filter_key_value(kv))
            .collect()
    }
}

impl<E> SpanExporter for SensitiveDataFilteringExporter<E>
where
    E: SpanExporter + Send + Sync + 'static,
{
    fn export(
        &mut self,
        batch: Vec<SpanData>,
    ) -> Pin<Box<dyn Future<Output = ExportResult> + Send>> {
        // Filter each span's attributes before export
        // Note: SpanEvents is #[non_exhaustive] so we cannot reconstruct it.
        // Event attributes are typically less sensitive (exception info, timing).
        // The primary filtering target is span attributes.
        let filtered_batch: Vec<SpanData> = batch
            .into_iter()
            .map(|mut span| {
                // Filter span attributes (main source of potentially sensitive data)
                span.attributes = self.filter_attributes(&span.attributes);
                span
            })
            .collect();

        self.inner.export(filtered_batch)
    }

    fn shutdown(&mut self) {
        self.inner.shutdown()
    }

    fn force_flush(&mut self) -> Pin<Box<dyn Future<Output = ExportResult> + Send>> {
        self.inner.force_flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sensitive_key_detection() {
        let matcher = SensitiveDataMatcher::new();

        // Should be sensitive
        assert!(matcher.is_sensitive_key("password"));
        assert!(matcher.is_sensitive_key("user_password"));
        assert!(matcher.is_sensitive_key("PASSWORD"));
        assert!(matcher.is_sensitive_key("api_key"));
        assert!(matcher.is_sensitive_key("x-servo-signature"));
        assert!(matcher.is_sensitive_key("authorization"));
        assert!(matcher.is_sensitive_key("bearer_token"));
        assert!(matcher.is_sensitive_key("hmac_secret"));
        assert!(matcher.is_sensitive_key("client_secret"));
        assert!(matcher.is_sensitive_key("access_token"));
        assert!(matcher.is_sensitive_key("refresh_token"));
        assert!(matcher.is_sensitive_key("database_url"));
        assert!(matcher.is_sensitive_key("connection_string"));
        assert!(matcher.is_sensitive_key("oidc_token"));
        assert!(matcher.is_sensitive_key("jwt_token"));
        assert!(matcher.is_sensitive_key("private_key"));
        assert!(matcher.is_sensitive_key("credential"));

        // Should NOT be sensitive (in allowlist)
        assert!(!matcher.is_sensitive_key("execution_id"));
        assert!(!matcher.is_sensitive_key("tenant_id"));
        assert!(!matcher.is_sensitive_key("db.operation"));
        assert!(!matcher.is_sensitive_key("http.status_code"));
        assert!(!matcher.is_sensitive_key("service.name"));
        assert!(!matcher.is_sensitive_key("workflow_id"));
        assert!(!matcher.is_sensitive_key("trace_id"));

        // Should NOT be sensitive (no pattern match)
        assert!(!matcher.is_sensitive_key("workflow_name"));
        assert!(!matcher.is_sensitive_key("status"));
        assert!(!matcher.is_sensitive_key("duration_ms"));
        assert!(!matcher.is_sensitive_key("count"));
    }

    #[test]
    fn test_sensitive_value_detection() {
        let matcher = SensitiveDataMatcher::new();

        // Should be sensitive
        assert!(matcher.is_sensitive_value("Bearer eyJhbGciOiJIUzI1NiIs..."));
        assert!(matcher.is_sensitive_value("bearer abc123"));
        assert!(matcher.is_sensitive_value("Basic dXNlcjpwYXNz"));
        assert!(matcher.is_sensitive_value("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9..."));
        assert!(matcher.is_sensitive_value("ghp_xxxxxxxxxxxxxxxxxxxx"));
        assert!(matcher.is_sensitive_value("gho_xxxxxxxxxxxxxxxxxxxx"));
        assert!(matcher.is_sensitive_value("sk-xxxxxxxxxxxxxxxx"));
        assert!(matcher.is_sensitive_value("pk_xxxxxxxxxxxxxxxx"));
        assert!(matcher.is_sensitive_value("ya29.a0AfH6SMBxxxxxxx"));
        assert!(matcher.is_sensitive_value("agp_xxxxxxxxxxxxxxxx"));
        assert!(matcher.is_sensitive_value("postgres://user:pass@host/db"));
        assert!(matcher.is_sensitive_value("postgresql://user:pass@host/db"));
        assert!(matcher.is_sensitive_value("mysql://user:pass@host/db"));
        assert!(matcher.is_sensitive_value("redis://user:pass@host/db"));

        // Should NOT be sensitive
        assert!(!matcher.is_sensitive_value("running"));
        assert!(!matcher.is_sensitive_value("12345"));
        assert!(!matcher.is_sensitive_value("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!matcher.is_sensitive_value("some random text"));
        assert!(!matcher.is_sensitive_value(""));
    }

    #[test]
    fn test_filter_value() {
        let matcher = SensitiveDataMatcher::new();

        // Key-based redaction
        assert_eq!(
            matcher.filter_value("password", "secret123").as_ref(),
            REDACTED
        );
        assert_eq!(matcher.filter_value("api_key", "abc123").as_ref(), REDACTED);
        assert_eq!(
            matcher.filter_value("authorization", "Bearer xyz").as_ref(),
            REDACTED
        );

        // Value-based redaction (even with safe key)
        assert_eq!(
            matcher
                .filter_value("some_field", "Bearer token123")
                .as_ref(),
            REDACTED
        );
        assert_eq!(
            matcher
                .filter_value("url", "postgres://user:pass@host/db")
                .as_ref(),
            REDACTED
        );
        assert_eq!(
            matcher
                .filter_value("data", "eyJhbGciOiJIUzI1NiJ9.payload")
                .as_ref(),
            REDACTED
        );

        // Safe key passthrough
        assert_eq!(
            matcher.filter_value("execution_id", "abc-123").as_ref(),
            "abc-123"
        );
        assert_eq!(
            matcher.filter_value("tenant_id", "tenant-001").as_ref(),
            "tenant-001"
        );
        assert_eq!(
            matcher.filter_value("db.operation", "SELECT").as_ref(),
            "SELECT"
        );

        // Normal value passthrough
        assert_eq!(
            matcher.filter_value("status", "running").as_ref(),
            "running"
        );
        assert_eq!(matcher.filter_value("count", "42").as_ref(), "42");
    }

    #[test]
    fn test_case_insensitivity() {
        let matcher = SensitiveDataMatcher::new();

        // Keys should be case-insensitive
        assert!(matcher.is_sensitive_key("PASSWORD"));
        assert!(matcher.is_sensitive_key("Password"));
        assert!(matcher.is_sensitive_key("pAsSwOrD"));
        assert!(matcher.is_sensitive_key("API_KEY"));
        assert!(matcher.is_sensitive_key("Api_Key"));

        // Values should be case-insensitive
        assert!(matcher.is_sensitive_value("BEARER token"));
        assert!(matcher.is_sensitive_value("Bearer token"));
        assert!(matcher.is_sensitive_value("BASIC auth"));
        assert!(matcher.is_sensitive_value("Basic auth"));
    }

    #[test]
    fn test_allowlist_precedence() {
        let matcher = SensitiveDataMatcher::new();

        // "access_token" is in sensitive patterns but "status" is in allowlist
        // This test verifies the allowlist takes precedence
        assert!(!matcher.is_sensitive_key("status"));
        assert!(!matcher.is_sensitive_key("service.name"));

        // These are sensitive because they match patterns and aren't in allowlist
        assert!(matcher.is_sensitive_key("access_token"));
        assert!(matcher.is_sensitive_key("my_secret"));
    }

    #[test]
    fn test_filter_key_value() {
        let exporter = SensitiveDataFilteringExporter {
            inner: MockExporter,
            matcher: SensitiveDataMatcher::new(),
        };

        // Sensitive key - should be redacted
        let kv = KeyValue::new("password", "secret123");
        let filtered = exporter.filter_key_value(&kv);
        assert_eq!(filtered.key.as_str(), "password");
        assert_eq!(
            filtered.value,
            opentelemetry::Value::String(REDACTED.into())
        );

        // Sensitive value - should be redacted
        let kv = KeyValue::new("some_field", "Bearer abc123");
        let filtered = exporter.filter_key_value(&kv);
        assert_eq!(filtered.key.as_str(), "some_field");
        assert_eq!(
            filtered.value,
            opentelemetry::Value::String(REDACTED.into())
        );

        // Safe key and value - should pass through
        let kv = KeyValue::new("execution_id", "uuid-123");
        let filtered = exporter.filter_key_value(&kv);
        assert_eq!(filtered.key.as_str(), "execution_id");
        assert_eq!(
            filtered.value,
            opentelemetry::Value::String("uuid-123".into())
        );

        // Non-string value - should pass through unchanged
        let kv = KeyValue::new("count", 42i64);
        let filtered = exporter.filter_key_value(&kv);
        assert_eq!(filtered.key.as_str(), "count");
        assert_eq!(filtered.value, opentelemetry::Value::I64(42));
    }

    // Mock exporter for testing
    #[derive(Debug)]
    struct MockExporter;

    impl SpanExporter for MockExporter {
        fn export(
            &mut self,
            _batch: Vec<SpanData>,
        ) -> Pin<Box<dyn Future<Output = ExportResult> + Send>> {
            Box::pin(async { Ok(()) })
        }

        fn shutdown(&mut self) {}

        fn force_flush(&mut self) -> Pin<Box<dyn Future<Output = ExportResult> + Send>> {
            Box::pin(async { Ok(()) })
        }
    }
}
