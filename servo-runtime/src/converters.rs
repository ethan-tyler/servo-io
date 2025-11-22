//! Converters between storage models and runtime domain types.

use crate::{ExecutionState, Result};
use chrono::{DateTime, Utc};
use servo_storage::ExecutionModel;
use uuid::Uuid;

/// Domain representation of an execution with typed state.
#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub state: ExecutionState,
    pub tenant_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<ExecutionModel> for ExecutionRecord {
    type Error = crate::Error;

    fn try_from(model: ExecutionModel) -> Result<Self> {
        let state: ExecutionState = model.state.as_str().try_into()?;
        Ok(Self {
            id: model.id,
            workflow_id: model.workflow_id,
            state,
            tenant_id: model.tenant_id,
            started_at: model.started_at,
            completed_at: model.completed_at,
            error_message: model.error_message,
            created_at: model.created_at,
            updated_at: model.updated_at,
        })
    }
}

impl From<&ExecutionRecord> for ExecutionModel {
    fn from(record: &ExecutionRecord) -> Self {
        Self {
            id: record.id,
            workflow_id: record.workflow_id,
            state: record.state.into(),
            tenant_id: record.tenant_id.clone(),
            started_at: record.started_at,
            completed_at: record.completed_at,
            error_message: record.error_message.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_to_record_roundtrip() {
        let model = ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            state: "running".to_string(),
            tenant_id: Some("tenant123".to_string()),
            started_at: Some(Utc::now()),
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let record: ExecutionRecord = model.clone().try_into().expect("conversion should work");
        assert_eq!(record.id, model.id);
        assert_eq!(record.workflow_id, model.workflow_id);
        assert_eq!(record.state, ExecutionState::Running);

        let back: ExecutionModel = (&record).into();
        assert_eq!(back.state, "running".to_string());
        assert_eq!(back.tenant_id, model.tenant_id);
    }

    #[test]
    fn test_invalid_state_is_rejected() {
        let model = ExecutionModel {
            id: Uuid::new_v4(),
            workflow_id: Uuid::new_v4(),
            state: "bogus".to_string(),
            tenant_id: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let result: Result<ExecutionRecord> = model.try_into();
        assert!(matches!(result, Err(crate::Error::InvalidState(_))));
    }
}
