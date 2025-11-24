# servo-cloud-gcp

Google Cloud Platform adapter for Servo, providing Cloud Tasks integration for workflow task scheduling.

## Overview

The `servo-cloud-gcp` crate enables Servo to schedule workflow executions using Google Cloud Tasks:

- **Cloud Tasks Queue**: Schedules workflow execution tasks with OIDC authentication
- **OAuth2 Authentication**: Service account authentication for Cloud Tasks API
- **HMAC Payload Signing**: Ensures task payload integrity
- **Retry Logic**: Exponential backoff with jitter for 5xx and 429 errors
- **Idempotency**: Prevents duplicate workflow executions via idempotency keys

## Usage

```rust
use servo_cloud_gcp::CloudTasksQueue;
use std::sync::Arc;

// Load service account credentials
let service_account_json = std::fs::read_to_string("key.json")?;
let hmac_secret = std::env::var("SERVO_HMAC_SECRET")?;

// Create queue
let queue = CloudTasksQueue::new(
    "my-project".to_string(),
    "us-central1".to_string(),
    "servo-tasks".to_string(),
    "https://my-worker.run.app".to_string(),
    &service_account_json,
    hmac_secret,
)?;

// Enqueue a workflow execution task
let task_name = queue
    .enqueue(
        execution_id,
        workflow_id,
        tenant_id,
        Some("idempotency-key-123".to_string()),
        Vec::new(), // Execution plan (currently empty)
    )
    .await?;
```

## Configuration

### Authentication

Set up GCP credentials:

```bash
# Production: Use Workload Identity (Cloud Run / GKE)
# No environment variables needed - automatic!

# Development: Use Application Default Credentials (ADC)
gcloud auth application-default login

# Legacy: Service account key (NOT recommended for production)
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/key.json
```

**⚠️ Recommendation**: Use [Workload Identity](../docs/WORKLOAD_IDENTITY_MIGRATION.md) in production
to eliminate JSON key files.

### Secret Manager Integration (Recommended)

For production deployments, use GCP Secret Manager to store HMAC secrets instead of environment variables:

```rust
use servo_cloud_gcp::{GcpAuth, secrets::SecretManager};
use std::sync::Arc;

// Initialize auth
let auth = Arc::new(GcpAuth::from_environment()?);

// Initialize Secret Manager
let secret_manager = SecretManager::new(
    "my-project".to_string(),
    "servo-hmac-secret".to_string(),  // Secret name in Secret Manager
    auth.clone(),
)?;

// Initialize secrets with startup validation
secret_manager.initialize().await?;

// Use in your application
let current_secret = secret_manager.get_current_secret().await?;
```

**Features**:

- ✅ Automatic secret rotation support (current + previous versions)
- ✅ 5-minute refresh interval (no restart needed)
- ✅ Startup validation (minimum 32-byte length, entropy checks)
- ✅ Fail-closed behavior on missing/invalid secrets
- ✅ Zero-downtime rotation window

**Setup**:

```bash
# Create secret in Secret Manager
echo -n "your-strong-random-secret-min-32-bytes" | \
  gcloud secrets create servo-hmac-secret \
    --data-file=- \
    --project=my-project

# Grant access to service account
gcloud secrets add-iam-policy-binding servo-hmac-secret \
    --member="serviceAccount:servo-worker@my-project.iam.gserviceaccount.com" \
    --role="roles/secretmanager.secretAccessor"
```

**Rotation Procedure**:

```bash
# 1. Add new secret version
echo -n "new-secret-value" | \
  gcloud secrets versions add servo-hmac-secret --data-file=-

# 2. Wait 5 minutes for all instances to refresh (automatic)

# 3. Destroy old version
gcloud secrets versions destroy 1 --secret=servo-hmac-secret
```

### Circuit Breaker Configuration

Circuit breakers protect against cascading failures by detecting consecutive errors and temporarily failing fast.
Each external dependency has its own circuit breaker with separate configuration.

**Protected Dependencies**:

- **Cloud Tasks API**: Task enqueueing operations
- **Secret Manager API**: Secret fetching operations
- **OAuth2 Token Endpoint**: Token acquisition for GCP API authentication

**Configuration (per dependency)**:

```bash
# Cloud Tasks circuit breaker
export SERVO_CB_CLOUD_TASKS_FAILURE_THRESHOLD=5        # Default: 5
export SERVO_CB_CLOUD_TASKS_HALF_OPEN_TIMEOUT_SECS=30  # Default: 30

# Secret Manager circuit breaker
export SERVO_CB_SECRET_MANAGER_FAILURE_THRESHOLD=5        # Default: 5
export SERVO_CB_SECRET_MANAGER_HALF_OPEN_TIMEOUT_SECS=30  # Default: 30

# OAuth2 token endpoint circuit breaker
export SERVO_CB_OAUTH2_FAILURE_THRESHOLD=5        # Default: 5
export SERVO_CB_OAUTH2_HALF_OPEN_TIMEOUT_SECS=30  # Default: 30
```

**Circuit Breaker States**:

