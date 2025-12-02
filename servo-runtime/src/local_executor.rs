//! Local executor for development and testing
//!
//! This executor runs workflows directly in the current process without
//! requiring cloud infrastructure. It's useful for:
//!
//! - Local development and debugging
//! - CI/CD testing without cloud credentials
//! - Demos and presentations
//!
//! # Architecture
//!
//! Unlike the [`CloudRunExecutor`](servo_cloud_gcp::executor::CloudRunExecutor),
//! the LocalExecutor performs **synchronous** execution in the same process:
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────────┐
//! │  LocalExecutor  │────▶│   PostgreSQL    │
//! │  (this module)  │     │   (state only)  │
//! └─────────────────┘     └─────────────────┘
//!         │
//!         ▼
//!   Python subprocess (via tokio::process)
//! ```
//!
//! ## Execution Flow
//!
//! 1. **execute()**: Creates execution record (state: `pending`)
//! 2. **run_workflow_locally()**: Updates state to `running`, simulates workflow
//! 3. **completion**: Updates state to `succeeded` or `failed`
//!
//! ## Important Limitations
//!
//! - **Synchronous**: Blocks until completion (unlike CloudRunExecutor)
//! - **Single process**: No distributed execution or scaling
//! - **Best-effort cancellation**: Uses in-memory flag checked between assets
//! - **Requires compute_fn_module/function**: Assets must have these fields populated
//!
//! ## How It Works
//!
//! 1. Retrieves workflow assets via `get_workflow_assets()` (pre-sorted by position)
//! 2. Executes each asset via Python subprocess using `compute_fn_module` and `compute_fn_function`
//! 3. Captures stdout/stderr and handles failures
//!
//! # Python Environment Configuration
//!
//! The LocalExecutor runs Python code via subprocess. To ensure your Python
//! modules can be imported, configure the environment appropriately:
//!
//! ## Option 1: PYTHONPATH Environment Variable
//!
//! Add your project directory to `PYTHONPATH` so Python can find your modules:
//!
//! ```bash
//! export PYTHONPATH="/path/to/your/project:$PYTHONPATH"
//! servo run my-workflow --local
//! ```
//!
//! Or configure via `LocalExecutorConfig`:
//!
//! ```rust,ignore
//! let mut config = LocalExecutorConfig::default();
//! config.env_vars.insert(
//!     "PYTHONPATH".to_string(),
//!     "/path/to/your/project".to_string()
//! );
//! let executor = LocalExecutor::with_config(storage, tenant_id, config);
//! ```
//!
//! ## Option 2: Virtual Environment
//!
//! If using a virtual environment, point `python_path` to the venv's Python:
//!
//! ```rust,ignore
//! let mut config = LocalExecutorConfig::default();
//! config.python_path = PathBuf::from("/path/to/venv/bin/python3");
//! let executor = LocalExecutor::with_config(storage, tenant_id, config);
//! ```
//!
//! Or ensure the venv is activated before running:
//!
//! ```bash
//! source /path/to/venv/bin/activate
//! servo run my-workflow --local
//! ```
//!
//! ## Option 3: Working Directory
//!
//! Set the working directory to your project root where Python packages are located:
//!
//! ```rust,ignore
//! let mut config = LocalExecutorConfig::default();
//! config.working_dir = PathBuf::from("/path/to/your/project");
//! let executor = LocalExecutor::with_config(storage, tenant_id, config);
//! ```
//!
//! ## Environment Variables Injected
//!
//! The executor automatically injects these environment variables for your Python code:
//!
//! - `SERVO_EXECUTION_ID`: UUID of the current execution
//! - `SERVO_ASSET_ID`: UUID of the current asset being executed
//! - `SERVO_ASSET_NAME`: Name of the current asset
//! - `SERVO_TENANT_ID`: Tenant ID for the execution
//!
//! Access them in your Python code:
//!
//! ```python
//! import os
//!
//! def my_asset_function():
//!     execution_id = os.environ.get("SERVO_EXECUTION_ID")
//!     asset_name = os.environ.get("SERVO_ASSET_NAME")
//!     print(f"Executing {asset_name} in execution {execution_id}")
//! ```
//!
//! # Security
//!
//! The executor implements several security measures:
//!
//! - **Input validation**: Module and function names are validated against a whitelist
//!   pattern to prevent command injection attacks
//! - **Output limits**: Stdout/stderr are limited to 10MB to prevent memory exhaustion
//! - **Timeout enforcement**: Processes are killed if they exceed the configured timeout
//! - **Process cleanup**: Zombie processes are properly cleaned up on timeout
//!
//! # Example
//!
//! ```rust,ignore
//! let executor = LocalExecutor::new(storage, tenant_id);
//! let result = executor.execute(workflow_id).await?;
//! println!("Execution completed: {:?}", result);
//! ```

