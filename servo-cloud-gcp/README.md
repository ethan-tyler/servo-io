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

## Required GCP APIs

Enable these APIs in your project:

- Cloud Tasks API

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
