# servo-cloud-gcp

Google Cloud Platform adapter for Servo, providing integration with Cloud Tasks, Cloud Run, and Cloud Logging.

## Overview

The `servo-cloud-gcp` crate enables Servo to run on Google Cloud Platform:

- **Cloud Run Executor**: Executes workflows on Cloud Run
- **Cloud Tasks Queue**: Schedules tasks using Cloud Tasks
- **Authentication**: GCP service account integration
- **Monitoring**: Cloud Logging and Cloud Monitoring integration

## Usage

```rust
use servo_cloud_gcp::{CloudRunExecutor, CloudTasksQueue};

// Create executor
let executor = CloudRunExecutor::new(
    "my-project".to_string(),
    "us-central1".to_string(),
    "servo-worker".to_string(),
);

// Create queue
let queue = CloudTasksQueue::new(
    "my-project".to_string(),
    "us-central1".to_string(),
    "servo-tasks".to_string(),
);

// Execute workflow
let result = executor.execute(workflow_id).await?;
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

- Cloud Run API
- Cloud Tasks API
- Cloud Logging API
- Cloud Monitoring API

## Testing

```bash
cargo test -p servo-cloud-gcp
```

## Documentation

For more details, see the [GCP deployment guide](https://docs.servo.dev/guides/deployment-gcp).