use crate::executor::{ExecutionId, ExecutionResult};
use crate::metrics::{
    PYTHON_EXECUTIONS_IN_FLIGHT, PYTHON_EXECUTION_DURATION_SECONDS, PYTHON_EXECUTION_ERRORS_TOTAL,
    PYTHON_EXECUTION_TOTAL,
};
use crate::{Error, Executor, Result};
use async_trait::async_trait;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use servo_core::WorkflowId;
use servo_storage::models::{AssetModel, ExecutionModel};
use servo_storage::{PostgresStorage, TenantId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Maximum output size in bytes (10MB)
const MAX_OUTPUT_SIZE: usize = 10 * 1024 * 1024;

/// Regex pattern for valid Python identifiers (module and function names)
/// Allows: letters, digits, underscores, and dots (for module paths)
/// Must start with letter or underscore
static PYTHON_IDENTIFIER_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_.]*$").expect("Invalid regex pattern"));

/// Validate a Python identifier (module or function name) to prevent command injection
///
/// # Security
///
/// This function prevents command injection attacks by ensuring that module
/// and function names contain only valid Python identifier characters.
fn validate_python_identifier(identifier: &str, identifier_type: &str) -> Result<()> {
    // Check for empty string
    if identifier.is_empty() {
        return Err(Error::Execution(format!(
            "Invalid {}: cannot be empty",
            identifier_type
        )));
    }

    // Check length (reasonable limit for module/function names)
    if identifier.len() > 256 {
        return Err(Error::Execution(format!(
            "Invalid {}: exceeds maximum length of 256 characters",
            identifier_type
        )));
    }

    // Check pattern
    if !PYTHON_IDENTIFIER_PATTERN.is_match(identifier) {
        return Err(Error::Execution(format!(
            "Invalid {}: '{}' contains invalid characters. \
             Must start with letter or underscore and contain only \
             letters, digits, underscores, and dots.",
            identifier_type, identifier
        )));
    }

    // Check for path traversal attempts
    if identifier.contains("..") {
        return Err(Error::Execution(format!(
            "Invalid {}: '{}' contains path traversal sequence '..'",
            identifier_type, identifier
        )));
    }

    // Check for shell metacharacters (defense in depth)
    let dangerous_chars = [
        ';', '|', '&', '$', '`', '(', ')', '{', '}', '<', '>', '\\', '\n', '\r',
    ];
    for c in dangerous_chars {
        if identifier.contains(c) {
            return Err(Error::Execution(format!(
                "Invalid {}: '{}' contains forbidden character '{}'",
                identifier_type, identifier, c
            )));
        }
    }

    Ok(())
}

/// Truncate a string from the end, keeping the last N characters
fn truncate_end(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let start = s.len() - max_len;
        format!("...{}", &s[start..])
    }
}

/// Execution states as stored in the database
pub mod states {
    pub const PENDING: &str = "pending";
    pub const RUNNING: &str = "running";
    pub const SUCCEEDED: &str = "succeeded";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";
}

