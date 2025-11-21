//! Execution state machine

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// State of a workflow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionState {
    /// Execution is pending
    Pending,

    /// Execution is running
    Running,

    /// Execution succeeded
    Succeeded,

    /// Execution failed
    Failed,

    /// Execution was cancelled
    Cancelled,

    /// Execution timed out
    Timeout,
}

/// State machine for managing execution lifecycle
pub struct StateMachine {
    current_state: ExecutionState,
    state_history: Vec<(ExecutionState, DateTime<Utc>)>,
}

impl StateMachine {
    /// Create a new state machine
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            current_state: ExecutionState::Pending,
            state_history: vec![(ExecutionState::Pending, now)],
        }
    }

    /// Get the current state
    pub fn current_state(&self) -> ExecutionState {
        self.current_state
    }

    /// Transition to a new state
    pub fn transition(&mut self, new_state: ExecutionState) -> Result<(), String> {
        // Validate state transition
        if !self.is_valid_transition(new_state) {
            return Err(format!(
                "Invalid transition from {:?} to {:?}",
                self.current_state, new_state
            ));
        }

        self.current_state = new_state;
        self.state_history.push((new_state, Utc::now()));

        Ok(())
    }

    /// Check if a state transition is valid
    fn is_valid_transition(&self, new_state: ExecutionState) -> bool {
        use ExecutionState::*;

        match (self.current_state, new_state) {
            (Pending, Running) => true,
            (Running, Succeeded | Failed | Cancelled | Timeout) => true,
            _ => false,
        }
    }

    /// Get the state history
    pub fn history(&self) -> &[(ExecutionState, DateTime<Utc>)] {
        &self.state_history
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let sm = StateMachine::new();
        assert_eq!(sm.current_state(), ExecutionState::Pending);
    }

    #[test]
    fn test_valid_transition() {
        let mut sm = StateMachine::new();
        assert!(sm.transition(ExecutionState::Running).is_ok());
        assert_eq!(sm.current_state(), ExecutionState::Running);
    }

    #[test]
    fn test_invalid_transition() {
        let mut sm = StateMachine::new();
        assert!(sm.transition(ExecutionState::Succeeded).is_err());
    }
}
