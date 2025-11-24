//! Servo Cloud Run Worker
//!
//! HTTP server for executing Servo workflows on Google Cloud Run.
//!
//! # Architecture
//!
//! - POST /execute - Receives workflow execution tasks from Cloud Tasks
//! - GET /health - Health check endpoint for Cloud Run
//!
//! The worker implements the "immediate ACK" pattern:
//! 1. Verify HMAC signature
//! 2. Respond 200 OK immediately
//! 3. Execute workflow in background
//!
//! # Configuration
//!
//! Environment variables:
//! - DATABASE_URL - PostgreSQL connection string
//! - SERVO_HMAC_SECRET - HMAC secret for signature verification
//! - PORT - HTTP port (default: 8080)
//! - EXECUTION_TIMEOUT - Workflow execution timeout in seconds (default: 600)

// Import from the library instead of declaring as modules
use axum::{
    routing::{get, post},
    Router,
};
use servo_worker::{
    config,
    executor,
    handler::{execute_handler, health_handler, metrics_handler, ready_handler, AppState},
    oidc,
};
use servo_storage::PostgresStorage;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber with JSON formatting for Cloud Logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "servo_worker=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting Servo Cloud Run Worker");

    // Load configuration from environment
    let config = match load_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(error = %e, "Failed to load configuration");
            std::process::exit(1);
        }
    };

    info!(
        database_url = %mask_password(&config.database_url),
        port = config.port,
        timeout_seconds = config.execution_timeout.as_secs(),
        "Configuration loaded"
    );

    // Initialize storage
    let storage = match PostgresStorage::new(&config.database_url).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            error!(error = %e, "Failed to initialize storage");
            std::process::exit(1);
        }
    };

    info!("Storage initialized successfully");

    // Initialize OIDC validator
    let oidc_config = match config::OidcConfig::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!(error = %e, "Failed to load OIDC configuration");
            std::process::exit(1);
        }
    };

    let oidc_validator = match oidc::initialize_validator(oidc_config).await {
        Ok(v) => Arc::new(v),
        Err(e) => {
            error!(error = %e, "Failed to initialize OIDC validator");
            std::process::exit(1);
        }
    };

    // Create executor
    let executor = Arc::new(executor::WorkflowExecutor::new(
        storage,
        config.execution_timeout,
    ));

    // Initialize rate limiters
    let tenant_rate_limiter_config = servo_worker::rate_limiter::TenantRateLimiterConfig::from_env();
    let tenant_rate_limiter = Arc::new(servo_worker::rate_limiter::TenantRateLimiter::new(
        tenant_rate_limiter_config,
    ));

    let ip_rate_limiter_config = servo_worker::rate_limiter::IpRateLimiterConfig::from_env();
    let ip_rate_limiter = Arc::new(servo_worker::rate_limiter::IpRateLimiter::new(
        ip_rate_limiter_config,
    ));

    // Create application state
    let state = AppState {
        executor,
        hmac_secret: config.hmac_secret,
        oidc_validator,
        tenant_rate_limiter,
        ip_rate_limiter,
    };

    // Build router with security hardening
    let app = Router::new()
        .route("/execute", post(execute_handler))
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/metrics", get(metrics_handler))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(Duration::from_secs(610))) // Slightly longer than execution timeout
        .layer(RequestBodyLimitLayer::new(10 * 1024 * 1024)) // 10MB max request body
        .with_state(state)
        .into_make_service_with_connect_info::<std::net::SocketAddr>(); // Enable ConnectInfo for IP extraction

    // Start server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, address = %addr, "Failed to bind server");
            std::process::exit(1);
        }
    };

    info!(address = %addr, "Server listening");

    // Run server with graceful shutdown
    // Note: app is already a MakeService with ConnectInfo
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        error!(error = %e, "Server error");
        std::process::exit(1);
    }

    info!("Server shut down gracefully");
}

/// Configuration loaded from environment variables
struct Config {
    database_url: String,
    hmac_secret: String,
    port: u16,
    execution_timeout: Duration,
}

/// Load configuration from environment variables
fn load_config() -> Result<Config, String> {
    let database_url =
        std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL environment variable not set")?;

    let hmac_secret = std::env::var("SERVO_HMAC_SECRET")
        .map_err(|_| "SERVO_HMAC_SECRET environment variable not set")?;

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .map_err(|e| format!("Invalid PORT value: {}", e))?;

    let timeout_seconds = std::env::var("EXECUTION_TIMEOUT")
        .unwrap_or_else(|_| "600".to_string())
        .parse::<u64>()
        .map_err(|e| format!("Invalid EXECUTION_TIMEOUT value: {}", e))?;

    Ok(Config {
        database_url,
        hmac_secret,
        port,
        execution_timeout: Duration::from_secs(timeout_seconds),
    })
}

/// Graceful shutdown signal handler
///
/// Waits for SIGTERM (Cloud Run shutdown signal) or Ctrl-C
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!("Received Ctrl-C, shutting down");
        }
        _ = terminate => {
            info!("Received SIGTERM, shutting down");
        }
    }
}

/// Mask password in database URL for logging
fn mask_password(url: &str) -> String {
    if let Some(at_pos) = url.find('@') {
        if let Some(colon_pos) = url[..at_pos].rfind(':') {
            let mut masked = url.to_string();
            masked.replace_range(colon_pos + 1..at_pos, "****");
            return masked;
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_password() {
        let url = "postgresql://user:password@localhost:5432/db";
        let masked = mask_password(url);
        assert!(masked.contains("****"));
        assert!(!masked.contains("password"));

        let url_no_password = "postgresql://localhost:5432/db";
        let masked = mask_password(url_no_password);
        assert_eq!(masked, url_no_password);
    }

    #[test]
    fn test_config_defaults() {
        // Test that PORT defaults to 8080
        std::env::remove_var("PORT");
        // Can't test full load_config without DATABASE_URL and SERVO_HMAC_SECRET
    }
}
