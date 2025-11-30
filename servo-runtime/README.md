# servo-runtime

Execution runtime for Servo workflows, providing state management, retry logic,
concurrency control, and execution orchestration with Row-Level Security (RLS)
enforcement.

## Overview

The `servo-runtime` crate manages the execution lifecycle of Servo workflows
with PostgreSQL persistence and multi-tenant isolation:

- **ExecutionOrchestrator**: Stateless orchestrator for managing execution lifecycle with RLS enforcement
- **BackfillExecutor**: Processes partition backfill jobs with pause/resume support and ETA tracking
- **State Machine**: Manages execution state transitions (Pending → Running → Succeeded/Failed/Cancelled/Timeout)
- **Retry Logic**: Configurable retry policies with exponential backoff + jitter and max elapsed time
- **Concurrency Control**: Limits parallel executions to prevent resource exhaustion
- **Executor Trait**: Interface for executing workflows on different backends

## Quick Start

### Basic Orchestrator Usage

```rust
use servo_runtime::{ExecutionOrchestrator, ExecutionState, RetryPolicy};
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to PostgreSQL with servo_app role (RLS enforced)
    let storage = Arc::new(
        PostgresStorage::new("postgresql://servo_app:servo_app@localhost/servo").await?
    );

    // Create orchestrator with default retry policy
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());

    let tenant = TenantId::new("my-tenant");
    let workflow_id = Uuid::parse_str("...")?;

    // Start execution
    let execution_id = orchestrator
        .start_execution(workflow_id, &tenant, None)
        .await?;

    // Transition to running
    orchestrator
        .transition_state(execution_id, ExecutionState::Pending, ExecutionState::Running, &tenant)
        .await?;

    // Complete successfully
    orchestrator
        .transition_state(execution_id, ExecutionState::Running, ExecutionState::Succeeded, &tenant)
        .await?;

    Ok(())
}
```

### Custom Retry Policy

```rust
use servo_runtime::RetryPolicy;
use std::time::Duration;

let retry_policy = RetryPolicy {
    max_attempts: 5,
    initial_delay: Duration::from_secs(1),
    max_delay: Duration::from_secs(30),
    max_elapsed: Duration::from_secs(300), // 5 minutes total
    strategy: RetryStrategy::ExponentialWithJitter,
    backoff_multiplier: 2.0,
};

// Execute with automatic retry on transient failures
retry_policy.execute_with_retry(|| async {
    // Your async operation here
    database_call().await
}).await?;
```

### Backfill Executor

The `BackfillExecutor` processes partition backfill jobs with built-in pause/resume support:

```rust
use servo_runtime::{BackfillExecutor, BackfillExecutorConfig, run_backfill_executor};
use servo_storage::{PostgresStorage, TenantId};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage = Arc::new(PostgresStorage::new("postgresql://...").await?);
    let orchestrator = ExecutionOrchestrator::new(storage.clone(), RetryPolicy::default());
    let tenant = TenantId::new("my-tenant");

    // Run backfill executor (polls for jobs and processes them)
    run_backfill_executor(
        storage,
        orchestrator,
        tenant,
        Some(BackfillExecutorConfig::default()),
    ).await?;

    Ok(())
}
```

**Backfill Job States:**

- `pending` → `running` (claimed by executor)
- `running` → `paused` (user requested pause)
- `paused` → `resuming` (user requested resume)
- `resuming` → `running` (claimed by executor)
- `running` → `completed` | `failed` | `cancelled`

**Key Features:**

- **Partition-boundary pause**: Current partition completes before stopping
- **Checkpoint persistence**: Progress saved for accurate resumption
- **ETA calculation**: EWMA-based estimation preserved across pause/resume
- **Atomic job claiming**: FOR UPDATE SKIP LOCKED prevents race conditions
- **Stale heartbeat recovery**: Auto-reclaim jobs from crashed executors

## Features

### ExecutionOrchestrator

- **Stateless & Thread-Safe**: Can be safely cloned and shared across async tasks
- **RLS Enforcement**: All operations respect PostgreSQL Row-Level Security policies
- **State Validation**: Invalid state transitions are rejected before persistence
- **Transaction Safety**: Multi-step operations wrapped in database transactions
- **Observability**: Comprehensive tracing with spans for all operations
- **Future-Proof**: Designed to accept idempotency keys (implementation planned)

**State Transitions:**

- `Pending → Running`
- `Running → Succeeded | Failed | Timeout | Cancelled`
- `Failed → Running` (retry)

### Retry Logic

**Full Jitter Algorithm** (AWS best practice):

```text
delay = random(0, min(max_delay, initial_delay * backoff_multiplier^attempt))
```

**Features:**

- Exponential backoff with full jitter to avoid thundering herd
- Maximum delay cap (`max_delay`)
- Maximum total elapsed time (`max_elapsed`)
- Retryable error classification

**Example:**

```rust
use servo_runtime::RetryPolicy;

let policy = RetryPolicy::default();

// Check if an error is retryable
if RetryPolicy::is_retryable(&error) {
    let delay = policy.calculate_delay_with_jitter(attempt);
    tokio::time::sleep(delay).await;
}
```

### State Machine