/// Configuration for the local executor
#[derive(Debug, Clone)]
pub struct LocalExecutorConfig {
    /// Path to Python interpreter (defaults to "python3")
    pub python_path: PathBuf,
    /// Working directory for execution (defaults to current dir)
    pub working_dir: PathBuf,
    /// Maximum execution timeout in seconds (defaults to 3600)
    pub timeout_seconds: u64,
    /// Environment variables to pass to Python process
    pub env_vars: HashMap<String, String>,
}

impl Default for LocalExecutorConfig {
    fn default() -> Self {
        Self {
            python_path: PathBuf::from("python3"),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            timeout_seconds: 3600,
            env_vars: HashMap::new(),
        }
    }
}

/// Local executor for running workflows directly in the current process
///
/// This executor is designed for development and testing scenarios where
/// you want to run workflows without cloud infrastructure. It:
///
/// 1. Creates execution records in PostgreSQL (same as cloud executor)
/// 2. Executes workflow assets locally via Python subprocess
/// 3. Updates execution state as processing completes
///
/// Unlike the CloudRunExecutor, this executes synchronously and blocks
/// until the workflow completes.
pub struct LocalExecutor {
    /// PostgreSQL storage for execution records
    storage: Arc<PostgresStorage>,
    /// Tenant context for multi-tenant isolation
    tenant_id: TenantId,
    /// Executor configuration (used when Python execution is implemented)
    #[allow(dead_code)]
    config: LocalExecutorConfig,
    /// In-memory state for cancelled executions
    cancelled: Arc<RwLock<std::collections::HashSet<Uuid>>>,
}

impl LocalExecutor {
    /// Create a new local executor with default configuration
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance
    /// * `tenant_id` - Tenant context for RLS
    pub fn new(storage: Arc<PostgresStorage>, tenant_id: TenantId) -> Self {
        Self::with_config(storage, tenant_id, LocalExecutorConfig::default())
    }

    /// Create a new local executor with custom configuration
    ///
    /// # Arguments
    ///
    /// * `storage` - PostgreSQL storage instance
    /// * `tenant_id` - Tenant context for RLS
    /// * `config` - Executor configuration
    pub fn with_config(
        storage: Arc<PostgresStorage>,
        tenant_id: TenantId,
        config: LocalExecutorConfig,
    ) -> Self {
        Self {
            storage,
            tenant_id,
            config,
            cancelled: Arc::new(RwLock::new(std::collections::HashSet::new())),
        }
    }

