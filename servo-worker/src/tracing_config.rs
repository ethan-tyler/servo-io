//! OpenTelemetry tracing configuration for Servo worker.
//!
//! This module provides configurable distributed tracing with:
//! - OpenTelemetry integration for Cloud Trace export
//! - W3C trace context propagation
//! - Environment-based sampling configuration
//! - PII/secret filtering
//!
//! # Configuration
//!
//! ```bash
//! # Enable distributed tracing
//! export SERVO_TRACE_ENABLED=true
//!
//! # Sampling rate (0.0 to 1.0)
//! export SERVO_TRACE_SAMPLE_RATE=0.01  # 1% in production
//!
//! # OTLP endpoint (auto-detected on Cloud Run)
//! export SERVO_OTLP_ENDPOINT=https://cloudtrace.googleapis.com
//!
//! # Environment (affects default sampling)
//! export SERVO_ENVIRONMENT=production  # production|staging|development
//! ```

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{Config, RandomIdGenerator, Sampler, TracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::resource::{SERVICE_NAME, SERVICE_VERSION};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::sensitive_filter::SensitiveDataFilteringExporter;

/// Configuration for distributed tracing.
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Whether tracing is enabled
    pub enabled: bool,
    /// Sampling rate (0.0 to 1.0)
    pub sample_rate: f64,
    /// Service name for spans
    pub service_name: String,
    /// Service version for spans
    pub service_version: String,
    /// OTLP endpoint for trace export
    pub otlp_endpoint: Option<String>,
    /// GCP project ID (for Cloud Trace)
    pub gcp_project_id: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl TracingConfig {
    /// Load configuration from environment variables.
    ///
    /// Environment variables:
    /// - `SERVO_TRACE_ENABLED`: Enable tracing (default: false)
    /// - `SERVO_TRACE_SAMPLE_RATE`: Sampling rate 0.0-1.0 (default: based on environment)
    /// - `SERVO_OTLP_ENDPOINT`: OTLP exporter endpoint
    /// - `SERVO_ENVIRONMENT`: Environment name (production/staging/development)
    /// - `GCP_PROJECT_ID`: GCP project for Cloud Trace
    pub fn from_environment() -> Self {
        let environment =
            std::env::var("SERVO_ENVIRONMENT").unwrap_or_else(|_| "development".to_string());

        // Default sampling rates based on environment
        let default_sample_rate = match environment.as_str() {
            "production" => 0.01, // 1% in production
            "staging" => 1.0,     // 100% in staging
            "development" => 1.0, // 100% in development
            _ => 0.01,            // Conservative default
        };

        // Default enabled state based on environment
        let default_enabled = environment != "development";

        // Parse and clamp sample rate to valid range [0.0, 1.0]
        let raw_sample_rate: f64 = std::env::var("SERVO_TRACE_SAMPLE_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_sample_rate);
        let sample_rate = raw_sample_rate.clamp(0.0, 1.0);
        if (raw_sample_rate - sample_rate).abs() > f64::EPSILON {
            eprintln!(
                "Warning: SERVO_TRACE_SAMPLE_RATE={} is out of range, clamped to {}",
                raw_sample_rate, sample_rate
            );
        }

        Self {
            enabled: std::env::var("SERVO_TRACE_ENABLED")
                .map(|v| v.to_lowercase() == "true" || v == "1")
                .unwrap_or(default_enabled),
            sample_rate,
            service_name: std::env::var("SERVO_SERVICE_NAME")
                .unwrap_or_else(|_| "servo-worker".to_string()),
            service_version: std::env::var("SERVO_SERVICE_VERSION")
                .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string()),
            otlp_endpoint: std::env::var("SERVO_OTLP_ENDPOINT").ok(),
            gcp_project_id: std::env::var("GCP_PROJECT_ID").ok(),
        }
    }
}

/// Initialize tracing with OpenTelemetry support.
///
/// When `config.enabled` is true, initializes:
/// - OpenTelemetry tracer provider with OTLP exporter
/// - tracing-opentelemetry bridge layer
/// - JSON formatting for Cloud Logging compatibility
///
/// When `config.enabled` is false, initializes standard tracing with JSON output.
///
/// # Errors
///
/// Returns an error if the OTLP exporter fails to initialize.
pub fn init_tracing(
    config: &TracingConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "servo_worker=info,tower_http=info".into());

    if config.enabled {
        tracing::info!(
            sample_rate = config.sample_rate,
            service_name = %config.service_name,
            "Initializing OpenTelemetry tracing"
        );

        // Build resource with service metadata
        let resource = Resource::new(vec![
            opentelemetry::KeyValue::new(SERVICE_NAME, config.service_name.clone()),
            opentelemetry::KeyValue::new(SERVICE_VERSION, config.service_version.clone()),
        ]);

        // Configure sampling strategy
        // ParentBased respects the sampling decision of the parent span
        let sampler =
            Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(config.sample_rate)));

        // Configure OTLP exporter
        let endpoint = config.otlp_endpoint.clone().unwrap_or_else(|| {
            // Default to Cloud Trace endpoint
            "https://cloudtrace.googleapis.com".to_string()
        });

        let base_exporter = opentelemetry_otlp::new_exporter()
            .tonic()
            .with_endpoint(&endpoint)
            .with_timeout(Duration::from_secs(10))
            .build_span_exporter()?;

        // Wrap exporter with sensitive data filter to redact PII/secrets
        let filtered_exporter = SensitiveDataFilteringExporter::new(base_exporter);

        // Build trace config with sampler and resource
        let trace_config = Config::default()
            .with_sampler(sampler)
            .with_id_generator(RandomIdGenerator::default())
            .with_resource(resource);

        // Build tracer provider with filtered exporter
        let provider = TracerProvider::builder()
            .with_batch_exporter(filtered_exporter, runtime::Tokio)
            .with_config(trace_config)
            .build();

        let tracer = provider.tracer("servo-worker");

        // Set global tracer provider
        opentelemetry::global::set_tracer_provider(provider);

        // Build OpenTelemetry layer for tracing
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

        // Initialize subscriber with all layers
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .with(otel_layer)
            .init();

        tracing::info!("OpenTelemetry tracing initialized successfully");
    } else {
        // Standard tracing without OpenTelemetry
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();

        tracing::info!("Tracing initialized (OpenTelemetry disabled)");
    }

    Ok(())
}