```rust
use servo_runtime::{StateMachine, ExecutionState};

let mut sm = StateMachine::new();
assert_eq!(sm.current_state(), ExecutionState::Pending);

// Valid transition
sm.transition(ExecutionState::Running)?;

// Invalid transition (returns error)
sm.transition(ExecutionState::Succeeded)?; // Error: must be Running first
```

## Testing

### Unit Tests

```bash
cargo test --package servo-runtime --lib
```

### Integration Tests

Integration tests validate the orchestrator against a real PostgreSQL database with RLS enforcement.

**Prerequisites:**
- PostgreSQL 15+ running locally
- Database setup with servo_app role (see `servo-storage/migrations`)
- Environment variables configured

**Setup:**
```bash
export TEST_APP_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
```

**Run Tests:**
```bash
# Run all integration tests
cargo test --package servo-runtime --all-features -- --ignored

# Run specific test
cargo test --package servo-runtime test_execution_lifecycle_with_rls -- --ignored
```

**Test Coverage:**

- Execution lifecycle (Pending → Running → Succeeded)
- Tenant isolation enforced by RLS
- Invalid state transition rejection
- State mismatch detection
- Failure recording with error messages
- Retry transitions (Failed → Running)
- Concurrent executions (10 parallel)
- Concurrent state transitions (5 parallel)

**Backfill Integration Tests:**

```bash
cargo test --package servo-runtime --test backfill_integration -- --ignored
```

- Pause running job at partition boundary
- Resume paused job from checkpoint
- Checkpoint persistence verification
- ETA preservation across pause/resume
- State transition validation (pause non-running fails)
- Tenant isolation for pause/resume operations

## Architecture

```
┌─────────────────────────────────────┐
│  servo-cli (deploy, run, status)   │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    ExecutionOrchestrator            │
│  ┌──────────────────────────────┐   │
│  │  start_execution()           │   │
│  │  transition_state()          │   │
│  │  record_failure()            │   │
│  └──────────────────────────────┘   │
│                                     │
│  State Validation + Tracing         │
│  ┌──────────────────────────────┐   │
│  │  is_valid_transition()       │   │
│  │  RetryPolicy                 │   │
│  └──────────────────────────────┘   │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    servo-storage                    │
│  PostgresStorage with RLS           │
│  ┌──────────────────────────────┐   │
│  │  create_execution()          │   │
│  │  update_execution()          │   │
│  │  get_execution()             │   │
│  └──────────────────────────────┘   │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│    PostgreSQL Database              │
│  Multi-tenant with RLS enforcement  │
└─────────────────────────────────────┘
```

## Error Handling

```rust
use servo_runtime::Error;

match orchestrator.start_execution(workflow_id, &tenant, None).await {
    Ok(execution_id) => println!("Started: {}", execution_id),
    Err(Error::Execution(msg)) => eprintln!("Business logic error: {}", msg),
    Err(Error::Internal(msg)) => eprintln!("Internal error: {}", msg),
    Err(Error::Timeout(msg)) => eprintln!("Timeout: {}", msg),
    Err(e) => eprintln!("Unknown error: {}", e),
}
```

**Error Types:**
- `Execution`: Business logic errors (invalid transitions, state mismatches)
- `Internal`: Database connection errors, storage failures
- `Timeout`: Operation timeouts
- `RetryExhausted`: Max retry attempts exceeded
- `ConcurrencyLimitExceeded`: Too many concurrent executions

## Observability

All orchestrator operations emit structured traces using the `tracing` crate.

**Trace Fields:**
- `execution_id`: UUID of the execution
- `workflow_id`: UUID of the workflow
- `tenant_id`: Tenant identifier
- `duration_ms`: Operation duration (for slow ops >100ms)
- `error`: Error details (on failure)

**Example:**
```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// Initialize JSON logging
tracing_subscriber::registry()
    .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
    .with(tracing_subscriber::fmt::layer().json())
    .init();
```

## Design Principles

1. **Stateless**: Orchestrator holds no mutable state; all state persisted to database
2. **Thread-Safe**: Can be cloned and used across multiple async tasks
3. **Type-Safe**: Strong typing with newtypes (ExecutionId, WorkflowId, TenantId)
4. **Error Context**: Rich error types with context using thiserror
5. **Observability First**: Tracing spans on all async functions
6. **Testable**: Dependency injection for storage, integration tests with real DB
7. **Database as Source of Truth**: All state changes persist to PostgreSQL
8. **Idempotent**: Execution operations designed to be safe to retry

## Future Enhancements (P1)

- **Execution Timeouts**: Auto-transition to Timeout state after deadline
- **Cancellation Support**: Graceful shutdown with cancel() method

## Implemented Features

- **Idempotency Keys**: Prevent duplicate execution creation ✓
- **Backfill Pause/Resume**: Pause at partition boundaries, resume from checkpoint ✓
- **ETA Calculation**: EWMA-based estimation with persistence ✓
- **Prometheus Metrics**: Low-cardinality metrics for jobs, partitions, ETA distribution ✓

## Documentation

For more details, see:
- [Servo Documentation](https://docs.servo.dev)
- [API Documentation](https://docs.rs/servo-runtime)
- [servo-storage README](../servo-storage/README.md) - Database layer with RLS

## License

Apache-2.0