    /// Execute a workflow with optional idempotency key
    pub async fn execute_with_idempotency(
        &self,
        workflow_id: WorkflowId,
        idempotency_key: Option<String>,
    ) -> Result<ExecutionResult> {
        let execution_id = Uuid::new_v4();
        let now = Utc::now();

        // Create execution model
        let execution = ExecutionModel {
            id: execution_id,
            workflow_id: workflow_id.0,
            state: states::PENDING.to_string(),
            tenant_id: Some(self.tenant_id.as_str().to_string()),
            idempotency_key: idempotency_key.clone(),
            started_at: None,
            completed_at: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };

        // Try to create execution (handles idempotency)
        let (actual_execution_id, was_created) = self
            .storage
            .create_execution_or_get_existing(&execution, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create execution: {}", e)))?;

        if !was_created {
            // Return existing execution
            let existing = self
                .storage
                .get_execution(actual_execution_id, &self.tenant_id)
                .await
                .map_err(|e| Error::Internal(format!("Failed to get execution: {}", e)))?;

            return Ok(ExecutionResult {
                execution_id: ExecutionId(actual_execution_id),
                success: existing.state == states::SUCCEEDED,
                error: existing.error_message,
            });
        }

        // Update state to running
        self.update_execution_state(actual_execution_id, states::RUNNING, None)
            .await?;

        // Execute the workflow locally
        let result = self
            .run_workflow_locally(workflow_id, actual_execution_id)
            .await;

        // Update final state based on result
        match &result {
            Ok(_) => {
                self.update_execution_state(actual_execution_id, states::SUCCEEDED, None)
                    .await?;
            }
            Err(e) => {
                self.update_execution_state(
                    actual_execution_id,
                    states::FAILED,
                    Some(e.to_string()),
                )
                .await?;
            }
        }

        result
    }

    /// Run the workflow locally by executing each asset via Python subprocess
    ///
    /// This method:
    /// 1. Retrieves workflow assets from storage (pre-sorted by position/dependencies)
    /// 2. Executes each asset via Python subprocess
    /// 3. Checks for cancellation between assets
    /// 4. Returns success or failure based on execution results
    async fn run_workflow_locally(
        &self,
        workflow_id: WorkflowId,
        execution_id: Uuid,
    ) -> Result<ExecutionResult> {
        tracing::info!(
            workflow_id = %workflow_id.0,
            execution_id = %execution_id,
            "Starting local workflow execution"
        );

        // Check if already cancelled
        if self.cancelled.read().await.contains(&execution_id) {
            return Err(Error::Execution("Execution was cancelled".to_string()));
        }

        // Get workflow from storage to verify it exists
        let workflow = self
            .storage
            .get_workflow(workflow_id.0, &self.tenant_id)
            .await
            .map_err(|e| Error::NotFound(format!("Workflow not found: {}", e)))?;

        // Get workflow assets in execution order (pre-sorted by position)
        let assets = self
            .storage
            .get_workflow_assets(workflow_id.0, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get workflow assets: {}", e)))?;

        if assets.is_empty() {
            tracing::warn!(
                workflow_id = %workflow_id.0,
                workflow_name = %workflow.name,
                "Workflow has no assets to execute"
            );
            return Ok(ExecutionResult {
                execution_id: ExecutionId(execution_id),
                success: true,
                error: None,
            });
        }

        tracing::info!(
            workflow_name = %workflow.name,
            asset_count = assets.len(),
            "Executing {} assets in order",
            assets.len()
        );

        // Execute each asset in order
        for (index, asset) in assets.iter().enumerate() {
            // Check for cancellation between assets
            if self.cancelled.read().await.contains(&execution_id) {
                return Err(Error::Execution("Execution was cancelled".to_string()));
            }

            tracing::info!(
                asset_name = %asset.name,
                asset_id = %asset.id,
                position = index + 1,
                total = assets.len(),
                "Executing asset [{}/{}]",
                index + 1,
                assets.len()
            );

            // Execute the asset via Python subprocess
            self.execute_asset_python(asset, execution_id).await?;

            tracing::info!(
                asset_name = %asset.name,
                "Asset execution completed"
            );
        }

        tracing::info!(
            workflow_id = %workflow_id.0,
            execution_id = %execution_id,
            "Local workflow execution completed successfully"
        );

        Ok(ExecutionResult {
            execution_id: ExecutionId(execution_id),
            success: true,
            error: None,
        })
    }

    /// Execute a single asset via Python subprocess
    ///
    /// This method:
    /// 1. Validates that compute_fn_module and compute_fn_function are set
    /// 2. Validates module/function names to prevent command injection
    /// 3. Constructs Python code to import the module and call the function
    /// 4. Executes via tokio::process::Command with configured timeout
    /// 5. Properly kills the process on timeout
    /// 6. Enforces output size limits to prevent memory exhaustion
    /// 7. Captures stdout/stderr and returns error on failure
    ///
    /// # Security
    ///
    /// - Module and function names are validated against a whitelist pattern
    /// - Shell metacharacters are explicitly blocked
    /// - Output size is limited to prevent DoS via excessive output
    #[tracing::instrument(
        name = "local_executor.execute_asset_python",
        skip(self, asset),
        fields(
            asset_id = %asset.id,
            asset_name = %asset.name,
            module = ?asset.compute_fn_module,
            function = ?asset.compute_fn_function,
        )
    )]
    async fn execute_asset_python(&self, asset: &AssetModel, execution_id: Uuid) -> Result<()> {
        // Start timing for metrics
        let start_time = std::time::Instant::now();
        let tenant_id_str = self.tenant_id.as_str();

        // Track in-flight executions
        PYTHON_EXECUTIONS_IN_FLIGHT
            .with_label_values(&[tenant_id_str])
            .inc();

        // Use a helper to ensure we always decrement in-flight and record metrics
        let result = self.execute_asset_python_inner(asset, execution_id).await;

        // Record metrics based on outcome
        let elapsed = start_time.elapsed().as_secs_f64();
        let status = match &result {
            Ok(_) => "success",
            Err(Error::Timeout(_)) => "timeout",
            Err(_) => "failure",
        };

        PYTHON_EXECUTION_DURATION_SECONDS
            .with_label_values(&[status])
            .observe(elapsed);

        PYTHON_EXECUTION_TOTAL
            .with_label_values(&[tenant_id_str, status])
            .inc();

        // Record error type if applicable
        if let Err(ref e) = result {
            let error_type = match e {
                Error::Timeout(_) => "timeout",
                Error::Execution(msg) if msg.contains("no compute_fn_module") => "validation_error",
                Error::Execution(msg) if msg.contains("no compute_fn_function") => {
                    "validation_error"
                }
                Error::Execution(msg) if msg.contains("invalid characters") => "validation_error",
                Error::Execution(msg) if msg.contains("Failed to spawn") => "spawn_error",
                Error::Execution(msg) if msg.contains("ModuleNotFoundError") => "import_error",
                Error::Execution(msg) if msg.contains("ImportError") => "import_error",
                _ => "runtime_error",
            };
            PYTHON_EXECUTION_ERRORS_TOTAL
                .with_label_values(&[error_type])
                .inc();
        }

        // Decrement in-flight counter
        PYTHON_EXECUTIONS_IN_FLIGHT
            .with_label_values(&[tenant_id_str])
            .dec();

        result
    }