/// Shutdown the OpenTelemetry tracer provider.
///
/// This should be called during graceful shutdown to flush any pending spans.
pub fn shutdown_tracing() {
    tracing::info!("Shutting down OpenTelemetry tracer provider");
    opentelemetry::global::shutdown_tracer_provider();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to clean all tracing-related env vars
    fn clean_tracing_env() {
        std::env::remove_var("SERVO_TRACE_ENABLED");
        std::env::remove_var("SERVO_TRACE_SAMPLE_RATE");
        std::env::remove_var("SERVO_ENVIRONMENT");
        std::env::remove_var("SERVO_OTLP_ENDPOINT");
        std::env::remove_var("SERVO_SERVICE_NAME");
        std::env::remove_var("SERVO_SERVICE_VERSION");
        std::env::remove_var("GCP_PROJECT_ID");
    }

    #[test]
    fn test_default_config_development() {
        // Clean environment for isolated test
        clean_tracing_env();

        let config = TracingConfig::from_environment();

        // In development (default), tracing is disabled by default
        assert!(
            !config.enabled,
            "Tracing should be disabled by default in development"
        );
        assert_eq!(config.sample_rate, 1.0, "100% sampling in development");
        assert_eq!(config.service_name, "servo-worker");

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_config_production() {
        // Clean environment for isolated test
        clean_tracing_env();
        std::env::set_var("SERVO_ENVIRONMENT", "production");

        let config = TracingConfig::from_environment();

        assert!(config.enabled, "Tracing enabled by default in production");
        assert_eq!(config.sample_rate, 0.01, "1% sampling in production");

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_config_explicit_override() {
        // Clean environment for isolated test
        clean_tracing_env();

        std::env::set_var("SERVO_TRACE_ENABLED", "true");
        std::env::set_var("SERVO_TRACE_SAMPLE_RATE", "0.5");

        let config = TracingConfig::from_environment();

        assert!(
            config.enabled,
            "SERVO_TRACE_ENABLED=true should enable tracing"
        );
        assert_eq!(
            config.sample_rate, 0.5,
            "SERVO_TRACE_SAMPLE_RATE=0.5 should set rate"
        );

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_config_staging() {
        // Clean environment for isolated test
        clean_tracing_env();
        std::env::set_var("SERVO_ENVIRONMENT", "staging");

        let config = TracingConfig::from_environment();

        assert_eq!(config.sample_rate, 1.0, "100% sampling in staging");

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_sample_rate_clamping_high() {
        // Clean environment for isolated test
        clean_tracing_env();

        // Test value > 1.0 gets clamped to 1.0
        std::env::set_var("SERVO_TRACE_SAMPLE_RATE", "2.5");
        let config = TracingConfig::from_environment();
        assert_eq!(
            config.sample_rate, 1.0,
            "Sample rate > 1.0 should be clamped to 1.0"
        );

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_sample_rate_clamping_negative() {
        // Clean environment for isolated test
        clean_tracing_env();

        // Test negative value gets clamped to 0.0
        std::env::set_var("SERVO_TRACE_SAMPLE_RATE", "-0.5");
        let config = TracingConfig::from_environment();
        assert_eq!(
            config.sample_rate, 0.0,
            "Negative sample rate should be clamped to 0.0"
        );

        // Clean up
        clean_tracing_env();
    }

    #[test]
    fn test_sample_rate_valid_unchanged() {
        // Clean environment for isolated test
        clean_tracing_env();

        // Test valid value in range is unchanged
        std::env::set_var("SERVO_TRACE_SAMPLE_RATE", "0.75");
        let config = TracingConfig::from_environment();
        assert_eq!(
            config.sample_rate, 0.75,
            "Valid sample rate should be unchanged"
        );

        // Clean up
        clean_tracing_env();
    }
}