- **Closed** (normal): All requests pass through
- **Open** (fail-fast): Rejects requests immediately after threshold consecutive failures
- **Half-Open** (recovery): After timeout, allows single probe request to test recovery

**Error Classification** (determines which errors trip the breaker):

- ✅ **Trip breaker**: Network errors (timeout, connection refused, DNS), 5xx server errors, 429 rate limits
- ❌ **Don't trip**: 4xx client errors (bad request, unauthorized, not found) - these indicate configuration issues

**Metrics**:

```promql
# Circuit breaker state (0=closed, 1=open)
servo_circuit_breaker_state{dependency="cloud_tasks|secret_manager|oauth2"}

# Total circuit opens
servo_circuit_breaker_opens_total{dependency="cloud_tasks|secret_manager|oauth2"}

# Half-open probe attempts
servo_circuit_breaker_half_open_attempts_total{dependency="cloud_tasks|secret_manager|oauth2",result="success|failure"}
```

**Recommendations**:

- Use **defaults** (5 failures, 30s timeout) for most production workloads
- **Increase threshold** (e.g., 10) for noisy dependencies with transient failures
- **Decrease timeout** (e.g., 10s) for aggressive fail-fast behavior
- Monitor `servo_circuit_breaker_opens_total` to detect dependency issues early

## Required GCP APIs

Enable these APIs in your project:

- Cloud Tasks API
- Secret Manager API (if using Secret Manager for HMAC secrets)

## Security Considerations

### Metrics Endpoint Protection

**CRITICAL**: The `/metrics` endpoint exposes operational metrics and should be protected in production:

#### Option 1: Bearer Token Authentication (Recommended for internet-facing services)

```bash
# Set metrics token environment variable
export SERVO_METRICS_TOKEN="$(openssl rand -base64 32)"

# Configure your monitoring system (Prometheus, Datadog, etc.) to include the token
# Example Prometheus scrape config:
scrape_configs:
  - job_name: 'servo-worker'
    bearer_token: 'your-metrics-token-here'
    static_configs:
      - targets: ['servo-worker.run.app:443']
```

#### Option 2: Internal-Only Access (Recommended for VPC deployments)

Restrict `/metrics` to internal traffic only:

```bash
# Cloud Run: Use ingress settings
gcloud run services update servo-worker \
  --ingress=internal  # Only accessible within VPC

# GKE: Use NetworkPolicy
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-metrics-from-prometheus
spec:
  podSelector:
    matchLabels:
      app: servo-worker
  ingress:
  - from:
    - podSelector:
        matchLabels:
          app: prometheus
    ports:
    - port: 8080
      protocol: TCP
```

#### Option 3: Separate Metrics Port (Advanced)

Run metrics on a separate internal-only port (requires code changes).

### Metrics Cardinality Control

To prevent high-cardinality metrics in multi-tenant environments:

```bash
# Disable per-tenant metrics (recommended for >100 tenants)
export SERVO_METRICS_INCLUDE_TENANT_ID=false
```

This disables `servo_enqueue_total_by_tenant` and keeps only aggregate `servo_enqueue_total`
metrics. Use structured logs with tenant_id for per-tenant debugging instead.

## Features

### Idempotency Enforcement

The orchestrator enforces idempotency for workflow executions. When an idempotency key is provided:

1. The orchestrator checks for an existing execution with the same `(tenant_id, workflow_id, idempotency_key)` tuple
2. If found, returns the existing `execution_id` instead of creating a duplicate
3. If not found, creates a new execution with the idempotency key

This prevents duplicate workflow executions when clients retry failed API calls.

### Authentication & Security

- **OIDC Authentication**: Cloud Tasks automatically injects OIDC tokens for Cloud Run worker authentication using the
  service account
- **OAuth2 Access Tokens**: Used to authenticate API calls to Cloud Tasks
- **HMAC Signing**: All task payloads are signed with HMAC-SHA256 for integrity verification

### Retry & Error Handling

- **5xx Server Errors**: Retried up to 3 times with exponential backoff (100ms → 200ms → 400ms) plus random jitter
- **429 Rate Limiting**: Retried with backoff, honoring `Retry-After` header if present
- **4xx Client Errors**: Not retried (except 429)
- **HTTP Timeout**: 30-second timeout on all Cloud Tasks API calls

## Current Limitations

### Execution Plan Pre-compilation

**Status**: Not implemented

The `execution_plan` parameter in `enqueue()` is currently always an empty vector. Execution plans are not
pre-compiled by the orchestrator.

**Current Behavior**: The worker receives an empty execution plan and must:

1. Fetch the workflow from storage
2. Compile the execution plan from the workflow DAG at runtime

**Future Enhancement**: The orchestrator should pre-compile the execution plan (topological sort of workflow assets)
before enqueueing, reducing worker startup latency and ensuring plan consistency.

## Testing

```bash
cargo test -p servo-cloud-gcp
```

## Documentation

For more details, see the [GCP deployment guide](https://docs.servo.dev/guides/deployment-gcp).
