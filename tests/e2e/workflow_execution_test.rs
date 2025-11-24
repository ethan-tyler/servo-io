//! End-to-end workflow execution tests
//!
//! These tests validate the complete workflow execution lifecycle from
//! CLI invocation to terminal state, using a real PostgreSQL database.
//!
//! ## Running these tests
//! ```bash
//! # Set up test database
//! export TEST_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//!
//! # Run E2E tests
//! cargo test --test workflow_execution_test -- --ignored --test-threads=1
//! ```

use servo_runtime::ExecutionState;
use std::time::Duration;

mod e2e;
use e2e::{fixtures, test_harness::E2ETestHarness};

#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -- --ignored
async fn test_happy_path_workflow_execution() {
    // Setup test harness
    let harness = E2ETestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Create a simple workflow
    let (name, description) = fixtures::simple_workflow();
    let workflow_id = harness
        .create_workflow(name, description)
        .await
        .expect("Failed to create workflow");

    // Start execution
    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start workflow execution");

    // Verify execution was created in Pending state
    let execution = harness
        .storage
        .get_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to fetch execution");

    assert_eq!(execution.state, "Pending");
    assert_eq!(execution.workflow_id, workflow_id);

    // Simulate worker processing: Pending -> Running -> Succeeded
    harness
        .transition_execution_state(execution_id, ExecutionState::Pending, ExecutionState::Running)
        .await
        .expect("Failed to transition to Running");

    harness
        .transition_execution_state(
            execution_id,
            ExecutionState::Running,
            ExecutionState::Succeeded,
        )
        .await
        .expect("Failed to transition to Succeeded");

    // Wait for completion
    let final_state = harness
        .wait_for_completion(execution_id, Duration::from_secs(5))
        .await
        .expect("Execution did not complete");

    // Verify final state
    assert_eq!(final_state, ExecutionState::Succeeded);

    // Verify execution record
    let final_execution = harness
        .storage
        .get_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to fetch final execution");

    assert_eq!(final_execution.state, "Succeeded");
    assert!(final_execution.started_at.is_some());
    assert!(final_execution.completed_at.is_some());
    assert!(final_execution.error_message.is_none());

    // Cleanup
    harness
        .teardown()
        .await
        .expect("Failed to teardown harness");
}

#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -- --ignored
async fn test_failing_workflow_execution() {
    // Setup test harness
    let harness = E2ETestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Create a workflow that will fail
    let (name, description) = fixtures::failing_workflow();
    let workflow_id = harness
        .create_workflow(name, description)
        .await
        .expect("Failed to create workflow");

    // Start execution
    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start workflow execution");

    // Simulate worker processing: Pending -> Running -> Failed
    harness
        .transition_execution_state(execution_id, ExecutionState::Pending, ExecutionState::Running)
        .await
        .expect("Failed to transition to Running");

    harness
        .transition_execution_state(execution_id, ExecutionState::Running, ExecutionState::Failed)
        .await
        .expect("Failed to transition to Failed");

    // Wait for completion
    let final_state = harness
        .wait_for_completion(execution_id, Duration::from_secs(5))
        .await
        .expect("Execution did not complete");

    // Verify final state is Failed
    assert_eq!(final_state, ExecutionState::Failed);

    // Verify execution record
    let final_execution = harness
        .storage
        .get_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to fetch final execution");

    assert_eq!(final_execution.state, "Failed");
    assert!(final_execution.started_at.is_some());
    assert!(final_execution.completed_at.is_some());

    // Cleanup
    harness
        .teardown()
        .await
        .expect("Failed to teardown harness");
}

#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -- --ignored
async fn test_workflow_with_idempotency_key() {
    // Setup test harness
    let harness = E2ETestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Create workflow
    let (name, description) = fixtures::simple_workflow();
    let workflow_id = harness
        .create_workflow(name, description)
        .await
        .expect("Failed to create workflow");

    // Start execution with idempotency key
    let execution_id_1 = harness
        .orchestrator
        .start_execution(
            workflow_id,
            &harness.tenant,
            Some("test-idempotency-key-123".to_string()),
        )
        .await
        .expect("Failed to start first execution");

    // Attempt to start same execution again with same idempotency key
    let execution_id_2 = harness
        .orchestrator
        .start_execution(
            workflow_id,
            &harness.tenant,
            Some("test-idempotency-key-123".to_string()),
        )
        .await
        .expect("Failed to start second execution");

    // Should return the same execution ID
    assert_eq!(
        execution_id_1, execution_id_2,
        "Idempotency key should return same execution"
    );

    // Cleanup
    harness
        .teardown()
        .await
        .expect("Failed to teardown harness");
}