    /// Inner implementation of Python asset execution (metrics recorded by wrapper)
    async fn execute_asset_python_inner(
        &self,
        asset: &AssetModel,
        execution_id: Uuid,
    ) -> Result<()> {
        let module = asset.compute_fn_module.as_ref().ok_or_else(|| {
            Error::Execution(format!(
                "Asset '{}' has no compute_fn_module - cannot execute.\n\
                 Set compute_fn_module in the asset definition to specify the Python module.",
                asset.name
            ))
        })?;

        let function = asset.compute_fn_function.as_ref().ok_or_else(|| {
            Error::Execution(format!(
                "Asset '{}' has no compute_fn_function - cannot execute.\n\
                 Set compute_fn_function in the asset definition to specify the Python function.",
                asset.name
            ))
        })?;

        // SECURITY: Validate module and function names to prevent command injection
        validate_python_identifier(module, "compute_fn_module")?;
        validate_python_identifier(function, "compute_fn_function")?;

        // Build Python code to import and execute the function
        let python_code = format!(
            r#"
import sys
import json

try:
    from {} import {}
    result = {}()
    print(json.dumps({{"status": "success", "asset": "{}", "result": str(result) if result is not None else None}}))
except Exception as e:
    import traceback
    print(json.dumps({{"status": "error", "asset": "{}", "error": str(e), "traceback": traceback.format_exc()}}), file=sys.stderr)
    sys.exit(1)
"#,
            module, function, function, asset.name, asset.name
        );

        tracing::debug!(
            python_path = ?self.config.python_path,
            working_dir = ?self.config.working_dir,
            "Executing Python subprocess"
        );

        // Spawn process with piped stdout/stderr for controlled reading
        let mut child = tokio::process::Command::new(&self.config.python_path)
            .arg("-c")
            .arg(&python_code)
            .current_dir(&self.config.working_dir)
            .envs(&self.config.env_vars)
            .env("SERVO_EXECUTION_ID", execution_id.to_string())
            .env("SERVO_ASSET_ID", asset.id.to_string())
            .env("SERVO_ASSET_NAME", &asset.name)
            .env("SERVO_TENANT_ID", self.tenant_id.as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::Execution(format!(
                    "Failed to spawn Python process for asset '{}': {}\n\
                     Verify that '{}' is installed and accessible.",
                    asset.name,
                    e,
                    self.config.python_path.display()
                ))
            })?;

