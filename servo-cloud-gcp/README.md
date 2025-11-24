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

Set up GCP credentials:

```bash
# Using service account key
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/key.json

# Or use Application Default Credentials (ADC)
gcloud auth application-default login
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
