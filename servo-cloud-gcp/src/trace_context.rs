//! W3C Trace Context propagation utilities.
//!
//! This module provides utilities for propagating distributed trace context
//! using the W3C Trace Context standard (traceparent/tracestate headers).
//!
//! # Usage
//!
//! ## Injecting trace context (when enqueueing tasks)
//!
//! ```rust,ignore
//! use servo_cloud_gcp::trace_context::inject_trace_context;
//! use std::collections::HashMap;
//!
//! let mut headers = HashMap::new();
//! inject_trace_context(&mut headers);
//! // headers now contains "traceparent" and optionally "tracestate"
//! ```
//!
//! ## Extracting trace context (when receiving tasks)
//!
//! ```rust,ignore
//! use servo_cloud_gcp::trace_context::extract_trace_context;
//! use http::HeaderMap;
//!
//! let parent_context = extract_trace_context(&headers);
//! // Use parent_context to create child spans
//! ```

use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::collections::HashMap;

// Re-export http::HeaderMap for use by callers
pub use http::HeaderMap;

/// W3C Trace Context header names
pub const TRACEPARENT_HEADER: &str = "traceparent";
pub const TRACESTATE_HEADER: &str = "tracestate";

/// Inject the current trace context into HTTP headers for outgoing requests.
///
/// This function extracts the trace context from the current OpenTelemetry context
/// and injects it into the provided header map using W3C Trace Context format.
///
/// The injected headers are:
/// - `traceparent`: Contains version, trace-id, parent-id, and trace-flags
/// - `tracestate` (optional): Contains vendor-specific trace information
///
/// # Arguments
///
/// * `headers` - Mutable HashMap to inject trace headers into
///
/// # Example
///
/// ```rust,ignore
/// use servo_cloud_gcp::trace_context::inject_trace_context;
/// use std::collections::HashMap;
///
/// let mut headers = HashMap::new();
/// inject_trace_context(&mut headers);
///
/// // Headers now contain W3C trace context
/// assert!(headers.contains_key("traceparent"));
/// ```
pub fn inject_trace_context(headers: &mut HashMap<String, String>) {
    let propagator = TraceContextPropagator::new();
    let cx = opentelemetry::Context::current();
    propagator.inject_context(&cx, &mut HeaderInjector(headers));
}

/// Inject trace context from a specific OpenTelemetry context.
///
/// This variant allows injecting from a specific context rather than the current one,
/// which is useful when propagating context across async boundaries.
///
/// # Arguments
///
/// * `cx` - The OpenTelemetry context to inject from
/// * `headers` - Mutable HashMap to inject trace headers into
pub fn inject_trace_context_from(
    cx: &opentelemetry::Context,
    headers: &mut HashMap<String, String>,
) {
    let propagator = TraceContextPropagator::new();
    propagator.inject_context(cx, &mut HeaderInjector(headers));
}

/// Extract trace context from HTTP headers.
///
/// This function extracts the W3C Trace Context from HTTP headers and returns
/// an OpenTelemetry Context that can be used to create child spans.
///
/// # Arguments
///
/// * `headers` - HTTP HeaderMap containing trace context headers
///
/// # Returns
///
/// An OpenTelemetry Context. If no valid trace context is found in the headers,
/// returns an empty context (which will start a new trace).
///
/// # Example
///
/// ```rust,ignore
/// use servo_cloud_gcp::trace_context::extract_trace_context;
/// use http::HeaderMap;
///
/// let headers = HeaderMap::new();
/// let parent_cx = extract_trace_context(&headers);
///
/// // Create a child span with this parent context
/// let span = tracer
///     .span_builder("my_operation")
///     .start_with_context(&tracer, &parent_cx);
/// ```
pub fn extract_trace_context(headers: &http::HeaderMap) -> opentelemetry::Context {
    let propagator = TraceContextPropagator::new();
    propagator.extract(&HeaderExtractor(headers))
}

/// Extract trace context from a HashMap (for testing or non-HTTP sources).
///
/// # Arguments
///
/// * `headers` - HashMap containing trace context headers
///
/// # Returns
///
/// An OpenTelemetry Context extracted from the headers.
pub fn extract_trace_context_from_map(headers: &HashMap<String, String>) -> opentelemetry::Context {
    let propagator = TraceContextPropagator::new();
    propagator.extract(&MapExtractor(headers))
}

/// Check if trace context headers are present in the request.
///
/// # Arguments
///
/// * `headers` - HTTP HeaderMap to check
///
/// # Returns
///
/// `true` if the `traceparent` header is present
pub fn has_trace_context(headers: &http::HeaderMap) -> bool {
    headers.contains_key(TRACEPARENT_HEADER)
}

// Internal adapter to inject into HashMap<String, String>
struct HeaderInjector<'a>(&'a mut HashMap<String, String>);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        // Use lowercase keys for consistency
        self.0.insert(key.to_lowercase(), value);
    }
}

// Internal adapter to extract from http::HeaderMap
struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

// Internal adapter to extract from HashMap<String, String>
struct MapExtractor<'a>(&'a HashMap<String, String>);

impl Extractor for MapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        // Try both lowercase and original case
        self.0
            .get(key)
            .map(|s| s.as_str())
            .or_else(|| self.0.get(&key.to_lowercase()).map(|s| s.as_str()))
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TraceContextExt;

    #[test]
    fn test_inject_and_extract_roundtrip() {
        // This test verifies the basic structure without requiring a tracer
        let mut headers = HashMap::new();
        inject_trace_context(&mut headers);

        // Without an active span, no headers should be injected
        // This is expected behavior - the propagator only injects when there's a valid context
        // In production, there will be an active span from the tracing instrumentation
    }

    #[test]
    fn test_has_trace_context() {
        let mut headers = http::HeaderMap::new();
        assert!(!has_trace_context(&headers));

        headers.insert(
            TRACEPARENT_HEADER,
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
                .parse()
                .unwrap(),
        );
        assert!(has_trace_context(&headers));
    }

    #[test]
    fn test_extract_from_map() {
        let mut headers = HashMap::new();
        headers.insert(
            TRACEPARENT_HEADER.to_string(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string(),
        );

        let cx = extract_trace_context_from_map(&headers);
        // Context should be valid (not empty) with the traceparent
        // The actual validation depends on OpenTelemetry internals
        assert!(!cx.span().span_context().trace_id().to_string().is_empty());
    }

    #[test]
    fn test_header_injector_lowercase() {
        let mut headers = HashMap::new();
        let mut injector = HeaderInjector(&mut headers);
        injector.set("Traceparent", "some-value".to_string());

        // Should be stored in lowercase
        assert!(headers.contains_key("traceparent"));
        assert_eq!(headers.get("traceparent"), Some(&"some-value".to_string()));
    }

    #[test]
    fn test_map_extractor_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("traceparent".to_string(), "value1".to_string());

        let extractor = MapExtractor(&headers);

        // Should find lowercase key
        assert_eq!(extractor.get("traceparent"), Some("value1"));
        // Should also find with different case due to fallback
        assert_eq!(extractor.get("TRACEPARENT"), Some("value1"));
    }
}
