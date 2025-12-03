# Servo: Supplementary Design Document
## Best Practices, Extended Features, and Operational Excellence

**Document Status:** Supplementary to Project Design Document v1.0  
**Last Updated:** November 2024  
**Related Document:** [Servo Project Design Document v1.0](PROJECT_DESIGN.md)

---

## Executive Summary

This supplementary document extends the core Servo Project Design Document with additional best practices, operational patterns, and enterprise-grade features identified through industry analysis and feedback. While the primary document covers the foundational architecture, this document addresses:

- **Testing strategies** for distributed systems
- **Configuration and secrets management** patterns
- **Observability beyond basic logging**
- **Security hardening** for production environments
- **Cost optimization** and monitoring
- **High availability** and disaster recovery
- **Developer experience** enhancements
- **Extensibility patterns** for customization

These additions ensure Servo meets production-grade standards and aligns with modern data platform best practices.

---

## Table of Contents

1. [Testing and Validation Strategy](#1-testing-and-validation-strategy)
2. [Configuration and Secrets Management](#2-configuration-and-secrets-management)
3. [Advanced Observability](#3-advanced-observability)
4. [Idempotency and Retry Patterns](#4-idempotency-and-retry-patterns)
5. [Security Hardening](#5-security-hardening)
6. [Cost Monitoring and Optimization](#6-cost-monitoring-and-optimization)
7. [High Availability and Disaster Recovery](#7-high-availability-and-disaster-recovery)
8. [Extensibility and Plugin System](#8-extensibility-and-plugin-system)
9. [Developer Experience Enhancements](#9-developer-experience-enhancements)
10. [Optional Web Dashboard](#10-optional-web-dashboard)
11. [Implementation Priorities](#11-implementation-priorities)

---

## 1. Testing and Validation Strategy

### 1.1 Testing Pyramid

```
                    E2E Tests
                   ───────────
                  /           \
                 /   Contract  \
                /     Tests     \
               ─────────────────
              /                 \
             /  Integration Tests\
            /                     \
           ───────────────────────
          /                       \
         /       Unit Tests        \
        /                           \
       ─────────────────────────────
```

### 1.2 Unit Tests

**Scope:** Test individual functions and modules in isolation

**Rust Unit Tests:**

```rust
// servo-core/src/workflow.rs

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_workflow_topological_sort() {
        let workflow = Workflow {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            assets: vec![
                Asset {
                    key: "a".to_string(),
                    dependencies: vec![],
                    ..Default::default()
                },
                Asset {
                    key: "b".to_string(),
                    dependencies: vec!["a".to_string()],
                    ..Default::default()
                },
                Asset {
                    key: "c".to_string(),
                    dependencies: vec!["a".to_string(), "b".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        
        let sorted = workflow.topological_sort().unwrap();
        
        // Verify order: a must come before b and c
        let pos_a = sorted.iter().position(|x| x.key == "a").unwrap();
        let pos_b = sorted.iter().position(|x| x.key == "b").unwrap();
        let pos_c = sorted.iter().position(|x| x.key == "c").unwrap();
        
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }
    
    #[test]
    fn test_workflow_cycle_detection() {
        let workflow = Workflow {
            assets: vec![
                Asset {
                    key: "a".to_string(),
                    dependencies: vec!["b".to_string()],
                    ..Default::default()
                },
                Asset {
                    key: "b".to_string(),
                    dependencies: vec!["a".to_string()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        
        let result = workflow.topological_sort();
        assert!(matches!(result, Err(Error::CyclicDependency)));
    }
}
```

**Python Unit Tests:**

```python
# servo-python/tests/test_decorators.py

import pytest
from servo import asset, workflow
from servo.exceptions import ServoError

def test_asset_decorator_captures_dependencies():
    @asset(name="upstream")
    def upstream_asset():
        return {"data": [1, 2, 3]}
    
    @asset(name="downstream", dependencies=["upstream"])
    def downstream_asset(upstream):
        return upstream["data"]
    
    # Verify decorator attached metadata
    assert hasattr(downstream_asset, '_servo_asset')
    assert downstream_asset._servo_asset.name == "downstream"
    assert downstream_asset._servo_asset.dependencies == ["upstream"]

def test_workflow_discovery():
    @asset(name="a")
    def asset_a():
        return 1
    
    @asset(name="b")
    def asset_b():
        return 2
    
    @workflow(name="test_workflow")
    def my_workflow():
        a_val = asset_a()
        b_val = asset_b()
        return a_val + b_val
    
    # Verify workflow discovered assets
    assert hasattr(my_workflow, '_servo_workflow')
    workflow_def = my_workflow._servo_workflow
    assert len(workflow_def.assets) == 2
```

### 1.3 Integration Tests

**Scope:** Test component interactions with real dependencies (PostgreSQL, cloud services)

```rust
// servo-core/tests/integration/workflow_execution.rs

use servo_core::*;
use servo_storage::*;
use servo_runtime::*;
use testcontainers::*;

#[tokio::test]
async fn test_end_to_end_workflow_execution() {
    // Start test PostgreSQL container
    let docker = clients::Cli::default();
    let postgres = docker.run(images::postgres::Postgres::default());
    let connection_string = format!(
        "postgres://postgres:postgres@localhost:{}/postgres",
        postgres.get_host_port_ipv4(5432)
    );
    
    // Initialize storage
    let pool = PgPoolOptions::new()
        .connect(&connection_string)
        .await
        .unwrap();
    
    // Run migrations
    sqlx::migrate!("../servo-storage/migrations")
        .run(&pool)
        .await
        .unwrap();
    
    let storage = PostgresMetadataStore::new(pool.clone());
    
    // Create test workflow
    let workflow = Workflow {
        id: Uuid::new_v4(),
        name: "test_workflow".to_string(),
        assets: vec![
            Asset {
                key: "extract".to_string(),
                dependencies: vec![],
                compute_fn: ComputeFn::PythonCallable {
                    module: "test_module".to_string(),
                    function: "extract_data".to_string(),
                    args: vec![],
                    kwargs: HashMap::new(),
                },
                ..Default::default()
            },
            Asset {
                key: "transform".to_string(),
                dependencies: vec!["extract".to_string()],
                compute_fn: ComputeFn::PythonCallable {
                    module: "test_module".to_string(),
                    function: "transform_data".to_string(),
                    args: vec![],
                    kwargs: HashMap::new(),
                },
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    
    // Store workflow
    storage.store_workflow(&workflow).await.unwrap();
    
    // Create execution plan
    let plan = workflow.create_execution_plan().unwrap();
    assert_eq!(plan.levels.len(), 2); // Two sequential levels
    
    // Simulate execution
    let execution = Execution {
        execution_id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Pending,
        ..Default::default()
    };
    
    storage.record_execution(&execution).await.unwrap();
    
    // Verify execution was stored
    let retrieved = storage.get_execution(&execution.execution_id).await.unwrap();
    assert_eq!(retrieved.execution_id, execution.execution_id);
    assert_eq!(retrieved.status, ExecutionStatus::Pending);
}
```

### 1.4 Contract Tests

**Scope:** Verify cloud provider API contracts don't break

```rust
// servo-cloud-gcp/tests/contract/cloud_tasks.rs

#[tokio::test]
#[ignore] // Run only in CI with real GCP credentials
async fn test_cloud_tasks_contract() {
    let executor = CloudRunExecutor::new(
        std::env::var("GCP_PROJECT").unwrap(),
        "us-central1".to_string()
    ).await.unwrap();
    
    let task = Task {
        id: TaskId::new(),
        execution_id: Uuid::new_v4(),
        tenant_id: None,
        asset: create_test_asset(),
        inputs: vec![],
        schedule_time: Utc::now(),
        retry_count: 0,
        max_retries: 3,
        timeout: Duration::from_secs(300),
    };
    
    // Verify we can enqueue
    let task_id = executor.enqueue_task(task.clone()).await.unwrap();
    assert_eq!(task_id, task.id);
    
    // Verify we can check status
    let status = executor.get_task_status(&task_id).await.unwrap();
    assert!(matches!(status, TaskStatus::Pending));
    
    // Verify we can cancel
    executor.cancel_task(&task_id).await.unwrap();
}
```

### 1.5 End-to-End Tests

**Scope:** Full workflow execution from definition to completion

```python
# examples/simple-etl/tests/test_e2e.py

import pytest
import os
from servo import asset, workflow, executor
from servo.client import ServoClient

@pytest.fixture
def servo_client():
    # Assumes test environment is set up
    return ServoClient(
        api_url=os.getenv("SERVO_API_URL"),
        database_url=os.getenv("DATABASE_URL")
    )

def test_simple_etl_e2e(servo_client):
    """Test complete ETL workflow execution."""
    
    @asset(name="extract_customers")
    def extract():
        return [
            {"id": 1, "name": "Alice", "email": "alice@example.com"},
            {"id": 2, "name": "Bob", "email": "bob@example.com"},
        ]
    
    @asset(name="transform_customers", dependencies=["extract_customers"])
    def transform(raw_data):
        return [
            {**customer, "email_domain": customer["email"].split("@")[1]}
            for customer in raw_data
        ]
    
    @workflow(
        name="test_etl",
        executor=executor.CloudRun(
            project=os.getenv("GCP_PROJECT"),
            region="us-central1"
        )
    )
    def etl_pipeline():
        raw = extract()
        transformed = transform(raw)
        return transformed
    
    # Deploy workflow
    workflow_id = servo_client.deploy_workflow(etl_pipeline)
    
    # Trigger execution
    execution = servo_client.trigger_workflow(workflow_id)
    
    # Wait for completion (with timeout)
    result = execution.wait(timeout=300)
    
    # Verify success
    assert result.status == "succeeded"
    
    # Verify lineage was recorded
    lineage = servo_client.get_asset_lineage("transform_customers")
    assert "extract_customers" in [a.key for a in lineage.upstream]
    
    # Verify metrics were recorded
    metrics = servo_client.get_execution_metrics(execution.id)
    assert metrics.total_tasks == 2
    assert metrics.succeeded_tasks == 2
```

### 1.6 Chaos Engineering Tests

**Scope:** Test system resilience under failure conditions

```rust
// servo-runtime/tests/chaos/network_failures.rs

#[tokio::test]
async fn test_retry_on_transient_network_failure() {
    let mut mock_executor = MockExecutor::new();
    
    // Simulate 2 failures then success
    let mut call_count = 0;
    mock_executor
        .expect_enqueue_task()
        .times(3)
        .returning(move |_| {
            call_count += 1;
            if call_count <= 2 {
                Err(Error::NetworkTimeout)
            } else {
                Ok(TaskId::new())
            }
        });
    
    let runtime = Runtime::new(mock_executor);
    let task = create_test_task();
    
    // Should succeed after retries
    let result = runtime
        .execute_with_retry(&task, RetryPolicy::default())
        .await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_execution_continues_after_worker_crash() {
    // Simulate worker crash mid-execution
    // Verify task is re-enqueued and completes
    // This tests the "at-least-once" execution guarantee
}
```

### 1.7 Performance Tests

**Scope:** Benchmark critical paths

```rust
// benches/lineage_queries.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use servo_lineage::LineageGraph;

fn bench_upstream_query(c: &mut Criterion) {
    let graph = create_large_lineage_graph(1000); // 1000 assets
    
    c.bench_function("upstream_query_depth_5", |b| {
        b.iter(|| {
            graph.upstream_assets(
                black_box("asset_500"),
                black_box(5)
            )
        })
    });
}

fn bench_downstream_query(c: &mut Criterion) {
    let graph = create_large_lineage_graph(1000);
    
    c.bench_function("downstream_query_full", |b| {
        b.iter(|| {
            graph.downstream_assets(black_box("asset_1"))
        })
    });
}

criterion_group!(benches, bench_upstream_query, bench_downstream_query);
criterion_main!(benches);
```

### 1.8 Testing Infrastructure

**Directory Structure:**

```
servo/
├── servo-tests/                    # Shared test utilities
│   ├── src/
│   │   ├── lib.rs
│   │   ├── fixtures.rs            # Test data fixtures
│   │   ├── mocks.rs               # Mock implementations
│   │   ├── containers.rs          # Testcontainers setup
│   │   └── assertions.rs          # Custom assertions
│   └── Cargo.toml
├── .github/
│   └── workflows/
│       ├── unit-tests.yml
│       ├── integration-tests.yml
│       ├── e2e-tests.yml
│       └── performance-tests.yml
└── scripts/
    ├── run-unit-tests.sh
    ├── run-integration-tests.sh
    └── run-chaos-tests.sh
```

**Shared Test Utilities:**

```rust
// servo-tests/src/fixtures.rs

use servo_core::*;
use uuid::Uuid;

pub fn create_test_workflow(name: &str, num_assets: usize) -> Workflow {
    let mut assets = Vec::new();
    
    for i in 0..num_assets {
        assets.push(Asset {
            id: Uuid::new_v4(),
            key: format!("asset_{}", i),
            dependencies: if i > 0 {
                vec![format!("asset_{}", i - 1)]
            } else {
                vec![]
            },
            compute_fn: ComputeFn::PythonCallable {
                module: "test".to_string(),
                function: format!("fn_{}", i),
                args: vec![],
                kwargs: HashMap::new(),
            },
            ..Default::default()
        });
    }
    
    Workflow {
        id: Uuid::new_v4(),
        name: name.to_string(),
        assets,
        ..Default::default()
    }
}

pub fn create_test_asset() -> Asset {
    Asset {
        id: Uuid::new_v4(),
        key: "test_asset".to_string(),
        dependencies: vec![],
        compute_fn: ComputeFn::PythonCallable {
            module: "test".to_string(),
            function: "test_fn".to_string(),
            args: vec![],
            kwargs: HashMap::new(),
        },
        ..Default::default()
    }
}
```

---

## 2. Configuration and Secrets Management

### 2.1 Configuration Architecture

**Hierarchical Configuration:**

```
Environment Variables (highest priority)
    ↓
Configuration File (.servo/config.yaml)
    ↓
Defaults (lowest priority)
```

### 2.2 Configuration Schema

```yaml
# .servo/config.yaml

# Database configuration
database:
  url: ${DATABASE_URL}  # Environment variable substitution
  pool_size: 10
  connection_timeout: 30s
  ssl_mode: require

# Cloud provider configuration
cloud:
  provider: gcp  # gcp | aws | azure
  
  gcp:
    project: ${GCP_PROJECT}
    region: us-central1
    
    cloud_tasks:
      queue_name: servo-default
      max_dispatches_per_second: 500
    
    cloud_run:
      service_name: servo-worker
      max_instances: 100
      cpu: "2"
      memory: "4Gi"
    
    cloudsql:
      instance: servo-db
      database: servo_metadata
  
  aws:
    region: us-east-1
    
    sqs:
      queue_name: servo-default
    
    lambda:
      function_name: servo-worker
      memory: 2048
      timeout: 900

# Storage configuration
storage:
  artifacts_bucket: ${ARTIFACTS_BUCKET}
  logs_bucket: ${LOGS_BUCKET}
  retention_days: 90

# Observability
observability:
  logging:
    level: info  # trace | debug | info | warn | error
    format: json
    destination: stdout
  
  tracing:
    enabled: true
    exporter: opentelemetry
    endpoint: ${OTEL_COLLECTOR_ENDPOINT}
    sample_rate: 0.1  # 10% sampling
  
  metrics:
    enabled: true
    prometheus_port: 9090

# Security
security:
  secrets_backend: google_secret_manager  # google_secret_manager | aws_secrets_manager | vault
  
  encryption:
    at_rest: true
    in_transit: true
  
  audit_logging:
    enabled: true
    destination: ${AUDIT_LOG_BUCKET}

# Multi-tenancy
multi_tenant:
  enabled: true
  isolation_level: strict  # strict | relaxed
  
  quotas:
    default:
      max_executions_per_hour: 1000
      max_concurrent_executions: 100
      max_storage_gb: 100

# Cost tracking
cost_tracking:
  enabled: true
  provider_billing_api: true
  alert_threshold_usd: 1000
```

### 2.3 Configuration Loading (Rust)

```rust
// servo-core/src/config.rs

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServoConfig {
    pub database: DatabaseConfig,
    pub cloud: CloudConfig,
    pub storage: StorageConfig,
    pub observability: ObservabilityConfig,
    pub security: SecurityConfig,
    pub multi_tenant: MultiTenantConfig,
    pub cost_tracking: CostTrackingConfig,
}

impl ServoConfig {
    /// Load configuration with hierarchical override:
    /// 1. Default values
    /// 2. Config file (.servo/config.yaml)
    /// 3. Environment variables
    pub fn load() -> Result<Self> {
        // Start with defaults
        let mut config = Self::default();
        
        // Load from config file if exists
        if let Some(config_path) = Self::find_config_file() {
            let file_content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
            
            // Parse YAML with environment variable substitution
            let file_config: ServoConfig = serde_yaml::from_str(&file_content)
                .context("Failed to parse config file")?;
            
            config = config.merge(file_config);
        }
        
        // Override with environment variables
        config.apply_env_overrides()?;
        
        // Validate configuration
        config.validate()?;
        
        Ok(config)
    }
    
    fn find_config_file() -> Option<PathBuf> {
        // Search in order:
        // 1. Current directory: .servo/config.yaml
        // 2. Home directory: ~/.servo/config.yaml
        // 3. System: /etc/servo/config.yaml
        
        let candidates = vec![
            PathBuf::from(".servo/config.yaml"),
            dirs::home_dir()?.join(".servo/config.yaml"),
            PathBuf::from("/etc/servo/config.yaml"),
        ];
        
        candidates.into_iter().find(|p| p.exists())
    }
    
    fn apply_env_overrides(&mut self) -> Result<()> {
        // Override specific fields from environment variables
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            self.database.url = db_url;
        }
        
        if let Ok(project) = std::env::var("GCP_PROJECT") {
            if let CloudProvider::Gcp(ref mut gcp) = self.cloud.provider {
                gcp.project = project;
            }
        }
        
        // ... more overrides
        
        Ok(())
    }
    
    fn validate(&self) -> Result<()> {
        // Validate required fields
        if self.database.url.is_empty() {
            anyhow::bail!("DATABASE_URL is required");
        }
        
        // Validate sensible values
        if self.database.pool_size == 0 {
            anyhow::bail!("Database pool size must be > 0");
        }
        
        Ok(())
    }
    
    fn merge(mut self, other: ServoConfig) -> Self {
        // Merge configurations (other takes precedence for non-default values)
        // This is a simplified version; real implementation would be more thorough
        self
    }
}

impl Default for ServoConfig {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            cloud: CloudConfig::default(),
            storage: StorageConfig::default(),
            observability: ObservabilityConfig::default(),
            security: SecurityConfig::default(),
            multi_tenant: MultiTenantConfig::default(),
            cost_tracking: CostTrackingConfig::default(),
        }
    }
}
```

### 2.4 Secrets Management

**Secrets Interface:**

```rust
// servo-core/src/secrets.rs

#[async_trait]
pub trait SecretsBackend: Send + Sync {
    async fn get_secret(&self, name: &str) -> Result<String>;
    async fn set_secret(&self, name: &str, value: &str) -> Result<()>;
    async fn delete_secret(&self, name: &str) -> Result<()>;
    async fn list_secrets(&self) -> Result<Vec<String>>;
}

// Google Secret Manager implementation
pub struct GoogleSecretManager {
    client: SecretManagerServiceClient,
    project: String,
}

#[async_trait]
impl SecretsBackend for GoogleSecretManager {
    async fn get_secret(&self, name: &str) -> Result<String> {
        let secret_name = format!(
            "projects/{}/secrets/{}/versions/latest",
            self.project, name
        );
        
        let response = self.client
            .access_secret_version(AccessSecretVersionRequest {
                name: secret_name,
            })
            .await?;
        
        let payload = response
            .into_inner()
            .payload
            .ok_or_else(|| anyhow::anyhow!("Secret has no payload"))?;
        
        Ok(String::from_utf8(payload.data)?)
    }
    
    async fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        // Create or update secret
        let parent = format!("projects/{}", self.project);
        let secret_id = name.to_string();
        
        // First, ensure secret exists
        match self.client
            .create_secret(CreateSecretRequest {
                parent: parent.clone(),
                secret_id: secret_id.clone(),
                secret: Some(Secret {
                    replication: Some(Replication {
                        replication: Some(replication::Replication::Automatic(
                            Automatic {}
                        )),
                    }),
                    ..Default::default()
                }),
            })
            .await
        {
            Ok(_) => {},
            Err(e) if e.code() == Code::AlreadyExists => {},
            Err(e) => return Err(e.into()),
        }
        
        // Add new version
        let secret_name = format!("projects/{}/secrets/{}", self.project, name);
        self.client
            .add_secret_version(AddSecretVersionRequest {
                parent: secret_name,
                payload: Some(SecretPayload {
                    data: value.as_bytes().to_vec(),
                }),
            })
            .await?;
        
        Ok(())
    }
}

// AWS Secrets Manager implementation
pub struct AwsSecretsManager {
    client: aws_sdk_secretsmanager::Client,
}

#[async_trait]
impl SecretsBackend for AwsSecretsManager {
    async fn get_secret(&self, name: &str) -> Result<String> {
        let response = self.client
            .get_secret_value()
            .secret_id(name)
            .send()
            .await?;
        
        Ok(response.secret_string().unwrap_or_default().to_string())
    }
    
    async fn set_secret(&self, name: &str, value: &str) -> Result<()> {
        self.client
            .put_secret_value()
            .secret_id(name)
            .secret_string(value)
            .send()
            .await?;
        
        Ok(())
    }
}

// Factory for creating secrets backend
pub fn create_secrets_backend(config: &SecurityConfig) -> Result<Box<dyn SecretsBackend>> {
    match config.secrets_backend.as_str() {
        "google_secret_manager" => {
            Ok(Box::new(GoogleSecretManager::new(
                std::env::var("GCP_PROJECT")?
            )))
        },
        "aws_secrets_manager" => {
            Ok(Box::new(AwsSecretsManager::new().await?))
        },
        "vault" => {
            Ok(Box::new(VaultSecretsBackend::new(
                std::env::var("VAULT_ADDR")?,
                std::env::var("VAULT_TOKEN")?
            )))
        },
        _ => Err(anyhow::anyhow!("Unknown secrets backend: {}", config.secrets_backend)),
    }
}
```

**Usage in Worker:**

```rust
// servo-worker/src/main.rs

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = ServoConfig::load()?;
    
    // Initialize secrets backend
    let secrets = create_secrets_backend(&config.security)?;
    
    // Get database password from secrets
    let db_password = secrets.get_secret("servo-db-password").await?;
    
    // Build database URL with secret
    let database_url = format!(
        "postgresql://servo:{}@{}:5432/servo_metadata",
        db_password,
        config.database.host
    );
    
    // Continue with initialization...
}
```

### 2.5 Environment-Specific Configurations

**Development:**

```yaml
# .servo/config.dev.yaml
database:
  url: postgresql://servo:servo@localhost:5432/servo_dev
  pool_size: 5

cloud:
  provider: local  # Use local executor for development

observability:
  logging:
    level: debug
  tracing:
    sample_rate: 1.0  # 100% sampling in dev

security:
  secrets_backend: env  # Read from environment variables directly
```

**Staging:**

```yaml
# .servo/config.staging.yaml
database:
  url: ${DATABASE_URL}
  pool_size: 10

cloud:
  provider: gcp
  gcp:
    project: servo-staging
    region: us-central1

observability:
  logging:
    level: info
  tracing:
    sample_rate: 0.5  # 50% sampling in staging
```

**Production:**

```yaml
# .servo/config.prod.yaml
database:
  url: ${DATABASE_URL}
  pool_size: 20
  ssl_mode: require

cloud:
  provider: gcp
  gcp:
    project: servo-production
    region: us-central1
    
    cloud_run:
      max_instances: 1000

observability:
  logging:
    level: info
  tracing:
    sample_rate: 0.1  # 10% sampling in prod

security:
  secrets_backend: google_secret_manager
  encryption:
    at_rest: true
    in_transit: true
  audit_logging:
    enabled: true

multi_tenant:
  quotas:
    default:
      max_executions_per_hour: 5000
```

---

## 3. Advanced Observability

### 3.1 Structured Logging

**Logging Standards:**

```rust
// Use tracing for structured logging throughout

use tracing::{info, warn, error, debug, trace, instrument};

#[instrument(
    skip(self, task),
    fields(
        execution_id = %task.execution_id,
        tenant_id = ?task.tenant_id,
        asset_key = %task.asset.key,
    )
)]
async fn execute_task(&self, task: Task) -> Result<TaskResult> {
    info!("Starting task execution");
    
    let start = Instant::now();
    
    match self.run_task_logic(&task).await {
        Ok(result) => {
            let duration = start.elapsed();
            info!(
                duration_ms = duration.as_millis(),
                output_location = %result.location,
                row_count = result.row_count,
                "Task completed successfully"
            );
            Ok(result)
        }
        Err(e) => {
            let duration = start.elapsed();
            error!(
                duration_ms = duration.as_millis(),
                error = %e,
                "Task execution failed"
            );
            Err(e)
        }
    }
}
```

**Log Structure Example:**

```json
{
  "timestamp": "2024-01-01T02:15:30.123Z",
  "level": "INFO",
  "message": "Task completed successfully",
  "fields": {
    "execution_id": "550e8400-e29b-41d4-a716-446655440000",
    "tenant_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
    "asset_key": "clean_customers",
    "duration_ms": 12450,
    "output_location": "gs://servo-data/tenant-123/clean_customers/2024-01-01.parquet",
    "row_count": 15234
  },
  "span": {
    "name": "execute_task",
    "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
    "span_id": "00f067aa0ba902b7"
  }
}
```

### 3.2 Distributed Tracing

**OpenTelemetry Integration:**

```rust
// servo-runtime/src/observability.rs

use opentelemetry::{
    global,
    trace::{Tracer, TraceContextExt},
    Context, KeyValue,
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub fn init_tracing(config: &ObservabilityConfig) -> Result<()> {
    // Initialize OpenTelemetry exporter
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(&config.tracing.endpoint);
    
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(exporter)
        .with_trace_config(
            opentelemetry::sdk::trace::config()
                .with_sampler(opentelemetry::sdk::trace::Sampler::TraceIdRatioBased(
                    config.tracing.sample_rate
                ))
                .with_resource(opentelemetry::sdk::Resource::new(vec![
                    KeyValue::new("service.name", "servo-worker"),
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                ]))
        )
        .install_batch(opentelemetry::runtime::Tokio)?;
    
    // Set up tracing subscriber
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    
    tracing_subscriber::registry()
        .with(telemetry)
        .with(tracing_subscriber::fmt::layer().json())
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    
    Ok(())
}

// Example traced function
#[tracing::instrument(skip(self))]
async fn execute_workflow(&self, workflow: &Workflow) -> Result<ExecutionId> {
    let span = tracing::Span::current();
    span.set_attribute(KeyValue::new("workflow.name", workflow.name.clone()));
    span.set_attribute(KeyValue::new("workflow.asset_count", workflow.assets.len() as i64));
    
    // Create execution plan
    let plan = workflow.create_execution_plan()?;
    
    // Execute levels sequentially
    for (level_idx, level) in plan.levels.iter().enumerate() {
        let level_span = tracing::info_span!(
            "execute_level",
            level = level_idx,
            tasks = level.len()
        );
        
        async move {
            // Execute tasks in parallel within level
            let futures: Vec<_> = level.iter()
                .map(|asset| self.execute_asset(asset))
                .collect();
            
            futures::future::try_join_all(futures).await?;
            Ok::<_, Error>(())
        }
        .instrument(level_span)
        .await?;
    }
    
    Ok(ExecutionId::new())
}
```

**Trace Propagation Across Services:**

```rust
// Worker receives task with trace context
async fn handle_task_request(
    req: Request<Body>,
    runtime: Runtime,
) -> Result<Response<Body>> {
    // Extract trace context from HTTP headers
    let parent_cx = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor::new(req.headers()))
    });
    
    // Attach parent context to current span
    let span = tracing::info_span!("handle_task");
    span.set_parent(parent_cx);
    
    async move {
        let task: Task = serde_json::from_slice(&hyper::body::to_bytes(req).await?)?;
        
        let result = runtime.execute_task(task).await?;
        
        Ok(Response::new(Body::from(serde_json::to_string(&result)?)))
    }
    .instrument(span)
    .await
}
```

### 3.3 Metrics Collection

**Prometheus Metrics:**

```rust
// servo-runtime/src/metrics.rs

use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    HistogramVec, IntCounterVec, IntGaugeVec,
};
use lazy_static::lazy_static;

lazy_static! {
    // Task execution metrics
    pub static ref TASK_EXECUTION_DURATION: HistogramVec = register_histogram_vec!(
        "servo_task_execution_duration_seconds",
        "Task execution duration in seconds",
        &["tenant_id", "workflow_name", "asset_key", "status"],
        vec![0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0]
    ).unwrap();
    
    pub static ref TASK_EXECUTION_TOTAL: IntCounterVec = register_int_counter_vec!(
        "servo_task_execution_total",
        "Total number of task executions",
        &["tenant_id", "workflow_name", "status"]
    ).unwrap();
    
    pub static ref ACTIVE_EXECUTIONS: IntGaugeVec = register_int_gauge_vec!(
        "servo_active_executions",
        "Number of currently active executions",
        &["tenant_id"]
    ).unwrap();
    
    // Queue metrics
    pub static ref QUEUE_DEPTH: IntGaugeVec = register_int_gauge_vec!(
        "servo_queue_depth",
        "Number of tasks in queue",
        &["queue_name"]
    ).unwrap();
    
    pub static ref QUEUE_ENQUEUE_DURATION: HistogramVec = register_histogram_vec!(
        "servo_queue_enqueue_duration_seconds",
        "Time to enqueue a task",
        &["queue_name"],
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]
    ).unwrap();
    
    // Lineage metrics
    pub static ref LINEAGE_QUERY_DURATION: HistogramVec = register_histogram_vec!(
        "servo_lineage_query_duration_seconds",
        "Lineage query duration",
        &["query_type", "depth"],
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5]
    ).unwrap();
    
    // Cost metrics
    pub static ref EXECUTION_COST_USD: HistogramVec = register_histogram_vec!(
        "servo_execution_cost_usd",
        "Estimated execution cost in USD",
        &["tenant_id", "workflow_name"],
        vec![0.01, 0.1, 1.0, 10.0, 100.0]
    ).unwrap();
}

// Usage in code
impl Runtime {
    async fn execute_task(&self, task: Task) -> Result<TaskResult> {
        let timer = TASK_EXECUTION_DURATION
            .with_label_values(&[
                task.tenant_id.as_ref().map(|id| id.to_string()).as_deref().unwrap_or("none"),
                &task.workflow_name,
                &task.asset.key,
                "running",
            ])
            .start_timer();
        
        ACTIVE_EXECUTIONS
            .with_label_values(&[
                task.tenant_id.as_ref().map(|id| id.to_string()).as_deref().unwrap_or("none")
            ])
            .inc();
        
        let result = self.run_task_logic(&task).await;
        
        let status = if result.is_ok() { "succeeded" } else { "failed" };
        
        timer.observe_duration();
        
        TASK_EXECUTION_TOTAL
            .with_label_values(&[
                task.tenant_id.as_ref().map(|id| id.to_string()).as_deref().unwrap_or("none"),
                &task.workflow_name,
                status,
            ])
            .inc();
        
        ACTIVE_EXECUTIONS
            .with_label_values(&[
                task.tenant_id.as_ref().map(|id| id.to_string()).as_deref().unwrap_or("none")
            ])
            .dec();
        
        result
    }
}
```

**Prometheus Endpoint:**

```rust
// servo-worker/src/main.rs

use actix_web::{web, App, HttpResponse, HttpServer};
use prometheus::{Encoder, TextEncoder};

async fn metrics_handler() -> HttpResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(buffer)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // ... initialization
    
    HttpServer::new(|| {
        App::new()
            .route("/execute", web::post().to(execute_task_handler))
            .route("/health", web::get().to(health_check_handler))
            .route("/metrics", web::get().to(metrics_handler))  // Prometheus scrape endpoint
    })
    .bind(("0.0.0.0", 8080))?
    .run()
    .await
}
```

### 3.4 Alerting

**Alert Definitions:**

```yaml
# alerts/servo-alerts.yaml

groups:
  - name: servo_execution_alerts
    interval: 30s
    rules:
      - alert: HighTaskFailureRate
        expr: |
          (
            sum(rate(servo_task_execution_total{status="failed"}[5m])) by (tenant_id, workflow_name)
            /
            sum(rate(servo_task_execution_total[5m])) by (tenant_id, workflow_name)
          ) > 0.1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High task failure rate for {{ $labels.workflow_name }}"
          description: "{{ $labels.workflow_name }} has a failure rate of {{ $value | humanizePercentage }} over the last 5 minutes"
      
      - alert: ExecutionStuck
        expr: |
          servo_active_executions > 0
          and
          changes(servo_active_executions[10m]) == 0
        for: 10m
        labels:
          severity: critical
        annotations:
          summary: "Execution appears stuck for tenant {{ $labels.tenant_id }}"
          description: "Active execution count has not changed in 10 minutes"
      
      - alert: QueueBacklog
        expr: servo_queue_depth > 1000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Queue {{ $labels.queue_name }} has large backlog"
          description: "{{ $labels.queue_name }} has {{ $value }} tasks pending"
      
      - alert: HighExecutionCost
        expr: |
          sum(rate(servo_execution_cost_usd[1h])) by (tenant_id) > 100
        for: 1h
        labels:
          severity: info
        annotations:
          summary: "High execution costs for tenant {{ $labels.tenant_id }}"
          description: "Tenant {{ $labels.tenant_id }} has spent ${{ $value }} in the last hour"
```

---

## 4. Idempotency and Retry Patterns

### 4.1 Idempotency Keys

**Execution-Level Idempotency:**

```rust
// servo-storage/src/postgres.rs

impl PostgresMetadataStore {
    pub async fn create_execution_idempotent(
        &self,
        execution: &Execution,
        idempotency_key: &str,
    ) -> Result<Execution> {
        // Use PostgreSQL's ON CONFLICT to ensure exactly-once execution creation
        let result = sqlx::query_as!(
            Execution,
            r#"
            INSERT INTO executions (
                execution_id, workflow_id, tenant_id, status,
                idempotency_key, created_at
            )
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (idempotency_key) DO NOTHING
            RETURNING *
            "#,
            execution.execution_id,
            execution.workflow_id,
            execution.tenant_id,
            execution.status.to_string(),
            idempotency_key
        )
        .fetch_optional(&self.pool)
        .await?;
        
        match result {
            Some(exec) => {
                // New execution created
                Ok(exec)
            }
            None => {
                // Idempotency key collision - return existing execution
                let existing = sqlx::query_as!(
                    Execution,
                    r#"
                    SELECT * FROM executions
                    WHERE idempotency_key = $1
                    "#,
                    idempotency_key
                )
                .fetch_one(&self.pool)
                .await?;
                
                Ok(existing)
            }
        }
    }
}
```

**Task-Level Idempotency:**

```rust
// servo-runtime/src/idempotency.rs

pub struct IdempotentTaskExecutor {
    storage: Arc<dyn MetadataStore>,
    executor: Arc<dyn Executor>,
}

impl IdempotentTaskExecutor {
    pub async fn execute_task_idempotent(&self, task: Task) -> Result<TaskResult> {
        // Generate idempotency key from task content
        let idempotency_key = self.generate_idempotency_key(&task);
        
        // Check if task already completed
        if let Some(result) = self.storage.get_task_result(&idempotency_key).await? {
            tracing::info!(
                task_id = %task.id,
                idempotency_key = %idempotency_key,
                "Task already completed, returning cached result"
            );
            return Ok(result);
        }
        
        // Execute task
        let result = self.executor.execute_task(task.clone()).await?;
        
        // Store result with idempotency key
        self.storage.store_task_result(&idempotency_key, &result).await?;
        
        Ok(result)
    }
    
    fn generate_idempotency_key(&self, task: &Task) -> String {
        use sha2::{Sha256, Digest};
        
        // Hash: execution_id + asset_key + inputs_hash
        let mut hasher = Sha256::new();
        hasher.update(task.execution_id.as_bytes());
        hasher.update(task.asset.key.as_bytes());
        
        // Hash input locations
        for input in &task.inputs {
            hasher.update(input.location.as_bytes());
        }
        
        format!("{:x}", hasher.finalize())
    }
}
```

### 4.2 Retry Policies

**Comprehensive Retry Configuration:**

```rust
// servo-core/src/retry.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_attempts: u32,
    
    /// Initial delay before first retry
    pub initial_delay: Duration,
    
    /// Maximum delay between retries
    pub max_delay: Duration,
    
    /// Backoff multiplier (e.g., 2.0 for exponential backoff)
    pub backoff_multiplier: f64,
    
    /// Jitter factor (0.0-1.0) to randomize delays
    pub jitter_factor: f64,
    
    /// List of error types that should trigger retry
    pub retry_on: Vec<ErrorType>,
    
    /// List of error types that should NOT trigger retry
    pub dont_retry_on: Vec<ErrorType>,
    
    /// Whether to retry on timeout
    pub retry_on_timeout: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ErrorType {
    NetworkError,
    TimeoutError,
    RateLimitError,
    ServiceUnavailable,
    InternalServerError,
    
    // Non-retryable errors
    AuthenticationError,
    AuthorizationError,
    ValidationError,
    NotFoundError,
}

impl RetryPolicy {
    pub fn exponential_backoff() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(300),
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
            retry_on: vec![
                ErrorType::NetworkError,
                ErrorType::TimeoutError,
                ErrorType::RateLimitError,
                ErrorType::ServiceUnavailable,
                ErrorType::InternalServerError,
            ],
            dont_retry_on: vec![
                ErrorType::AuthenticationError,
                ErrorType::AuthorizationError,
                ErrorType::ValidationError,
                ErrorType::NotFoundError,
            ],
            retry_on_timeout: true,
        }
    }
    
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        use rand::Rng;
        
        // Calculate base delay with exponential backoff
        let base_delay_secs = self.initial_delay.as_secs_f64()
            * self.backoff_multiplier.powi(attempt as i32);
        
        // Cap at max delay
        let capped_delay_secs = base_delay_secs.min(self.max_delay.as_secs_f64());
        
        // Add jitter
        let mut rng = rand::thread_rng();
        let jitter = rng.gen_range(-self.jitter_factor..=self.jitter_factor);
        let final_delay_secs = capped_delay_secs * (1.0 + jitter);
        
        Duration::from_secs_f64(final_delay_secs.max(0.0))
    }
    
    pub fn should_retry(&self, error: &Error, attempt: u32) -> bool {
        // Check if we've exceeded max attempts
        if attempt >= self.max_attempts {
            return false;
        }
        
        // Check if error type is in "don't retry" list
        let error_type = error.classify();
        if self.dont_retry_on.contains(&error_type) {
            return false;
        }
        
        // Check if error type is in "retry" list
        self.retry_on.contains(&error_type)
    }
}

// Retry executor
pub struct RetryExecutor<E: Executor> {
    inner: E,
}

impl<E: Executor> RetryExecutor<E> {
    pub async fn execute_with_retry(
        &self,
        task: Task,
        policy: &RetryPolicy,
    ) -> Result<TaskResult> {
        let mut attempt = 0;
        
        loop {
            match self.inner.execute_task(task.clone()).await {
                Ok(result) => {
                    if attempt > 0 {
                        tracing::info!(
                            task_id = %task.id,
                            attempts = attempt + 1,
                            "Task succeeded after retries"
                        );
                    }
                    return Ok(result);
                }
                Err(e) => {
                    attempt += 1;
                    
                    if !policy.should_retry(&e, attempt) {
                        tracing::error!(
                            task_id = %task.id,
                            attempts = attempt,
                            error = %e,
                            "Task failed, not retrying"
                        );
                        return Err(e);
                    }
                    
                    let delay = policy.calculate_delay(attempt);
                    
                    tracing::warn!(
                        task_id = %task.id,
                        attempt = attempt,
                        error = %e,
                        retry_after_secs = delay.as_secs(),
                        "Task failed, will retry"
                    );
                    
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}
```

### 4.3 Circuit Breaker Pattern

```rust
// servo-runtime/src/circuit_breaker.rs

use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,    // Normal operation
    Open,      // Failing, reject requests
    HalfOpen,  // Testing if service recovered
}

pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitBreakerState>>,
    config: CircuitBreakerConfig,
}

#[derive(Debug)]
struct CircuitBreakerState {
    current_state: CircuitState,
    failure_count: u32,
    last_failure_time: Option<Instant>,
    last_success_time: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: u32,
    
    /// Duration to wait before attempting half-open
    pub timeout: Duration,
    
    /// Number of successful requests to close circuit from half-open
    pub success_threshold: u32,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitBreakerState {
                current_state: CircuitState::Closed,
                failure_count: 0,
                last_failure_time: None,
                last_success_time: None,
            })),
            config,
        }
    }
    
    pub async fn call<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        // Check if circuit is open
        {
            let mut state = self.state.write().await;
            
            match state.current_state {
                CircuitState::Open => {
                    // Check if timeout has passed
                    if let Some(last_failure) = state.last_failure_time {
                        if last_failure.elapsed() >= self.config.timeout {
                            // Transition to half-open
                            state.current_state = CircuitState::HalfOpen;
                            tracing::info!("Circuit breaker transitioning to half-open");
                        } else {
                            return Err(Error::CircuitBreakerOpen);
                        }
                    }
                }
                _ => {}
            }
        }
        
        // Execute function
        let result = f().await;
        
        // Update state based on result
        {
            let mut state = self.state.write().await;
            
            match result {
                Ok(_) => {
                    state.last_success_time = Some(Instant::now());
                    
                    match state.current_state {
                        CircuitState::HalfOpen => {
                            // Successful request in half-open, close circuit
                            state.current_state = CircuitState::Closed;
                            state.failure_count = 0;
                            tracing::info!("Circuit breaker closed after successful test");
                        }
                        CircuitState::Closed => {
                            // Reset failure count on success
                            state.failure_count = 0;
                        }
                        _ => {}
                    }
                }
                Err(_) => {
                    state.last_failure_time = Some(Instant::now());
                    state.failure_count += 1;
                    
                    match state.current_state {
                        CircuitState::HalfOpen => {
                            // Failed in half-open, re-open circuit
                            state.current_state = CircuitState::Open;
                            tracing::warn!("Circuit breaker re-opened after failed test");
                        }
                        CircuitState::Closed => {
                            if state.failure_count >= self.config.failure_threshold {
                                state.current_state = CircuitState::Open;
                                tracing::warn!(
                                    failures = state.failure_count,
                                    "Circuit breaker opened due to failures"
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        
        result
    }
}

// Usage
pub struct ResilientExecutor {
    inner: Arc<dyn Executor>,
    circuit_breaker: CircuitBreaker,
}

impl ResilientExecutor {
    pub async fn execute_task(&self, task: Task) -> Result<TaskResult> {
        self.circuit_breaker
            .call(|| self.inner.execute_task(task.clone()))
            .await
    }
}
```

---

**[Continue to next sections in follow-up response due to length...]**

