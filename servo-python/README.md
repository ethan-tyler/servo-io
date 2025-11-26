# Servo Python SDK

Python SDK for Servo - an asset-centric data orchestration platform.

## Installation

```bash
pip install servo-orchestrator
```

With optional dependencies:

```bash
# For pandas support
pip install servo-orchestrator[pandas]

# For polars support
pip install servo-orchestrator[polars]

# For development
pip install servo-orchestrator[dev]
```

## Quick Start

```python
from servo import asset, workflow, ServoClient

# Define an asset
@asset(
    name="raw_orders",
    description="Raw orders from the source database",
)
def raw_orders() -> dict:
    return {"data": [...]}

# Define a dependent asset
@asset(
    name="processed_orders",
    dependencies=["raw_orders"],
    metadata={"owner": "data-team"},
)
def processed_orders(raw_orders: dict) -> dict:
    # Transform the raw data
    return {"processed": raw_orders["data"]}

# Define a workflow
@workflow(
    name="orders_pipeline",
    schedule="0 * * * *",  # Every hour
)
def orders_pipeline():
    return [raw_orders, processed_orders]

# Interact with Servo API
client = ServoClient(base_url="https://api.servo.io", api_key="...")
client.materialize("processed_orders")
```

## Features

- **Asset-centric**: Define data assets with dependencies, metadata, and lineage
- **Type-safe**: Full type annotations and mypy support
- **Flexible**: Works with pandas, polars, or plain Python
- **Observable**: Built-in tracing and metrics integration

## Validation

### Dependency Validation

After registering all assets, validate that dependencies are satisfied:

```python
from servo import validate_dependencies, validate_dependencies_strict

# Get list of validation errors (non-throwing)
errors = validate_dependencies()
if errors:
    print(f"Missing dependencies: {errors}")

# Or raise an error if any dependencies are missing
validate_dependencies_strict()  # Raises ServoValidationError
```

### Duplicate Detection

Asset and workflow names must be unique. Attempting to register a duplicate raises `ServoValidationError`:

```python
@asset(name="my_asset")
def first(): ...

@asset(name="my_asset")  # Raises ServoValidationError
def second(): ...
```

### Cron Schedule Validation

Workflow schedules are validated using standard 5-part cron format:

```text
minute (0-59) hour (0-23) day-of-month (1-31) month (1-12) day-of-week (0-6)
```

Supported syntax: `*`, ranges (`1-5`), steps (`*/15`), lists (`1,15,30`)

```python
@workflow(schedule="0 9-17 * * 1-5")  # Business hours, weekdays
def business_workflow(): ...

@workflow(schedule="*/15 * * * *")  # Every 15 minutes
def frequent_workflow(): ...
```

## Development

```bash
# Install development dependencies
pip install -e ".[dev]"

# Run tests
pytest

# Type checking
mypy servo

# Linting
ruff check servo
ruff format servo
```

## License

Apache-2.0
