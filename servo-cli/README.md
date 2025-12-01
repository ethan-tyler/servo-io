# servo-cli

Command-line interface tool for the Servo orchestration platform.

## Overview

The Servo CLI provides commands for managing workflows, executions, lineage, and backfills:

- `servo init` - Initialize metadata database
- `servo migrate` - Run database migrations
- `servo deploy` - Deploy a workflow
- `servo run` - Execute a workflow
- `servo status` - Check execution status
- `servo lineage` - Show asset lineage
- `servo backfill` - Manage backfill operations (create, pause, resume, cancel)

## Installation

```bash
# From source
cargo install --path servo-cli

# Or from crates.io (when published)
cargo install servo-cli
```

## Usage

### Initialize Database

```bash
servo init --database-url postgresql://localhost/servo
```

### Deploy a Workflow

```bash
servo deploy workflow.py
```

### Run a Workflow

```bash
# Simple run
servo run daily_etl

# With parameters
servo run daily_etl --params '{"date": "2025-01-15"}'
```

### Check Status

```bash
servo status <execution-id>
```

### Show Lineage

```bash
# Show full lineage
servo lineage customer_data

# Show only upstream
servo lineage customer_data --upstream

# Show only downstream
servo lineage customer_data --downstream
```

### Backfill Operations

```bash
# Create a backfill for a date range
servo backfill create daily_etl \
  --start 2024-01-01 \
  --end 2024-01-31

# Check backfill status with ETA
servo backfill status <job_id>

# List active backfills
servo backfill list --state running

# Pause a running backfill (stops at partition boundary)
servo backfill pause <job_id>

# Resume a paused backfill (continues from checkpoint)
servo backfill resume <job_id>

# Cancel a backfill
servo backfill cancel <job_id> --reason "maintenance window"

# Watch backfill progress in real-time
servo backfill status <job_id> --watch
```

**Pause/Resume Notes:**

- Pause stops at the next partition boundary (current partition completes)
- Resume picks up from the checkpoint, preserving progress
- ETA estimates are preserved across pause/resume cycles

## Configuration

Set environment variables:

```bash
export DATABASE_URL=postgresql://localhost/servo
export CLOUD_PROVIDER=gcp
export PROJECT_ID=my-project
export REGION=us-central1
```

Or create a `.servo/config.toml` file:

```toml
database_url = "postgresql://localhost/servo"
cloud_provider = "gcp"
project_id = "my-project"
region = "us-central1"
```

## Development

```bash
# Run locally
cargo run --bin servo -- init --database-url postgresql://localhost/servo

# Run with verbose logging
cargo run --bin servo -- -v run my_workflow
```

## Documentation

For more details, see the [main Servo documentation](https://docs.servo.dev).
