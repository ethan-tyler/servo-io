//! Workflow fixtures for E2E testing
//!
//! Provides predefined workflow definitions for common test scenarios.

/// Simple workflow that should succeed
///
/// This workflow has minimal configuration and is designed to test
/// the happy path of workflow execution.
pub fn simple_workflow() -> (&'static str, &'static str) {
    (
        "simple-success-workflow",
        "A simple workflow that succeeds immediately"
    )
}

/// Workflow that should fail
///
/// This workflow is designed to test failure handling in the execution flow.
pub fn failing_workflow() -> (&'static str, &'static str) {
    (
        "failing-workflow",
        "A workflow designed to fail for testing purposes"
    )
}

/// Workflow that simulates a timeout scenario
pub fn timeout_workflow() -> (&'static str, &'static str) {
    (
        "timeout-workflow",
        "A workflow that takes longer than the timeout duration"
    )
}

/// Multi-step workflow for testing complex execution flows
pub fn multi_step_workflow() -> (&'static str, &'static str) {
    (
        "multi-step-workflow",
        "A workflow with multiple sequential steps"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_names_are_unique() {
        let fixtures = vec![
            simple_workflow().0,
            failing_workflow().0,
            timeout_workflow().0,
            multi_step_workflow().0,
        ];

        // Check for unique names
        let mut unique = fixtures.clone();
        unique.sort();
        unique.dedup();

        assert_eq!(
            fixtures.len(),
            unique.len(),
            "Fixture names must be unique"
        );
    }
}
