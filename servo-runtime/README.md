# servo-runtime

Execution runtime for Servo workflows, providing state management, retry logic, and concurrency control.

## Overview

The `servo-runtime` crate manages the execution lifecycle of Servo workflows:

- **Executor**: Trait for executing workflows on different backends
- **State Machine**: Manages execution state transitions (pending -> running -> succeeded/failed)
- **Retry Logic**: Configurable retry policies with exponential backoff
- **Concurrency Control**: Limits parallel executions to prevent resource exhaustion

## Usage

```rust
use servo_runtime::{StateMachine, ExecutionState, RetryPolicy};

// Create a state machine
let mut sm = StateMachine::new();
assert_eq!(sm.current_state(), ExecutionState::Pending);

// Transition to running
sm.transition(ExecutionState::Running)?;

// Configure retry policy
let retry_policy = RetryPolicy {
    max_attempts: 3,
    strategy: RetryStrategy::ExponentialWithJitter,
    ..Default::default()
};

// Calculate retry delay
let delay = retry_policy.calculate_delay(1);
```

## Features

- Async execution with Tokio
- State machine with validation
- Exponential backoff with jitter
- Concurrency limiting
- Timeout support

## Testing

```bash
cargo test -p servo-runtime
```

## Documentation

For more details, see the [main Servo documentation](https://docs.servo.dev).
