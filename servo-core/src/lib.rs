//! # Servo Core
//!
//! Core orchestration engine for Servo, providing workflow compilation,
//! asset management, and execution scheduling.

pub mod asset;
pub mod compiler;
pub mod partition_context;
pub mod partition_mapping;
pub mod partition_validator;
pub mod quality;
pub mod registry;
pub mod scheduler;
pub mod workflow;

// Re-export commonly used types
pub use asset::{
    Asset, AssetBuilder, AssetDependency, AssetId, AssetMetadata, DependencyType, PartitionConfig,
    PartitionType, TimeGranularity,
};
pub use partition_context::PartitionExecutionContext;
pub use compiler::{CompileError, ExecutionPlan, WorkflowCompiler};
pub use partition_mapping::{
    infer_mapping, PartitionMapper, PartitionMapping, PartitionMappingError,
};
pub use partition_validator::{PartitionConfigExt, PartitionValidator, ValidationError};
pub use quality::{
    AssetCheck, AssetCheckBuilder, CheckExecutionSummary, CheckId, CheckOutcome, CheckResult,
    CheckSeverity, CheckType,
};
pub use registry::{AssetRegistry, RegistryError};
pub use scheduler::{Schedule, ScheduleError, Scheduler};
pub use workflow::{Workflow, WorkflowId, WorkflowMetadata};

/// Result type for Servo core operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for Servo core operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Compilation error: {0}")]
    Compile(#[from] CompileError),

    #[error("Registry error: {0}")]
    Registry(#[from] RegistryError),

    #[error("Schedule error: {0}")]
    Schedule(#[from] ScheduleError),

    #[error("Internal error: {0}")]
    Internal(String),
}