        // Execute with timeout and proper cleanup
        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);

        let result = tokio::time::timeout(timeout_duration, async {
            // Read stdout and stderr with size limits
            let stdout_handle = child.stdout.take().unwrap();
            let stderr_handle = child.stderr.take().unwrap();

            let mut stdout_buf = Vec::with_capacity(4096);
            let mut stderr_buf = Vec::with_capacity(4096);

            // Read stdout with size limit
            let stdout_result = stdout_handle
                .take(MAX_OUTPUT_SIZE as u64)
                .read_to_end(&mut stdout_buf)
                .await;

            // Read stderr with size limit
            let stderr_result = stderr_handle
                .take(MAX_OUTPUT_SIZE as u64)
                .read_to_end(&mut stderr_buf)
                .await;

            // Wait for process to complete
            let status = child.wait().await?;

            Ok::<_, std::io::Error>((status, stdout_buf, stderr_buf, stdout_result, stderr_result))
        })
        .await;

        match result {
            Ok(Ok((status, stdout_buf, stderr_buf, stdout_result, stderr_result))) => {
                // Check for output size limits
                if stdout_result.is_ok() && stdout_buf.len() >= MAX_OUTPUT_SIZE {
                    tracing::warn!(
                        asset_name = %asset.name,
                        "Stdout output truncated at {} bytes",
                        MAX_OUTPUT_SIZE
                    );
                }
                if stderr_result.is_ok() && stderr_buf.len() >= MAX_OUTPUT_SIZE {
                    tracing::warn!(
                        asset_name = %asset.name,
                        "Stderr output truncated at {} bytes",
                        MAX_OUTPUT_SIZE
                    );
                }

                let stdout = String::from_utf8_lossy(&stdout_buf);
                let stderr = String::from_utf8_lossy(&stderr_buf);

                // Log stdout (may contain JSON result)
                if !stdout.is_empty() {
                    tracing::debug!(stdout = %stdout, "Python stdout");
                }

                // Check for failure
                if !status.success() {
                    let exit_code = status.code().unwrap_or(-1);

                    tracing::error!(
                        asset_name = %asset.name,
                        exit_code = exit_code,
                        stderr = %stderr,
                        "Asset execution failed"
                    );

                    // Try to extract error message from stderr JSON
                    let error_msg = if let Ok(error_json) =
                        serde_json::from_str::<serde_json::Value>(&stderr)
                    {
                        error_json
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or(&stderr)
                            .to_string()
                    } else {
                        stderr.to_string()
                    };

                    return Err(Error::Execution(format!(
                        "Asset '{}' failed (exit code {}):\n{}\n\n\
                         Stdout (last 500 chars):\n{}\n\n\
                         Stderr (last 500 chars):\n{}",
                        asset.name,
                        exit_code,
                        error_msg,
                        truncate_end(&stdout, 500),
                        truncate_end(&stderr, 500)
                    )));
                }

                tracing::info!(
                    asset_name = %asset.name,
                    "Asset Python execution succeeded"
                );

                Ok(())
            }
            Ok(Err(e)) => Err(Error::Execution(format!(
                "I/O error during asset '{}' execution: {}",
                asset.name, e
            ))),
            Err(_) => {
                // Timeout occurred - MUST kill the child process
                tracing::warn!(
                    asset_name = %asset.name,
                    timeout_seconds = self.config.timeout_seconds,
                    "Python execution timed out, killing process"
                );

                // Kill the process (sends SIGKILL on Unix)
                if let Err(e) = child.kill().await {
                    tracing::error!(
                        asset_name = %asset.name,
                        error = %e,
                        "Failed to kill timed-out Python process"
                    );
                }

                // Wait for it to actually terminate
                let _ = child.wait().await;

                Err(Error::Timeout(format!(
                    "Asset '{}' timed out after {} seconds.\n\n\
                     This usually means:\n\
                     1. The Python function is hanging (check for infinite loops)\n\
                     2. The function is processing too much data\n\
                     3. The timeout is too aggressive (current: {}s)\n\n\
                     Consider increasing timeout_seconds in LocalExecutorConfig.",
                    asset.name, self.config.timeout_seconds, self.config.timeout_seconds
                )))
            }
        }
    }

    /// Update execution state in the database
    async fn update_execution_state(
        &self,
        execution_id: Uuid,
        state: &str,
        error_message: Option<String>,
    ) -> Result<()> {
        let mut execution = self
            .storage
            .get_execution(execution_id, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get execution: {}", e)))?;

        execution.state = state.to_string();
        execution.updated_at = Utc::now();

        if state == states::RUNNING {
            execution.started_at = Some(Utc::now());
        } else if state == states::SUCCEEDED
            || state == states::FAILED
            || state == states::CANCELLED
        {
            execution.completed_at = Some(Utc::now());
        }

        if let Some(error) = error_message {
            execution.error_message = Some(error);
        }

        self.storage
            .update_execution(&execution, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to update execution: {}", e)))?;

        Ok(())
    }

    /// Get detailed execution status
    pub async fn get_execution_details(&self, execution_id: ExecutionId) -> Result<ExecutionModel> {
        self.storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await
            .map_err(|e| Error::Internal(format!("Failed to get execution: {}", e)))
    }
}

#[async_trait]
impl Executor for LocalExecutor {
    /// Execute a workflow locally (synchronous execution)
    ///
    /// Unlike the CloudRunExecutor, this blocks until the workflow completes.
    #[tracing::instrument(
        name = "local_executor.execute",
        skip(self),
        fields(
            workflow_id = %workflow_id.0,
            tenant_id = %self.tenant_id.as_str(),
        )
    )]
    async fn execute(&self, workflow_id: WorkflowId) -> Result<ExecutionResult> {
        self.execute_with_idempotency(workflow_id, None).await
    }

    /// Cancel a running execution
    ///
    /// Sets a cancellation flag that the executor checks between asset executions.
    /// If the execution is already past the check point, cancellation may not take
    /// immediate effect.
    #[tracing::instrument(
        name = "local_executor.cancel",
        skip(self),
        fields(
            execution_id = %execution_id.0,
            tenant_id = %self.tenant_id.as_str(),
        )
    )]
    async fn cancel(&self, execution_id: ExecutionId) -> Result<()> {
        // Mark as cancelled in memory
        self.cancelled.write().await.insert(execution_id.0);

        // Update database state
        let execution_result = self
            .storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await;

        match execution_result {
            Ok(mut execution) => {
                // Only cancel if not already in terminal state
                if execution.state != states::SUCCEEDED
                    && execution.state != states::FAILED
                    && execution.state != states::CANCELLED
                {
                    execution.state = states::CANCELLED.to_string();
                    execution.completed_at = Some(Utc::now());
                    execution.updated_at = Utc::now();

                    self.storage
                        .update_execution(&execution, &self.tenant_id)
                        .await
                        .map_err(|e| {
                            Error::Internal(format!("Failed to cancel execution: {}", e))
                        })?;

                    tracing::info!(
                        execution_id = %execution_id.0,
                        "Execution cancelled"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    execution_id = %execution_id.0,
                    error = %e,
                    "Failed to get execution for cancellation"
                );
            }
        }

        Ok(())
    }

    /// Get the status of an execution
    #[tracing::instrument(
        name = "local_executor.status",
        skip(self),
        fields(
            execution_id = %execution_id.0,
            tenant_id = %self.tenant_id.as_str(),
        )
    )]
    async fn status(&self, execution_id: ExecutionId) -> Result<ExecutionResult> {
        let execution = self
            .storage
            .get_execution(execution_id.0, &self.tenant_id)
            .await
            .map_err(|e| Error::NotFound(format!("Execution not found: {}", e)))?;

        let success = execution.state == states::SUCCEEDED;
        let error = if execution.state == states::FAILED {
            execution.error_message.clone()
        } else {
            None
        };

        Ok(ExecutionResult {
            execution_id,
            success,
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = LocalExecutorConfig::default();
        assert_eq!(config.python_path, PathBuf::from("python3"));
        assert_eq!(config.timeout_seconds, 3600);
        assert!(config.env_vars.is_empty());
    }

    #[test]
    fn test_execution_states() {
        assert_eq!(states::PENDING, "pending");
        assert_eq!(states::RUNNING, "running");
        assert_eq!(states::SUCCEEDED, "succeeded");
        assert_eq!(states::FAILED, "failed");
        assert_eq!(states::CANCELLED, "cancelled");
    }

    // =========================================================================
    // Security: Python identifier validation tests
    // =========================================================================

    #[test]
    fn test_valid_python_identifiers() {
        // Valid module names
        assert!(validate_python_identifier("mymodule", "module").is_ok());
        assert!(validate_python_identifier("my_module", "module").is_ok());
        assert!(validate_python_identifier("myproject.workflows", "module").is_ok());
        assert!(validate_python_identifier("myproject.subpkg.module", "module").is_ok());
        assert!(validate_python_identifier("_private", "module").is_ok());
        assert!(validate_python_identifier("module123", "module").is_ok());

        // Valid function names
        assert!(validate_python_identifier("run", "function").is_ok());
        assert!(validate_python_identifier("clean_data", "function").is_ok());
        assert!(validate_python_identifier("processRow2", "function").is_ok());
        assert!(validate_python_identifier("_internal_fn", "function").is_ok());
    }

    #[test]
    fn test_invalid_python_identifiers_shell_injection() {
        // Shell injection attempts - all should be rejected
        let result = validate_python_identifier("mymodule; rm -rf /", "module");
        assert!(result.is_err(), "Should reject semicolon injection");

        let result = validate_python_identifier("module | cat /etc/passwd", "module");
        assert!(result.is_err(), "Should reject pipe injection");

        let result = validate_python_identifier("module && echo evil", "module");
        assert!(result.is_err(), "Should reject && injection");

        let result = validate_python_identifier("$(whoami)", "module");
        assert!(result.is_err(), "Should reject $() injection");

        let result = validate_python_identifier("`whoami`", "module");
        assert!(result.is_err(), "Should reject backtick injection");
    }

    #[test]
    fn test_invalid_python_identifiers_path_traversal() {
        // Path traversal attempts - should be rejected
        let result = validate_python_identifier("../../etc/passwd", "module");
        assert!(result.is_err(), "Should reject path traversal");

        // Double dots in module name
        let result = validate_python_identifier("mymodule..secrets", "module");
        assert!(result.is_err(), "Should reject double dots");
        assert!(result.unwrap_err().to_string().contains("path traversal"));
    }

    #[test]
    fn test_invalid_python_identifiers_special_chars() {
        // Invalid characters
        assert!(validate_python_identifier("my-module", "module").is_err()); // hyphens
        assert!(validate_python_identifier("my module", "module").is_err()); // spaces
        assert!(validate_python_identifier("my/module", "module").is_err()); // slashes
        assert!(validate_python_identifier("my\\module", "module").is_err()); // backslashes
        assert!(validate_python_identifier("module\nfunction", "module").is_err());
        // newlines
    }

    #[test]
    fn test_invalid_python_identifiers_edge_cases() {
        // Empty string
        let result = validate_python_identifier("", "module");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));

        // Too long (>256 chars)
        let long_name = "a".repeat(257);
        let result = validate_python_identifier(&long_name, "module");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("maximum length"));

        // Starting with digit
        let result = validate_python_identifier("123module", "module");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("invalid characters"));
    }

    // =========================================================================
    // Helper function tests
    // =========================================================================

    #[test]
    fn test_truncate_end() {
        assert_eq!(truncate_end("hello", 10), "hello");
        assert_eq!(truncate_end("hello world", 5), "...world");
        assert_eq!(truncate_end("", 5), "");
        assert_eq!(truncate_end("ab", 2), "ab");
    }
}
