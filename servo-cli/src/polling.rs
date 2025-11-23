//! Status polling for workflow executions

use anyhow::{Context, Result};
use servo_runtime::ExecutionState;
use servo_storage::{models::ExecutionModel, postgres::PostgresStorage, TenantId};
use std::time::Duration;
use tokio::time::{interval, timeout};
use tracing::{debug, info};
use uuid::Uuid;

/// Check if an execution state is terminal
pub fn is_terminal_state(state: &str) -> bool {
    matches!(
        state,
        "succeeded" | "failed" | "cancelled" | "timeout"
    )
}

/// Parse state string into ExecutionState enum
pub fn parse_execution_state(state: &str) -> Option<ExecutionState> {
    match state {
        "pending" => Some(ExecutionState::Pending),
        "running" => Some(ExecutionState::Running),
        "succeeded" => Some(ExecutionState::Succeeded),
        "failed" => Some(ExecutionState::Failed),
        "cancelled" => Some(ExecutionState::Cancelled),
        "timeout" => Some(ExecutionState::Timeout),
        _ => None,
    }
}

/// Display execution status to the user
pub fn display_status(execution: &ExecutionModel) {
    let status_icon = match execution.state.as_str() {
        "pending" => "⏳",
        "running" => "🔄",
        "succeeded" => "✅",
        "failed" => "❌",
        "cancelled" => "🚫",
        "timeout" => "⏱️",
        _ => "❓",
    };

    info!(
        "{} Execution {} is {}",
        status_icon, execution.id, execution.state
    );

    // Show error message if failed
    if execution.state == "failed" {
        if let Some(error) = &execution.error_message {
            info!("   Error: {}", error);
        }
    }

    // Show timing information if available
    if let Some(started_at) = execution.started_at {
        let elapsed = chrono::Utc::now()
            .signed_duration_since(started_at)
            .to_std()
            .unwrap_or_default();
        info!("   Runtime: {:.1}s", elapsed.as_secs_f64());
    }
}

/// Poll execution status until it reaches a terminal state
///
/// Returns the final execution state or times out
pub async fn poll_execution_status(
    execution_id: Uuid,
    tenant_id: &TenantId,
    storage: &PostgresStorage,
    timeout_duration: Duration,
    poll_interval_duration: Duration,
) -> Result<ExecutionState> {
    debug!(
        "Starting to poll execution {} with timeout {:?} and interval {:?}",
        execution_id, timeout_duration, poll_interval_duration
    );

    let mut poll_ticker = interval(poll_interval_duration);

    let result = timeout(timeout_duration, async {
        loop {
            poll_ticker.tick().await;

            // Fetch current execution state
            let execution = storage
                .get_execution(execution_id, tenant_id)
                .await
                .context("Failed to fetch execution status")?;

            debug!(
                "Polled execution {}: state = {}",
                execution_id, execution.state
            );

            // Display status update
            display_status(&execution);

            // Check if terminal state reached
            if is_terminal_state(&execution.state) {
                let final_state = parse_execution_state(&execution.state)
                    .context("Invalid execution state")?;
                info!("Execution {} completed with state: {:?}", execution_id, final_state);
                return Ok(final_state);
            }
        }
    })
    .await;

    match result {
        Ok(state_result) => state_result,
        Err(_) => {
            anyhow::bail!(
                "Execution {} did not complete within timeout of {:?}",
                execution_id,
                timeout_duration
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_terminal_state() {
        assert!(is_terminal_state("succeeded"));
        assert!(is_terminal_state("failed"));
        assert!(is_terminal_state("cancelled"));
        assert!(is_terminal_state("timeout"));

        assert!(!is_terminal_state("pending"));
        assert!(!is_terminal_state("running"));
    }

    #[test]
    fn test_parse_execution_state() {
        assert_eq!(
            parse_execution_state("pending"),
            Some(ExecutionState::Pending)
        );
        assert_eq!(
            parse_execution_state("running"),
            Some(ExecutionState::Running)
        );
        assert_eq!(
            parse_execution_state("succeeded"),
            Some(ExecutionState::Succeeded)
        );
        assert_eq!(
            parse_execution_state("failed"),
            Some(ExecutionState::Failed)
        );
        assert_eq!(
            parse_execution_state("cancelled"),
            Some(ExecutionState::Cancelled)
        );
        assert_eq!(
            parse_execution_state("timeout"),
            Some(ExecutionState::Timeout)
        );
        assert_eq!(parse_execution_state("invalid"), None);
    }
}
