//! Workflow scheduler
//!
//! The scheduler determines when and how workflows should be executed.

use crate::workflow::WorkflowId;
use chrono::{DateTime, Utc};
use thiserror::Error;

/// Errors that can occur during scheduling
#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("Invalid schedule: {0}")]
    InvalidSchedule(String),

    #[error("Workflow not found: {0:?}")]
    WorkflowNotFound(WorkflowId),

    #[error("Scheduling conflict: {0}")]
    SchedulingConflict(String),
}

/// Schedule types for workflows
#[derive(Debug, Clone)]
pub enum Schedule {
    /// Run immediately
    Immediate,

    /// Run at a specific time
    At(DateTime<Utc>),

    /// Run on a cron schedule
    Cron(String),

    /// Run when dependencies are satisfied
    OnDependency,
}

/// Scheduler for managing workflow executions
pub struct Scheduler {
    schedules: Vec<(WorkflowId, Schedule)>,
}

impl Scheduler {
    /// Create a new scheduler
    pub fn new() -> Self {
        Self {
            schedules: Vec::new(),
        }
    }

    /// Schedule a workflow
    pub fn schedule(
        &mut self,
        workflow_id: WorkflowId,
        schedule: Schedule,
    ) -> Result<(), ScheduleError> {
        // TODO: Validate schedule
        // TODO: Check for conflicts

        self.schedules.push((workflow_id, schedule));
        Ok(())
    }

    /// Get all scheduled workflows
    pub fn get_schedules(&self) -> &[(WorkflowId, Schedule)] {
        &self.schedules
    }

    /// Remove a workflow from the schedule
    pub fn unschedule(&mut self, workflow_id: &WorkflowId) -> bool {
        let initial_len = self.schedules.len();
        self.schedules.retain(|(id, _)| id != workflow_id);
        self.schedules.len() < initial_len
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_creation() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.get_schedules().len(), 0);
    }

    #[test]
    fn test_schedule_workflow() {
        let mut scheduler = Scheduler::new();
        let workflow_id = WorkflowId::new();

        assert!(scheduler.schedule(workflow_id, Schedule::Immediate).is_ok());
        assert_eq!(scheduler.get_schedules().len(), 1);
    }

    #[test]
    fn test_unschedule_workflow() {
        let mut scheduler = Scheduler::new();
        let workflow_id = WorkflowId::new();

        scheduler
            .schedule(workflow_id, Schedule::Immediate)
            .unwrap();
        assert!(scheduler.unschedule(&workflow_id));
        assert_eq!(scheduler.get_schedules().len(), 0);
    }
}
