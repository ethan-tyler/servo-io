# servo-cli

Command-line interface tool for the Servo orchestration platform.

## Overview

The Servo CLI provides commands for managing workflows, executions, and lineage:

- `servo init` - Initialize metadata database
- `servo migrate` - Run database migrations
- `servo deploy` - Deploy a workflow
- `servo run` - Execute a workflow
- `servo status` - Check execution status
- `servo lineage` - Show asset lineage

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
