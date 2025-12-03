# Servo: Serverless Data Orchestration Platform
## Project Design Document v1.0

**Document Status:** Draft for Implementation  
**Last Updated:** November 2024  
**Authors:** Engineering Team  
**Reviewers:** [To be assigned]

---

## Executive Summary

**Servo** is an open-source, asset-centric data orchestration platform designed for serverless compute environments and multi-tenant SaaS applications. Built with a high-performance Rust core and ergonomic Python SDK, Servo delivers enterprise-grade orchestration features—including comprehensive lineage tracking, observability, retry logic, and versioning—without requiring persistent infrastructure.

**Key Differentiators:**
- **Zero idle costs**: Scales to $0 when inactive, unlike Airflow/Dagster/Prefect
- **Multi-tenant by design**: First-class tenant isolation, cost attribution, quota management
- **Asset-centric**: Data lineage as a core feature, not an afterthought
- **Cloud-native**: Uses managed queuing services, not custom schedulers
- **Performance-optimized**: Rust core provides 10-100x performance improvements for critical paths

**Target Market:**
1. SaaS companies building data products (multi-tenant analytics platforms)
2. Serverless-first data teams (Cloud Run, Lambda, ECS environments)
3. Cost-conscious organizations (startups, SMBs, cost centers)
4. Teams building on modern data stacks (DataFusion, Delta Lake, dbt)

**Business Model:**
- Open-source core (Apache 2.0 license)
- Optional managed service (Servo Cloud) for teams wanting zero-ops
- Enterprise features (SSO, advanced RBAC, dedicated support)

---

## Table of Contents

1. [Vision and Goals](#1-vision-and-goals)
2. [Core Concepts](#2-core-concepts)
3. [System Architecture](#3-system-architecture)
4. [Technical Design](#4-technical-design)
5. [Infrastructure Design](#5-infrastructure-design)
6. [Data Models](#6-data-models)
7. [API Specifications](#7-api-specifications)
8. [Security Architecture](#8-security-architecture)
9. [Implementation Roadmap](#9-implementation-roadmap)
10. [Success Metrics](#10-success-metrics)
11. [Risk Assessment](#11-risk-assessment)
12. [Appendices](#12-appendices)

---

## 1. Vision and Goals

### 1.1 Vision Statement

**"Servo makes data orchestration as scalable and cost-efficient as serverless compute—empowering teams to build reliable, observable data platforms without operational overhead."**

### 1.2 Strategic Goals

**Short-term (6 months):**
- MVP with GCP support (Cloud Tasks + Cloud Run + CloudSQL)
- Python SDK with core features (@asset, @workflow decorators)
- Basic lineage tracking and metadata storage
- CLI for local development and deployment
- 10 early adopter deployments

**Medium-term (12 months):**
- AWS and Azure adapter implementations
- Optional UI for execution monitoring and lineage visualization
- Comprehensive connector ecosystem (dbt, Airbyte, Spark, etc.)
- OpenTelemetry integration
- 100 production deployments, 1000+ GitHub stars

**Long-term (24 months):**
- Managed service (Servo Cloud) launch
- Enterprise features (advanced RBAC, SSO, SLA guarantees)
- Real-time streaming orchestration support
- Advanced cost optimization (spot instance management)
- Market leader for serverless data orchestration

### 1.3 Success Criteria

**Adoption Metrics:**
- 1,000 GitHub stars in Year 1
- 100 production deployments in Year 1
- 10 enterprise customers for managed service in Year 2
- Active community with 20+ contributors

**Technical Metrics:**
- Deploy-to-first-execution: <10 minutes
- Cold start latency: <100ms
- Lineage query performance: <100ms for 1000 assets
- Support 10,000 concurrent executions per cluster
- 99.9% uptime for Servo Cloud

**Business Metrics:**
- $500K ARR from Servo Cloud by end of Year 2
- 50% lower total cost of ownership vs Dagster/Airflow
- Net Promoter Score (NPS) > 40

### 1.4 Non-Goals

**What Servo is NOT:**
- Not a compute engine (uses Cloud Run/Lambda/ECS)
- Not a storage layer (uses GCS/S3/Azure Blob)
- Not a transformation framework (integrates with dbt/Spark/DataFusion)
- Not a real-time streaming platform (focused on batch/micro-batch)
- Not replacing existing enterprise Airflow deployments (different use case)

---

## 2. Core Concepts

### 2.1 Assets

**Definition:** An asset is a persistent data artifact produced by computation—a table, file, ML model, or report.

**Key Properties:**
- **Unique identifier**: `asset_id` (UUID)
- **Asset key**: Human-readable name (e.g., `"customers.clean_customers"`)
- **Schema**: Type information (optional but encouraged)
- **Dependencies**: Upstream assets required for computation
- **Metadata**: Owner, SLA, tags, business context
- **Partitions**: Time-based or custom partition keys

**Example:**
```python
@asset(
    name="clean_customers",
    description="Deduplicated customer records",
    schema=CustomerSchema,
    owner="data-team@company.com",
    tags=["pii", "gdpr"],
    partitions=PartitionDefinition.daily("date")
)
def clean_customers(raw_customers: Dataset) -> Dataset:
    return raw_customers.drop_duplicates()
```

### 2.2 Workflows

**Definition:** A workflow is a collection of assets with defined execution order.

**Key Properties:**
- **Workflow ID**: Unique identifier
- **Assets**: List of asset definitions
- **Schedule**: Cron, interval, or event-driven triggers
- **Executor**: Compute backend (Cloud Run, Lambda, etc.)
- **Concurrency Policy**: Limits on parallel execution

**Execution Model:**
1. User defines workflow via Python decorators
2. Servo compiles to execution plan (topological sort of assets)
3. Tasks enqueued to cloud queue service (Cloud Tasks, SQS)
4. Workers pull tasks, execute, record results
5. Downstream tasks triggered automatically

### 2.3 Executions

**Definition:** An execution is a single run of a workflow or asset.

**Lifecycle States:**
- `PENDING`: Enqueued but not yet started
- `RUNNING`: Currently executing
- `SUCCEEDED`: Completed successfully
- `FAILED`: Terminated with error
- `CANCELLED`: Manually stopped
- `TIMEOUT`: Exceeded time limit

**Tracked Metadata:**
- Execution ID (for correlation)
- Tenant ID (for multi-tenancy)
- Start/end timestamps
- Resource usage (CPU, memory, cost)
- Materialized assets
- Error details (if failed)

### 2.4 Lineage

**Definition:** Lineage is the graph of dependencies between assets—"what data depends on what."

**Lineage Types:**
- **Asset lineage**: Relationships between data artifacts
- **Execution lineage**: Which execution produced which assets
- **Column lineage**: Field-level dependencies (optional)

**Storage:**
Lineage stored as directed acyclic graph (DAG) in PostgreSQL:
```sql
-- Assets table
CREATE TABLE assets (
    asset_id UUID PRIMARY KEY,
    tenant_id UUID,
    asset_key TEXT NOT NULL,
    asset_type TEXT,
    location TEXT,
    last_materialized_at TIMESTAMPTZ,
    last_execution_id UUID
);

-- Lineage edges
CREATE TABLE asset_dependencies (
    downstream_asset_id UUID REFERENCES assets(asset_id),
    upstream_asset_id UUID REFERENCES assets(asset_id),
    last_seen_execution_id UUID,
    PRIMARY KEY (downstream_asset_id, upstream_asset_id)
);
```

### 2.5 Multi-Tenancy

**Definition:** Support for isolated execution contexts per customer/organization.

**Isolation Mechanisms:**
- **Execution isolation**: Separate task queues or namespace tags
- **Storage isolation**: Tenant-specific prefixes (`gs://data/{tenant_id}/`)
- **Metadata isolation**: Row-level security in PostgreSQL
- **Cost attribution**: Tag all cloud resources with `tenant_id`

**Tenant Configuration:**
```python
@workflow(
    tenant_id="{{ context.tenant_id }}",
    tenant_config=TenantConfig(
        storage_prefix="gs://data/{tenant_id}/",
        compute_quota=ComputeQuota(max_cpu_hours_per_day=100),
        priority=TenantPriority.STANDARD
    )
)
def tenant_pipeline(context: Context):
    pass
```

---

## 3. System Architecture

### 3.1 High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         USER LAYER                              │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Python SDK (servo-python)                               │  │
│  │  - @asset, @workflow decorators                          │  │
│  │  - High-level API for workflow definition                │  │
│  │  - Integration libraries (dbt, Spark, Airbyte)           │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                            │
                            │ PyO3 FFI
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                      RUST CORE LAYER                            │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-core                                              │  │
│  │  - Workflow compiler & execution engine                  │  │
│  │  - Asset registry & dependency resolver                  │  │
│  │  - Task scheduler                                        │  │
│  └──────────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-runtime                                           │  │
│  │  - Execution state machine                               │  │
│  │  - Retry & error handling logic                          │  │
│  │  - Idempotency & concurrency control                     │  │
│  └──────────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-lineage                                           │  │
│  │  - Lineage graph operations                              │  │
│  │  - Impact analysis & upstream/downstream queries         │  │
│  └──────────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-storage                                           │  │
│  │  - PostgreSQL metadata operations                        │  │
│  │  - Schema migrations                                     │  │
│  │  - Multi-tenant row-level security                       │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                            │
                            │ Cloud Provider Adapters
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                   CLOUD INFRASTRUCTURE LAYER                    │
│  ┌──────────────┬──────────────┬──────────────┐                │
│  │ servo-cloud- │ servo-cloud- │ servo-cloud- │                │
│  │ gcp          │ aws          │ azure        │                │
│  └──────────────┴──────────────┴──────────────┘                │
│                                                                 │
│  GCP:                  AWS:                  Azure:            │
│  • Cloud Tasks         • SQS                 • Queue Storage   │
│  • Cloud Run           • Lambda/ECS          • Container Inst  │
│  • CloudSQL            • RDS                 • PostgreSQL      │
│  • GCS                 • S3                  • Blob Storage    │
│  • Workload Identity   • IAM Roles           • Managed Identity│
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    OPTIONAL COMPONENTS                          │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-ui (React + FastAPI)                              │  │
│  │  - Execution monitoring dashboard                        │  │
│  │  - Asset catalog & search                                │  │
│  │  - Interactive lineage visualization                     │  │
│  │  - Cost attribution reports                              │  │
│  └──────────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  servo-cli                                               │  │
│  │  - Local workflow testing                                │  │
│  │  - Deployment commands                                   │  │
│  │  - Lineage queries                                       │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Component Breakdown

#### **servo-core** (Rust)
**Responsibilities:**
- Workflow parsing and validation
- Asset dependency resolution (topological sort)
- Execution plan generation
- Task distribution to executors

**Key Modules:**
- `workflow.rs`: Workflow and Asset definitions
- `compiler.rs`: Compile Python definitions to execution plan
- `scheduler.rs`: Task scheduling logic
- `registry.rs`: Asset catalog management

#### **servo-runtime** (Rust)
**Responsibilities:**
- Execution state management
- Retry and error handling
- Concurrency control
- Idempotency enforcement

**Key Modules:**
- `executor.rs`: Trait for execution backends
- `state_machine.rs`: Execution lifecycle management
- `retry.rs`: Retry policies and backoff strategies
- `concurrency.rs`: Concurrency limits and semaphores

#### **servo-lineage** (Rust)
**Responsibilities:**
- Lineage graph construction and queries
- Impact analysis
- Upstream/downstream traversal

**Key Modules:**
- `graph.rs`: DAG operations using petgraph
- `queries.rs`: Lineage query APIs
- `impact.rs`: Impact analysis algorithms

#### **servo-storage** (Rust)
**Responsibilities:**
- PostgreSQL interaction
- Schema migrations
- Multi-tenant row-level security

**Key Modules:**
- `postgres.rs`: SQLX-based database operations
- `migrations.rs`: Schema versioning
- `tenant.rs`: Multi-tenant isolation logic

#### **servo-cloud-{gcp,aws,azure}** (Rust)
**Responsibilities:**
- Cloud-specific implementations of Executor trait
- Queue management
- Authentication/authorization
- Resource tagging for cost attribution

**Key Modules:**
- `executor.rs`: Cloud-specific executor
- `queue.rs`: Queue operations (enqueue, dequeue, delete)
- `auth.rs`: Workload identity/IAM integration
- `monitoring.rs`: Cloud-native observability

#### **servo-python** (Rust + Python)
**Responsibilities:**
- PyO3 bindings for Rust core
- Python decorators (@asset, @workflow)
- High-level Python API
- Integration libraries

**Key Modules:**
- `src/lib.rs`: PyO3 module definitions
- `servo/__init__.py`: Python SDK
- `servo/decorators.py`: @asset, @workflow implementation
- `servo/connectors/`: Integration libraries

### 3.3 Execution Flow

**Workflow Definition Phase:**
```
1. User writes Python code with @asset, @workflow decorators
2. Python SDK captures function signatures and metadata
3. Servo compiles to internal representation
4. Validation: check for cycles, missing dependencies
5. Store workflow definition in metadata DB
```

**Execution Phase:**
```
1. Trigger received (cron, event, manual)
2. Servo queries metadata DB for workflow definition
3. Resolve dependencies → topological sort → execution plan
4. For each ready task:
   a. Enqueue to cloud queue service with task payload
   b. Record execution state = PENDING
5. Worker (Cloud Run/Lambda) pulls task from queue
6. Worker executes task:
   a. Update state = RUNNING
   b. Execute user function
   c. Record asset materialization
   d. Update lineage graph
   e. Update state = SUCCEEDED/FAILED
7. If SUCCEEDED:
   a. Check for downstream tasks
   b. Enqueue downstream tasks if all dependencies ready
8. If FAILED:
   a. Check retry policy
   b. Re-enqueue if retries remain
   c. Otherwise, mark execution as FAILED
```

**Lineage Tracking:**
```
1. Before execution: Servo captures declared dependencies
2. During execution: Optional runtime lineage extraction
3. After execution: Record asset materialization
4. Update lineage edges in PostgreSQL:
   INSERT INTO asset_dependencies (downstream, upstream, execution_id)
5. Lineage queries use recursive CTEs for traversal
```

---

## 4. Technical Design

### 4.1 Rust Core Implementation

#### **Workflow Definition**

```rust
// servo-core/src/workflow.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub assets: Vec<Asset>,
    pub schedule: Option<Schedule>,
    pub triggers: Vec<Trigger>,
    pub executor: ExecutorConfig,
    pub tenant_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Uuid,
    pub key: String,  // e.g., "customers.clean_customers"
    pub asset_type: AssetType,
    pub dependencies: Vec<String>,  // Keys of upstream assets
    pub compute_fn: ComputeFn,
    pub schema: Option<Schema>,
    pub partitions: Option<PartitionDefinition>,
    pub retry_policy: RetryPolicy,
    pub timeout: Option<Duration>,
    pub resources: ResourceSpec,
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AssetType {
    Table,
    File,
    Model,
    Report,
    View,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComputeFn {
    PythonCallable {
        module: String,
        function: String,
        args: Vec<Value>,
        kwargs: HashMap<String, Value>,
    },
    ContainerImage {
        image: String,
        command: Vec<String>,
        env: HashMap<String, String>,
    },
    SqlQuery {
        query: String,
        connection: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub cron: Option<String>,      // "0 2 * * *"
    pub interval: Option<Duration>, // Every 4 hours
    pub timezone: String,           // "America/New_York"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trigger {
    Schedule(Schedule),
    FileArrival {
        bucket: String,
        prefix: String,
        pattern: Option<String>,
    },
    PubSubMessage {
        topic: String,
        subscription: Option<String>,
    },
    WebhookEvent {
        endpoint: String,
    },
    AssetMaterialized {
        asset_key: String,
    },
}

impl Workflow {
    /// Resolve dependencies and create execution plan
    pub fn create_execution_plan(&self) -> Result<ExecutionPlan, Error> {
        // Topological sort of assets
        let sorted_assets = self.topological_sort()?;
        
        // Group into execution levels (assets that can run in parallel)
        let levels = self.group_by_level(&sorted_assets);
        
        Ok(ExecutionPlan {
            workflow_id: self.id,
            levels,
            total_assets: self.assets.len(),
        })
    }
    
    fn topological_sort(&self) -> Result<Vec<Asset>, Error> {
        // Build dependency graph
        let mut graph = DiGraph::new();
        let mut node_map = HashMap::new();
        
        // Add nodes
        for asset in &self.assets {
            let idx = graph.add_node(asset.clone());
            node_map.insert(asset.key.clone(), idx);
        }
        
        // Add edges
        for asset in &self.assets {
            let downstream_idx = node_map[&asset.key];
            for dep_key in &asset.dependencies {
                if let Some(&upstream_idx) = node_map.get(dep_key) {
                    graph.add_edge(upstream_idx, downstream_idx, ());
                } else {
                    return Err(Error::MissingDependency {
                        asset: asset.key.clone(),
                        missing: dep_key.clone(),
                    });
                }
            }
        }
        
        // Check for cycles
        if is_cyclic_directed(&graph) {
            return Err(Error::CyclicDependency);
        }
        
        // Topological sort using Kahn's algorithm
        let sorted_indices = toposort(&graph, None)
            .map_err(|_| Error::TopologicalSortFailed)?;
        
        Ok(sorted_indices
            .into_iter()
            .map(|idx| graph[idx].clone())
            .collect())
    }
}
```

#### **Execution Engine**

```rust
// servo-runtime/src/executor.rs

use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Executor: Send + Sync {
    /// Enqueue a task for execution
    async fn enqueue_task(&self, task: Task) -> Result<TaskId, Error>;
    
    /// Get status of a task
    async fn get_task_status(&self, task_id: &TaskId) -> Result<TaskStatus, Error>;
    
    /// Cancel a running task
    async fn cancel_task(&self, task_id: &TaskId) -> Result<(), Error>;
    
    /// List all tasks for an execution
    async fn list_tasks(&self, execution_id: &Uuid) -> Result<Vec<Task>, Error>;
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub execution_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub asset: Asset,
    pub inputs: Vec<AssetReference>,
    pub schedule_time: DateTime<Utc>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum TaskStatus {
    Pending,
    Running { started_at: DateTime<Utc>, worker_id: String },
    Succeeded { completed_at: DateTime<Utc>, output: AssetMaterialization },
    Failed { failed_at: DateTime<Utc>, error: String },
    Timeout { timeout_at: DateTime<Utc> },
    Cancelled { cancelled_at: DateTime<Utc> },
}

// State machine for task execution
impl TaskStatus {
    pub fn transition_to_running(&self, worker_id: String) -> Result<Self, Error> {
        match self {
            TaskStatus::Pending => Ok(TaskStatus::Running {
                started_at: Utc::now(),
                worker_id,
            }),
            _ => Err(Error::InvalidStateTransition {
                from: format!("{:?}", self),
                to: "Running".to_string(),
            }),
        }
    }
    
    pub fn transition_to_succeeded(&self, output: AssetMaterialization) -> Result<Self, Error> {
        match self {
            TaskStatus::Running { .. } => Ok(TaskStatus::Succeeded {
                completed_at: Utc::now(),
                output,
            }),
            _ => Err(Error::InvalidStateTransition {
                from: format!("{:?}", self),
                to: "Succeeded".to_string(),
            }),
        }
    }
    
    pub fn transition_to_failed(&self, error: String) -> Result<Self, Error> {
        match self {
            TaskStatus::Running { .. } | TaskStatus::Pending => Ok(TaskStatus::Failed {
                failed_at: Utc::now(),
                error,
            }),
            _ => Err(Error::InvalidStateTransition {
                from: format!("{:?}", self),
                to: "Failed".to_string(),
            }),
        }
    }
}
```

#### **GCP Executor Implementation**

```rust
// servo-cloud-gcp/src/executor.rs

use google_cloud_tasks::client::{Client as CloudTasksClient, CreateTaskRequest};
use servo_runtime::{Executor, Task, TaskId, TaskStatus};

pub struct CloudRunExecutor {
    project: String,
    region: String,
    queue: String,
    cloud_tasks_client: CloudTasksClient,
    service_url: String,  // Cloud Run service URL
}

impl CloudRunExecutor {
    pub async fn new(project: String, region: String) -> Result<Self, Error> {
        let cloud_tasks_client = CloudTasksClient::new().await?;
        let queue = format!(
            "projects/{}/locations/{}/queues/servo-default",
            project, region
        );
        let service_url = format!(
            "https://servo-worker-{}-{}.run.app",
            project, region
        );
        
        Ok(Self {
            project,
            region,
            queue,
            cloud_tasks_client,
            service_url,
        })
    }
}

#[async_trait]
impl Executor for CloudRunExecutor {
    async fn enqueue_task(&self, task: Task) -> Result<TaskId, Error> {
        // Serialize task to JSON
        let task_json = serde_json::to_vec(&task)?;
        
        // Create Cloud Tasks HTTP task
        let task_name = format!(
            "{}/tasks/{}",
            self.queue,
            task.id
        );
        
        let http_request = HttpRequest {
            url: format!("{}/execute", self.service_url),
            http_method: HttpMethod::Post,
            headers: {
                let mut h = HashMap::new();
                h.insert("Content-Type".to_string(), "application/json".to_string());
                if let Some(tenant_id) = &task.tenant_id {
                    h.insert("X-Tenant-Id".to_string(), tenant_id.to_string());
                }
                h
            },
            body: task_json,
            oidc_token: Some(OidcToken {
                service_account_email: format!(
                    "servo-worker@{}.iam.gserviceaccount.com",
                    self.project
                ),
                audience: self.service_url.clone(),
            }),
        };
        
        let cloud_task = CloudTask {
            name: task_name.clone(),
            http_request: Some(http_request),
            schedule_time: Some(task.schedule_time.into()),
        };
        
        // Enqueue task
        self.cloud_tasks_client
            .create_task(CreateTaskRequest {
                parent: self.queue.clone(),
                task: cloud_task,
            })
            .await?;
        
        Ok(task.id)
    }
    
    async fn get_task_status(&self, task_id: &TaskId) -> Result<TaskStatus, Error> {
        // Query metadata DB for task status
        // (Cloud Tasks doesn't provide status, we track in PostgreSQL)
        let pool = get_db_pool().await?;
        
        let row = sqlx::query!(
            r#"
            SELECT status, started_at, completed_at, failed_at, error, worker_id
            FROM task_executions
            WHERE task_id = $1
            "#,
            task_id.as_uuid()
        )
        .fetch_one(&pool)
        .await?;
        
        let status = match row.status.as_str() {
            "pending" => TaskStatus::Pending,
            "running" => TaskStatus::Running {
                started_at: row.started_at.unwrap(),
                worker_id: row.worker_id.unwrap(),
            },
            "succeeded" => TaskStatus::Succeeded {
                completed_at: row.completed_at.unwrap(),
                output: self.get_output(task_id).await?,
            },
            "failed" => TaskStatus::Failed {
                failed_at: row.failed_at.unwrap(),
                error: row.error.unwrap(),
            },
            _ => return Err(Error::UnknownStatus),
        };
        
        Ok(status)
    }
    
    async fn cancel_task(&self, task_id: &TaskId) -> Result<(), Error> {
        // Delete from Cloud Tasks queue
        let task_name = format!("{}/tasks/{}", self.queue, task_id);
        
        self.cloud_tasks_client
            .delete_task(task_name)
            .await?;
        
        // Update metadata DB
        let pool = get_db_pool().await?;
        sqlx::query!(
            r#"
            UPDATE task_executions
            SET status = 'cancelled', cancelled_at = NOW()
            WHERE task_id = $1
            "#,
            task_id.as_uuid()
        )
        .execute(&pool)
        .await?;
        
        Ok(())
    }
}
```

#### **Lineage Graph Operations**

```rust
// servo-lineage/src/graph.rs

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo::{dijkstra, has_path_connecting};
use std::collections::HashMap;

pub struct LineageGraph {
    graph: DiGraph<Asset, DependencyEdge>,
    asset_index: HashMap<String, NodeIndex>,
}

#[derive(Debug, Clone)]
pub struct DependencyEdge {
    pub execution_id: Uuid,
    pub transformation_type: String,  // "join", "filter", "aggregate"
    pub column_mappings: Option<HashMap<String, Vec<String>>>,
}

impl LineageGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            asset_index: HashMap::new(),
        }
    }
    
    /// Load lineage from PostgreSQL
    pub async fn load_from_db(pool: &PgPool) -> Result<Self, Error> {
        let mut graph = Self::new();
        
        // Load all assets
        let assets = sqlx::query_as!(
            Asset,
            r#"
            SELECT asset_id, asset_key, asset_type, location,
                   last_materialized_at, metadata
            FROM assets
            WHERE tenant_id = $1 OR tenant_id IS NULL
            ORDER BY asset_key
            "#,
            tenant_id
        )
        .fetch_all(pool)
        .await?;
        
        for asset in assets {
            graph.add_asset(asset);
        }
        
        // Load dependencies
        let edges = sqlx::query!(
            r#"
            SELECT downstream_asset_id, upstream_asset_id, last_seen_execution_id
            FROM asset_dependencies
            "#
        )
        .fetch_all(pool)
        .await?;
        
        for edge in edges {
            graph.add_dependency(
                &edge.downstream_asset_id.to_string(),
                &edge.upstream_asset_id.to_string(),
                edge.last_seen_execution_id,
            );
        }
        
        Ok(graph)
    }
    
    pub fn add_asset(&mut self, asset: Asset) -> NodeIndex {
        let idx = self.graph.add_node(asset.clone());
        self.asset_index.insert(asset.key.clone(), idx);
        idx
    }
    
    pub fn add_dependency(
        &mut self,
        downstream: &str,
        upstream: &str,
        execution_id: Uuid,
    ) {
        if let (Some(&down_idx), Some(&up_idx)) = (
            self.asset_index.get(downstream),
            self.asset_index.get(upstream),
        ) {
            self.graph.add_edge(
                up_idx,
                down_idx,
                DependencyEdge {
                    execution_id,
                    transformation_type: "unknown".to_string(),
                    column_mappings: None,
                },
            );
        }
    }
    
    /// Get all upstream assets up to specified depth
    pub fn upstream_assets(&self, asset_key: &str, depth: u32) -> Vec<Asset> {
        let start_idx = match self.asset_index.get(asset_key) {
            Some(idx) => *idx,
            None => return vec![],
        };
        
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back((start_idx, 0));
        
        while let Some((idx, current_depth)) = queue.pop_front() {
            if current_depth >= depth || visited.contains(&idx) {
                continue;
            }
            
            visited.insert(idx);
            
            // Get all incoming edges (dependencies)
            for edge in self.graph.edges_directed(idx, petgraph::Direction::Incoming) {
                let upstream_idx = edge.source();
                result.push(self.graph[upstream_idx].clone());
                queue.push_back((upstream_idx, current_depth + 1));
            }
        }
        
        result
    }
    
    /// Get all downstream assets (impact analysis)
    pub fn downstream_assets(&self, asset_key: &str) -> Vec<Asset> {
        let start_idx = match self.asset_index.get(asset_key) {
            Some(idx) => *idx,
            None => return vec![],
        };
        
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![start_idx];
        
        while let Some(idx) = stack.pop() {
            if visited.contains(&idx) {
                continue;
            }
            
            visited.insert(idx);
            
            // Get all outgoing edges (dependents)
            for edge in self.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
                let downstream_idx = edge.target();
                result.push(self.graph[downstream_idx].clone());
                stack.push(downstream_idx);
            }
        }
        
        result
    }
    
    /// Export to Cytoscape.js format for UI visualization
    pub fn to_cytoscape_json(&self) -> serde_json::Value {
        let mut nodes = vec![];
        let mut edges = vec![];
        
        for (key, &idx) in &self.asset_index {
            let asset = &self.graph[idx];
            nodes.push(json!({
                "data": {
                    "id": key,
                    "label": key,
                    "type": asset.asset_type,
                    "metadata": asset.metadata,
                }
            }));
        }
        
        for edge in self.graph.edge_references() {
            let source = &self.graph[edge.source()].key;
            let target = &self.graph[edge.target()].key;
            edges.push(json!({
                "data": {
                    "source": source,
                    "target": target,
                    "transformation": edge.weight().transformation_type,
                }
            }));
        }
        
        json!({
            "elements": {
                "nodes": nodes,
                "edges": edges,
            }
        })
    }
}
```

### 4.2 Python SDK Implementation

#### **Decorators**

```python
# servo/decorators.py

import functools
import inspect
from typing import Any, Callable, List, Optional, Type
from servo._servo import (  # Rust extension module
    PyAsset,
    PyWorkflow,
    PyExecutor,
)

def asset(
    name: str,
    description: Optional[str] = None,
    dependencies: Optional[List[str]] = None,
    schema: Optional[Type] = None,
    partitions: Optional[PartitionDefinition] = None,
    retry: Optional[RetryPolicy] = None,
    timeout: Optional[str] = None,
    resources: Optional[Resources] = None,
    metadata: Optional[dict] = None,
):
    """
    Decorator to define a data asset.
    
    Example:
        @asset(
            name="clean_customers",
            dependencies=["raw_customers"],
            schema=CustomerSchema,
            retry=RetryPolicy(max_attempts=3)
        )
        def clean_customers(raw: Dataset) -> Dataset:
            return raw.drop_duplicates()
    """
    def decorator(func: Callable) -> Callable:
        # Extract type hints for automatic lineage
        sig = inspect.signature(func)
        input_types = []
        for param in sig.parameters.values():
            if param.annotation != inspect.Parameter.empty:
                input_types.append(param.annotation)
        
        output_type = sig.return_annotation if sig.return_annotation != inspect.Signature.empty else None
        
        # Infer dependencies from function parameters if not specified
        if dependencies is None:
            inferred_deps = []
            for param_name in sig.parameters.keys():
                # Check if parameter name matches an asset name
                # This is a simple heuristic; can be improved
                inferred_deps.append(param_name)
        else:
            inferred_deps = dependencies
        
        # Create Rust asset object via PyO3
        asset_def = PyAsset(
            name=name,
            description=description,
            dependencies=inferred_deps,
            schema=schema,
            input_types=input_types,
            output_type=output_type,
            compute_fn=func,
            partitions=partitions,
            retry=retry or RetryPolicy.default(),
            timeout=timeout,
            resources=resources or Resources.default(),
            metadata=metadata or {},
        )
        
        @functools.wraps(func)
        def wrapper(*args, **kwargs):
            # When called directly, just execute the function
            return func(*args, **kwargs)
        
        # Attach asset definition for workflow discovery
        wrapper._servo_asset = asset_def
        return wrapper
    
    return decorator


def workflow(
    name: Optional[str] = None,
    description: Optional[str] = None,
    schedule: Optional[Schedule] = None,
    sensors: Optional[List[Sensor]] = None,
    executor: Optional[Executor] = None,
    concurrency: Optional[ConcurrencyPolicy] = None,
    tenant_id: Optional[str] = None,
    security: Optional[Security] = None,
    observability: Optional[ObservabilityConfig] = None,
):
    """
    Decorator to define a workflow.
    
    Example:
        @workflow(
            name="daily_etl",
            schedule=Schedule.cron("0 2 * * *"),
            executor=executor.CloudRun(project="my-project")
        )
        def daily_pipeline():
            raw = extract_data()
            clean = clean_customers(raw)
            segments = segment_customers(clean)
            return segments
    """
    def decorator(func: Callable) -> Callable:
        # Discover all @asset decorated functions called in this workflow
        assets = _discover_assets_in_function(func)
        
        # Create Rust workflow object
        workflow_def = PyWorkflow(
            name=name or func.__name__,
            description=description,
            assets=assets,
            schedule=schedule,
            sensors=sensors or [],
            executor=executor or _get_default_executor(),
            concurrency=concurrency,
            tenant_id=tenant_id,
            security=security,
            observability=observability,
        )
        
        @functools.wraps(func)
        def wrapper(**kwargs):
            # Execute workflow via Rust runtime
            execution_id = workflow_def.execute(**kwargs)
            return execution_id
        
        wrapper._servo_workflow = workflow_def
        return wrapper
    
    return decorator


def _discover_assets_in_function(func: Callable) -> List[PyAsset]:
    """
    Discover @asset decorated functions referenced in workflow function.
    Uses AST parsing to find function calls.
    """
    import ast
    
    source = inspect.getsource(func)
    tree = ast.parse(source)
    
    assets = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call):
            if isinstance(node.func, ast.Name):
                # Try to resolve the function
                func_name = node.func.id
                if func_name in func.__globals__:
                    called_func = func.__globals__[func_name]
                    if hasattr(called_func, '_servo_asset'):
                        assets.append(called_func._servo_asset)
    
    return assets
```

#### **Executor Configuration**

```python
# servo/executor.py

from dataclasses import dataclass
from typing import Optional
from servo._servo import PyExecutor

@dataclass
class Resources:
    cpu: str = "1"
    memory: str = "512Mi"
    timeout: str = "30m"
    gpu: Optional[str] = None
    disk: Optional[str] = None
    spot_instances: bool = False
    
    @staticmethod
    def default():
        return Resources()


class CloudRunExecutor:
    """GCP Cloud Run executor."""
    
    def __init__(
        self,
        project: str,
        region: str = "us-central1",
        service_account: Optional[str] = None,
        vpc_connector: Optional[str] = None,
        max_parallel_tasks: int = 10,
    ):
        self.project = project
        self.region = region
        self.service_account = service_account
        self.vpc_connector = vpc_connector
        self.max_parallel_tasks = max_parallel_tasks
        
        # Create Rust executor via PyO3
        self._inner = PyExecutor.cloud_run(
            project=project,
            region=region,
            service_account=service_account,
            vpc_connector=vpc_connector,
            max_parallel_tasks=max_parallel_tasks,
        )
    
    def to_dict(self):
        return {
            "type": "cloud_run",
            "project": self.project,
            "region": self.region,
            "service_account": self.service_account,
            "vpc_connector": self.vpc_connector,
            "max_parallel_tasks": self.max_parallel_tasks,
        }


class LambdaExecutor:
    """AWS Lambda executor."""
    
    def __init__(
        self,
        region: str = "us-east-1",
        role_arn: Optional[str] = None,
        vpc_config: Optional[dict] = None,
        max_parallel_tasks: int = 10,
    ):
        self.region = region
        self.role_arn = role_arn
        self.vpc_config = vpc_config
        self.max_parallel_tasks = max_parallel_tasks
        
        self._inner = PyExecutor.lambda_(
            region=region,
            role_arn=role_arn,
            vpc_config=vpc_config,
            max_parallel_tasks=max_parallel_tasks,
        )
```

### 4.3 Worker Implementation

**Cloud Run Worker (Entry Point)**

```python
# servo-worker/main.py

from fastapi import FastAPI, Request, HTTPException
from servo._servo import PyTask, PyRuntime
import logging

app = FastAPI()
runtime = PyRuntime.initialize()

logger = logging.getLogger(__name__)

@app.post("/execute")
async def execute_task(request: Request):
    """
    Worker endpoint for task execution.
    Called by Cloud Tasks with task payload.
    """
    try:
        # Parse task from request
        task_json = await request.json()
        task = PyTask.from_json(task_json)
        
        tenant_id = request.headers.get("X-Tenant-Id")
        if tenant_id:
            task.tenant_id = tenant_id
        
        logger.info(
            "Executing task",
            extra={
                "task_id": str(task.id),
                "execution_id": str(task.execution_id),
                "tenant_id": tenant_id,
                "asset_key": task.asset.key,
            }
        )
        
        # Execute task via Rust runtime
        result = await runtime.execute_task(task)
        
        logger.info(
            "Task completed successfully",
            extra={
                "task_id": str(task.id),
                "asset_id": str(result.asset_id),
                "location": result.location,
            }
        )
        
        return {
            "status": "success",
            "asset_id": str(result.asset_id),
            "location": result.location,
        }
        
    except Exception as e:
        logger.error(
            "Task execution failed",
            extra={
                "error": str(e),
                "task_id": task_json.get("id"),
            },
            exc_info=True
        )
        
        # Record failure in metadata DB
        await runtime.record_task_failure(task.id, str(e))
        
        # Check retry policy
        if task.retry_count < task.max_retries:
            # Re-enqueue with exponential backoff
            await runtime.retry_task(task)
            return {"status": "retrying", "retry_count": task.retry_count + 1}
        else:
            return {"status": "failed", "error": str(e)}, 500


@app.get("/health")
async def health_check():
    """Health check endpoint for Cloud Run."""
    return {"status": "healthy"}


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8080)
```

**Dockerfile for Worker**

```dockerfile
# Dockerfile

FROM rust:1.75 as rust-builder

WORKDIR /app
COPY servo-core ./servo-core
COPY servo-runtime ./servo-runtime
COPY servo-storage ./servo-storage
COPY servo-lineage ./servo-lineage
COPY servo-cloud-gcp ./servo-cloud-gcp
COPY servo-python ./servo-python
COPY Cargo.toml Cargo.lock ./

# Build Rust core and Python extension
RUN cargo build --release

FROM python:3.11-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy Rust binaries
COPY --from=rust-builder /app/target/release/libservo.so /usr/local/lib/
COPY --from=rust-builder /app/target/release/servo-cli /usr/local/bin/

# Copy Python application
COPY servo-worker/requirements.txt ./
RUN pip install --no-cache-dir -r requirements.txt

COPY servo-worker/ ./

# Set environment variables
ENV PYTHONPATH=/app
ENV LD_LIBRARY_PATH=/usr/local/lib:$LD_LIBRARY_PATH

# Expose port
EXPOSE 8080

# Run worker
CMD ["python", "main.py"]
```

---

## 5. Infrastructure Design

### 5.1 GCP Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        GCP INFRASTRUCTURE                        │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Control Plane (Optional - for UI/API)                 │    │
│  │                                                          │    │
│  │  Cloud Run Service: servo-api                           │    │
│  │  - FastAPI backend for REST API                         │    │
│  │  - Serves UI static files                               │    │
│  │  - Handles workflow registration                        │    │
│  │  - Query lineage / execution history                    │    │
│  │  - Min instances: 0 (scale to zero)                     │    │
│  │  - Max instances: 10                                     │    │
│  │  - Workload Identity: servo-api@project.iam             │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           │ Reads/Writes                         │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Metadata Storage                                       │    │
│  │                                                          │    │
│  │  CloudSQL PostgreSQL                                    │    │
│  │  - Database: servo_metadata                             │    │
│  │  - Machine type: db-f1-micro (dev) / db-n1-standard-2  │    │
│  │  - Storage: 20GB SSD, auto-resize enabled              │    │
│  │  - High Availability: true (prod)                       │    │
│  │  - Backups: Automated daily, 7-day retention           │    │
│  │  - Private IP: connected via VPC peering                │    │
│  │  - Tables: executions, assets, asset_dependencies,     │    │
│  │            task_executions, workflows, etc.             │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Orchestration Queue                                    │    │
│  │                                                          │    │
│  │  Cloud Tasks                                            │    │
│  │  - Queue: servo-default                                 │    │
│  │  - Rate limits: 500 dispatches/sec                      │    │
│  │  - Max concurrent: 1000                                 │    │
│  │  - Max retry: 5 attempts                                │    │
│  │  - Retry backoff: exponential (2s to 5m)               │    │
│  │  - Target: Cloud Run service (servo-worker)            │    │
│  │  - OIDC authentication for security                     │    │
│  │                                                          │    │
│  │  Optional: Additional queues for priority:             │    │
│  │  - servo-high-priority                                  │    │
│  │  - servo-low-priority                                   │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           │ Triggers                             │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Execution Workers                                      │    │
│  │                                                          │    │
│  │  Cloud Run Service: servo-worker                        │    │
│  │  - Container: Rust runtime + Python SDK                 │    │
│  │  - Min instances: 0 (scale to zero)                     │    │
│  │  - Max instances: 100 (per-tenant limits)              │    │
│  │  - CPU: 2 vCPU, Memory: 4 GiB (configurable)          │    │
│  │  - Timeout: 3600s (1 hour max per task)               │    │
│  │  - Concurrency: 10 requests per instance               │    │
│  │  - Workload Identity: servo-worker@project.iam         │    │
│  │  - VPC Connector: for CloudSQL private IP             │    │
│  │                                                          │    │
│  │  Autoscaling triggers:                                  │    │
│  │  - CPU utilization > 70%                                │    │
│  │  - Request latency > 5s                                 │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           │ Reads/Writes                         │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Data Storage                                           │    │
│  │                                                          │    │
│  │  Cloud Storage Buckets                                  │    │
│  │  - servo-artifacts-{project}: Execution outputs        │    │
│  │  - servo-logs-{project}: Structured logs               │    │
│  │  - Per-tenant: servo-data-{tenant_id}                  │    │
│  │  - Lifecycle: Archive to Coldline after 90 days        │    │
│  │  - Versioning: Enabled for audit trail                 │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Scheduling & Triggers                                  │    │
│  │                                                          │    │
│  │  Cloud Scheduler                                        │    │
│  │  - Cron jobs trigger Cloud Tasks                        │    │
│  │  - Example: "0 2 * * *" → daily-etl workflow           │    │
│  │                                                          │    │
│  │  Pub/Sub Topics                                         │    │
│  │  - servo-events: Workflow triggers                      │    │
│  │  - servo-asset-materialized: Asset completion events   │    │
│  │                                                          │    │
│  │  Eventarc                                               │    │
│  │  - GCS file arrival → Pub/Sub → Cloud Tasks            │    │
│  │  - BigQuery job completion → workflow trigger          │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Observability                                          │    │
│  │                                                          │    │
│  │  Cloud Logging                                          │    │
│  │  - Structured logs from all components                  │    │
│  │  - Correlation via execution_id                         │    │
│  │  - Retention: 30 days (configurable)                    │    │
│  │                                                          │    │
│  │  Cloud Monitoring                                       │    │
│  │  - Metrics: task_duration, task_failures, queue_depth  │    │
│  │  - Alerts: High failure rate, queue backlog             │    │
│  │  - Dashboards: Execution trends, cost attribution       │    │
│  │                                                          │    │
│  │  Cloud Trace                                            │    │
│  │  - Distributed tracing for task execution               │    │
│  │  - Latency analysis                                     │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Security                                               │    │
│  │                                                          │    │
│  │  Workload Identity                                      │    │
│  │  - servo-api@project.iam.gserviceaccount.com           │    │
│  │    - Roles: cloudsql.client, storage.objectViewer      │    │
│  │  - servo-worker@project.iam.gserviceaccount.com        │    │
│  │    - Roles: cloudsql.client, storage.objectAdmin,      │    │
│  │              cloudtasks.enqueuer, logging.logWriter     │    │
│  │                                                          │    │
│  │  Secret Manager                                         │    │
│  │  - Database credentials                                 │    │
│  │  - API keys for external services                       │    │
│  │                                                          │    │
│  │  VPC Service Controls (Enterprise)                     │    │
│  │  - Perimeter around CloudSQL, GCS                       │    │
│  │  - Restrict data exfiltration                           │    │
│  └────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

**GCP Terraform Configuration**

```hcl
# terraform/gcp/main.tf

terraform {
  required_version = ">= 1.0"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
}

# CloudSQL PostgreSQL instance
resource "google_sql_database_instance" "servo_db" {
  name             = "servo-metadata"
  database_version = "POSTGRES_15"
  region           = var.region
  
  settings {
    tier              = var.db_tier  # "db-f1-micro" for dev, "db-n1-standard-2" for prod
    availability_type = var.high_availability ? "REGIONAL" : "ZONAL"
    disk_size         = 20
    disk_autoresize   = true
    disk_type         = "PD_SSD"
    
    backup_configuration {
      enabled            = true
      start_time         = "03:00"
      point_in_time_recovery_enabled = true
      transaction_log_retention_days = 7
    }
    
    ip_configuration {
      ipv4_enabled    = false
      private_network = google_compute_network.servo_network.id
      require_ssl     = true
    }
    
    database_flags {
      name  = "max_connections"
      value = "100"
    }
  }
  
  deletion_protection = var.deletion_protection
}

resource "google_sql_database" "servo_metadata" {
  name     = "servo_metadata"
  instance = google_sql_database_instance.servo_db.name
}

# VPC for private CloudSQL access
resource "google_compute_network" "servo_network" {
  name                    = "servo-network"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "servo_subnet" {
  name          = "servo-subnet"
  ip_cidr_range = "10.0.0.0/24"
  region        = var.region
  network       = google_compute_network.servo_network.id
}

# VPC Connector for Cloud Run
resource "google_vpc_access_connector" "servo_connector" {
  name          = "servo-connector"
  region        = var.region
  network       = google_compute_network.servo_network.name
  ip_cidr_range = "10.8.0.0/28"
}

# Service Account for Worker
resource "google_service_account" "servo_worker" {
  account_id   = "servo-worker"
  display_name = "Servo Worker Service Account"
}

resource "google_project_iam_member" "worker_cloudsql" {
  project = var.project_id
  role    = "roles/cloudsql.client"
  member  = "serviceAccount:${google_service_account.servo_worker.email}"
}

resource "google_project_iam_member" "worker_storage" {
  project = var.project_id
  role    = "roles/storage.objectAdmin"
  member  = "serviceAccount:${google_service_account.servo_worker.email}"
}

resource "google_project_iam_member" "worker_tasks" {
  project = var.project_id
  role    = "roles/cloudtasks.enqueuer"
  member  = "serviceAccount:${google_service_account.servo_worker.email}"
}

# Cloud Tasks Queue
resource "google_cloud_tasks_queue" "servo_default" {
  name     = "servo-default"
  location = var.region
  
  rate_limits {
    max_dispatches_per_second = 500
    max_concurrent_dispatches = 1000
  }
  
  retry_config {
    max_attempts       = 5
    max_retry_duration = "300s"
    min_backoff        = "2s"
    max_backoff        = "300s"
    max_doublings      = 5
  }
}

# Cloud Run Worker Service
resource "google_cloud_run_v2_service" "servo_worker" {
  name     = "servo-worker"
  location = var.region
  
  template {
    service_account = google_service_account.servo_worker.email
    
    scaling {
      min_instance_count = 0
      max_instance_count = var.max_worker_instances
    }
    
    containers {
      image = var.worker_image  # "gcr.io/${var.project_id}/servo-worker:latest"
      
      resources {
        limits = {
          cpu    = "2000m"
          memory = "4Gi"
        }
      }
      
      env {
        name  = "DATABASE_URL"
        value = "postgresql://servo:${var.db_password}@${google_sql_database_instance.servo_db.private_ip_address}:5432/servo_metadata"
      }
      
      env {
        name  = "GCS_BUCKET"
        value = google_storage_bucket.servo_artifacts.name
      }
      
      env {
        name  = "RUST_LOG"
        value = "info"
      }
    }
    
    vpc_access {
      connector = google_vpc_access_connector.servo_connector.id
      egress    = "PRIVATE_RANGES_ONLY"
    }
  }
  
  traffic {
    type    = "TRAFFIC_TARGET_ALLOCATION_TYPE_LATEST"
    percent = 100
  }
}

# GCS Buckets
resource "google_storage_bucket" "servo_artifacts" {
  name          = "servo-artifacts-${var.project_id}"
  location      = var.region
  force_destroy = false
  
  uniform_bucket_level_access = true
  
  versioning {
    enabled = true
  }
  
  lifecycle_rule {
    condition {
      age = 90
    }
    action {
      type          = "SetStorageClass"
      storage_class = "COLDLINE"
    }
  }
}

resource "google_storage_bucket" "servo_logs" {
  name          = "servo-logs-${var.project_id}"
  location      = var.region
  force_destroy = false
  
  uniform_bucket_level_access = true
  
  lifecycle_rule {
    condition {
      age = 30
    }
    action {
      type = "Delete"
    }
  }
}

# Outputs
output "worker_url" {
  value = google_cloud_run_v2_service.servo_worker.uri
}

output "db_connection_name" {
  value = google_sql_database_instance.servo_db.connection_name
}

output "artifacts_bucket" {
  value = google_storage_bucket.servo_artifacts.name
}
```

### 5.2 AWS Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        AWS INFRASTRUCTURE                        │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Control Plane (Optional - for UI/API)                 │    │
│  │                                                          │    │
│  │  ECS Fargate Service: servo-api                         │    │
│  │  OR                                                      │    │
│  │  Lambda Function URL (for API Gateway)                  │    │
│  │  - FastAPI backend                                       │    │
│  │  - Connects to RDS via VPC                              │    │
│  │  - IAM role: ServoAPIRole                               │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Metadata Storage                                       │    │
│  │                                                          │    │
│  │  RDS PostgreSQL                                         │    │
│  │  - Instance class: db.t3.micro (dev) / db.r5.large     │    │
│  │  - Storage: 20GB gp3, auto-scaling enabled             │    │
│  │  - Multi-AZ: true (prod)                                │    │
│  │  - Backups: Automated, 7-day retention                  │    │
│  │  - Encryption: AWS KMS                                  │    │
│  │  - VPC: Private subnets only                            │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Orchestration Queue                                    │    │
│  │                                                          │    │
│  │  Amazon SQS                                             │    │
│  │  - Queue: servo-default.fifo                            │    │
│  │  - Message retention: 14 days                           │    │
│  │  - Visibility timeout: 3600s                            │    │
│  │  - DLQ: servo-dlq (after 5 retries)                    │    │
│  │  - Encryption: AWS KMS                                  │    │
│  │                                                          │    │
│  │  Optional priority queues:                              │    │
│  │  - servo-high-priority.fifo                             │    │
│  │  - servo-low-priority.fifo                              │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           │ Triggers via Lambda                  │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Execution Workers                                      │    │
│  │                                                          │    │
│  │  OPTION A: Lambda Functions                             │    │
│  │  - Runtime: Custom (Rust binary)                        │    │
│  │  - Memory: 2048 MB                                      │    │
│  │  - Timeout: 900s (15 min max)                           │    │
│  │  - Concurrency: 100 (per-tenant limits)                │    │
│  │  - IAM role: ServoWorkerRole                            │    │
│  │  - VPC: Access to RDS via ENI                           │    │
│  │                                                          │    │
│  │  OPTION B: ECS Fargate Tasks                            │    │
│  │  - Task definition: servo-worker                        │    │
│  │  - CPU: 2 vCPU, Memory: 4 GB                            │    │
│  │  - Scaling: Target tracking on SQS queue depth         │    │
│  │  - IAM role: ServoWorkerRole                            │    │
│  │  - VPC: Private subnets with NAT gateway               │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Data Storage                                           │    │
│  │                                                          │    │
│  │  S3 Buckets                                             │    │
│  │  - servo-artifacts-{account}: Execution outputs        │    │
│  │  - servo-logs-{account}: Structured logs               │    │
│  │  - Per-tenant: servo-data-{tenant_id}                  │    │
│  │  - Lifecycle: Transition to Glacier after 90 days      │    │
│  │  - Versioning: Enabled                                  │    │
│  │  - Encryption: AWS KMS                                  │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Scheduling & Triggers                                  │    │
│  │                                                          │    │
│  │  EventBridge Rules                                      │    │
│  │  - Cron expressions → SQS messages                      │    │
│  │  - S3 events → workflow triggers                        │    │
│  │  - Custom events from applications                      │    │
│  │                                                          │    │
│  │  SNS Topics                                             │    │
│  │  - servo-events: Workflow notifications                 │    │
│  │  - servo-alerts: Failure alerts                         │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Observability                                          │    │
│  │                                                          │    │
│  │  CloudWatch                                             │    │
│  │  - Logs: /aws/lambda/servo-worker                       │    │
│  │  - Metrics: Custom metrics for task execution           │    │
│  │  - Alarms: High error rate, queue depth                │    │
│  │  - Dashboards: Execution trends                         │    │
│  │                                                          │    │
│  │  X-Ray                                                  │    │
│  │  - Distributed tracing                                  │    │
│  │  - Performance analysis                                 │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Security                                               │    │
│  │                                                          │    │
│  │  IAM Roles                                              │    │
│  │  - ServoWorkerRole                                      │    │
│  │    - Policies: RDS access, S3 read/write, SQS send,    │    │
│  │               CloudWatch logs, Secrets Manager          │    │
│  │                                                          │    │
│  │  Secrets Manager                                        │    │
│  │  - Database credentials                                 │    │
│  │  - API keys                                             │    │
│  │                                                          │    │
│  │  VPC                                                    │    │
│  │  - Private subnets for RDS, workers                     │    │
│  │  - Public subnets for NAT gateway                       │    │
│  │  - Security groups for network isolation               │    │
│  └────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

### 5.3 Azure Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       AZURE INFRASTRUCTURE                       │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Control Plane (Optional - for UI/API)                 │    │
│  │                                                          │    │
│  │  Azure Container Instances: servo-api                   │    │
│  │  OR                                                      │    │
│  │  Azure Functions (HTTP trigger)                         │    │
│  │  - Managed identity for authentication                  │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Metadata Storage                                       │    │
│  │                                                          │    │
│  │  Azure Database for PostgreSQL                         │    │
│  │  - Tier: Basic (dev) / General Purpose                 │    │
│  │  - Storage: 20GB, auto-grow enabled                     │    │
│  │  - High Availability: Zone-redundant (prod)            │    │
│  │  - Backups: Automated, 7-day retention                  │    │
│  │  - Encryption: Azure-managed keys                       │    │
│  │  - VNet integration for private access                  │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Orchestration Queue                                    │    │
│  │                                                          │    │
│  │  Azure Queue Storage                                    │    │
│  │  - Queue: servo-default                                 │    │
│  │  - Message TTL: 14 days                                 │    │
│  │  - Visibility timeout: 3600s                            │    │
│  │  - Poison queue: servo-dlq                              │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           │ Triggers                             │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Execution Workers                                      │    │
│  │                                                          │    │
│  │  Azure Container Instances                              │    │
│  │  - Container: servo-worker                              │    │
│  │  - CPU: 2 cores, Memory: 4 GB                           │    │
│  │  - Auto-scaling based on queue depth                    │    │
│  │  - Managed identity: servo-worker                       │    │
│  │  - VNet integration for database access                 │    │
│  └────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                           ▼                                      │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Data Storage                                           │    │
│  │                                                          │    │
│  │  Azure Blob Storage                                     │    │
│  │  - Container: servo-artifacts                           │    │
│  │  - Container: servo-logs                                │    │
│  │  - Per-tenant containers: servo-data-{tenant_id}       │    │
│  │  - Lifecycle: Cool tier after 90 days                   │    │
│  │  - Versioning: Enabled                                  │    │
│  │  - Encryption: Microsoft-managed keys                   │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Scheduling & Triggers                                  │    │
│  │                                                          │    │
│  │  Azure Functions (Timer trigger)                        │    │
│  │  - Cron expressions → Queue messages                    │    │
│  │                                                          │    │
│  │  Event Grid                                             │    │
│  │  - Blob storage events → workflow triggers              │    │
│  │  - Custom events                                        │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Observability                                          │    │
│  │                                                          │    │
│  │  Azure Monitor                                          │    │
│  │  - Log Analytics workspace                              │    │
│  │  - Application Insights                                 │    │
│  │  - Metrics: Custom task execution metrics               │    │
│  │  - Alerts: Failure rate, queue backlog                  │    │
│  └────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌────────────────────────────────────────────────────────┐    │
│  │  Security                                               │    │
│  │                                                          │    │
│  │  Managed Identities                                     │    │
│  │  - servo-worker                                         │    │
│  │    - Roles: Storage Blob Data Contributor,              │    │
│  │             Storage Queue Data Contributor              │    │
│  │                                                          │    │
│  │  Key Vault                                              │    │
│  │  - Database credentials                                 │    │
│  │  - API keys                                             │    │
│  │                                                          │    │
│  │  Virtual Network                                        │    │
│  │  - Private endpoints for PostgreSQL, Storage            │    │
│  │  - Network security groups                              │    │
│  └────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

### 5.4 Multi-Tenant Isolation Patterns

**Storage Isolation**

```python
# Storage path structure
gs://servo-data-{tenant_id}/
├── raw/
│   ├── customers/
│   │   └── 2024-01-01.parquet
│   └── orders/
│       └── 2024-01-01.parquet
├── processed/
│   ├── clean_customers/
│   └── customer_segments/
└── artifacts/
    ├── models/
    └── reports/

# Configuration
@workflow(
    tenant_id="{{ context.tenant_id }}",
    storage_config=StorageConfig(
        prefix="gs://servo-data-{tenant_id}/",
        isolation_level="strict"
    )
)
def tenant_pipeline(context: Context):
    # All asset materializations automatically use tenant prefix
    pass
```

**Compute Isolation**

```python
# Option 1: Separate task queues per tenant
@workflow(
    tenant_id="{{ context.tenant_id }}",
    executor=executor.CloudRun(
        queue_name="servo-{tenant_id}",  # Tenant-specific queue
        max_concurrent=10
    )
)

# Option 2: Shared queue with tenant tags
@workflow(
    tenant_id="{{ context.tenant_id }}",
    executor=executor.CloudRun(
        queue_name="servo-default",
        tags={"tenant_id": "{{ context.tenant_id }}"}
    )
)

# Option 3: Resource quotas via Cloud Scheduler
# Limit tenant to X executions per hour via rate limiting
```

**Metadata Isolation**

```sql
-- Row-level security in PostgreSQL
ALTER TABLE executions ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON executions
    USING (tenant_id = current_setting('app.current_tenant')::uuid);

-- Set tenant context per connection
SET app.current_tenant = 'tenant-uuid-here';

-- Now all queries automatically filter by tenant
SELECT * FROM executions;  -- Only returns current tenant's rows
```

**Cost Attribution**

```python
# Tag all cloud resources with tenant_id
@workflow(
    tenant_id="{{ context.tenant_id }}",
    cost_attribution=CostAttribution(
        labels={
            "tenant_id": "{{ context.tenant_id }}",
            "workflow_name": "{{ workflow.name }}",
            "environment": "production"
        }
    )
)

# Query costs via cloud provider APIs
from servo.billing import CostReport

report = CostReport.generate(
    provider="gcp",
    tenant_id="acme-corp",
    period=("2024-01-01", "2024-01-31")
)

# Returns breakdown:
# - Compute costs (Cloud Run, Cloud Tasks)
# - Storage costs (GCS, CloudSQL)
# - Network egress
# - Total: $XYZ
```

---

## 6. Data Models

### 6.1 PostgreSQL Schema

```sql
-- Schema version: 1.0.0

-- Workflows table
CREATE TABLE workflows (
    workflow_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    description TEXT,
    tenant_id UUID,
    definition JSONB NOT NULL,  -- Serialized workflow definition
    schedule TEXT,  -- Cron expression
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by TEXT,
    is_active BOOLEAN DEFAULT true,
    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_workflows_tenant ON workflows(tenant_id);
CREATE INDEX idx_workflows_active ON workflows(is_active) WHERE is_active = true;

-- Executions table
CREATE TABLE executions (
    execution_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES workflows(workflow_id),
    tenant_id UUID,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled', 'timeout')),
    idempotency_key TEXT UNIQUE,  -- For exactly-once execution
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    duration_seconds DECIMAL,
    metadata JSONB,  -- User-defined metadata
    error TEXT,
    retry_count INTEGER DEFAULT 0,
    parent_execution_id UUID REFERENCES executions(execution_id),  -- For re-runs
    triggered_by TEXT,  -- "schedule", "manual", "event", "cascade"
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_executions_workflow ON executions(workflow_id);
CREATE INDEX idx_executions_tenant ON executions(tenant_id);
CREATE INDEX idx_executions_status ON executions(status);
CREATE INDEX idx_executions_started_at ON executions(started_at DESC);
CREATE INDEX idx_executions_idempotency ON executions(idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Assets table
CREATE TABLE assets (
    asset_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID,
    asset_key TEXT NOT NULL,  -- e.g., "customers.clean_customers"
    asset_type TEXT NOT NULL CHECK (asset_type IN ('table', 'file', 'model', 'report', 'view')),
    location TEXT,  -- GCS path, table reference, etc.
    schema_snapshot JSONB,  -- Column names and types
    partition_key TEXT,  -- e.g., "date"
    last_materialized_at TIMESTAMPTZ,
    last_execution_id UUID REFERENCES executions(execution_id),
    metadata JSONB,  -- Owner, SLA, tags, etc.
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, asset_key)
);

CREATE INDEX idx_assets_tenant ON assets(tenant_id);
CREATE INDEX idx_assets_key ON assets(asset_key);
CREATE INDEX idx_assets_type ON assets(asset_type);
CREATE INDEX idx_assets_last_materialized ON assets(last_materialized_at DESC);

-- Asset dependencies (lineage graph edges)
CREATE TABLE asset_dependencies (
    downstream_asset_id UUID NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    upstream_asset_id UUID NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    last_seen_execution_id UUID REFERENCES executions(execution_id),
    transformation_type TEXT,  -- "join", "filter", "aggregate", etc.
    column_mappings JSONB,  -- For column-level lineage
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (downstream_asset_id, upstream_asset_id)
);

CREATE INDEX idx_deps_upstream ON asset_dependencies(upstream_asset_id);
CREATE INDEX idx_deps_downstream ON asset_dependencies(downstream_asset_id);

-- Task executions table (individual asset executions within a workflow run)
CREATE TABLE task_executions (
    task_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    execution_id UUID NOT NULL REFERENCES executions(execution_id) ON DELETE CASCADE,
    asset_id UUID NOT NULL REFERENCES assets(asset_id),
    tenant_id UUID,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'timeout', 'cancelled')),
    worker_id TEXT,  -- Cloud Run instance ID, Lambda request ID, etc.
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    duration_seconds DECIMAL,
    retry_count INTEGER DEFAULT 0,
    max_retries INTEGER DEFAULT 3,
    error TEXT,
    output_location TEXT,  -- Where the materialized asset is stored
    row_count BIGINT,
    size_bytes BIGINT,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_tasks_execution ON task_executions(execution_id);
CREATE INDEX idx_tasks_asset ON task_executions(asset_id);
CREATE INDEX idx_tasks_status ON task_executions(status);
CREATE INDEX idx_tasks_started_at ON task_executions(started_at DESC);

-- Asset partitions table (for partitioned assets)
CREATE TABLE asset_partitions (
    partition_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    asset_id UUID NOT NULL REFERENCES assets(asset_id) ON DELETE CASCADE,
    partition_key TEXT NOT NULL,  -- e.g., "2024-01-01"
    status TEXT NOT NULL CHECK (status IN ('missing', 'stale', 'fresh')),
    location TEXT,
    last_materialized_at TIMESTAMPTZ,
    last_execution_id UUID REFERENCES executions(execution_id),
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (asset_id, partition_key)
);

CREATE INDEX idx_partitions_asset ON asset_partitions(asset_id);
CREATE INDEX idx_partitions_status ON asset_partitions(status);

-- Execution metrics table (for cost attribution and analytics)
CREATE TABLE execution_metrics (
    metric_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    execution_id UUID REFERENCES executions(execution_id) ON DELETE CASCADE,
    task_id UUID REFERENCES task_executions(task_id) ON DELETE CASCADE,
    tenant_id UUID,
    metric_name TEXT NOT NULL,
    metric_value DECIMAL NOT NULL,
    metric_unit TEXT,  -- "seconds", "bytes", "count", "usd"
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB
);

CREATE INDEX idx_metrics_execution ON execution_metrics(execution_id);
CREATE INDEX idx_metrics_tenant ON execution_metrics(tenant_id);
CREATE INDEX idx_metrics_name ON execution_metrics(metric_name);
CREATE INDEX idx_metrics_timestamp ON execution_metrics(timestamp DESC);

-- Row-level security for multi-tenancy
ALTER TABLE executions ENABLE ROW LEVEL SECURITY;
ALTER TABLE assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE task_executions ENABLE ROW LEVEL SECURITY;
ALTER TABLE execution_metrics ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_executions ON executions
    USING (tenant_id = current_setting('app.current_tenant', true)::uuid OR current_setting('app.current_tenant', true) IS NULL);

CREATE POLICY tenant_isolation_assets ON assets
    USING (tenant_id = current_setting('app.current_tenant', true)::uuid OR current_setting('app.current_tenant', true) IS NULL);

CREATE POLICY tenant_isolation_tasks ON task_executions
    USING (tenant_id = current_setting('app.current_tenant', true)::uuid OR current_setting('app.current_tenant', true) IS NULL);

CREATE POLICY tenant_isolation_metrics ON execution_metrics
    USING (tenant_id = current_setting('app.current_tenant', true)::uuid OR current_setting('app.current_tenant', true) IS NULL);

-- Materialized view for lineage queries (performance optimization)
CREATE MATERIALIZED VIEW asset_lineage_paths AS
WITH RECURSIVE lineage AS (
    SELECT
        downstream_asset_id AS asset_id,
        upstream_asset_id AS related_asset_id,
        1 AS depth,
        ARRAY[downstream_asset_id] AS path
    FROM asset_dependencies
    
    UNION ALL
    
    SELECT
        l.asset_id,
        ad.upstream_asset_id,
        l.depth + 1,
        l.path || ad.upstream_asset_id
    FROM lineage l
    JOIN asset_dependencies ad ON ad.downstream_asset_id = l.related_asset_id
    WHERE l.depth < 10  -- Max depth
      AND NOT (ad.upstream_asset_id = ANY(l.path))  -- Prevent cycles
)
SELECT * FROM lineage;

CREATE INDEX idx_lineage_paths_asset ON asset_lineage_paths(asset_id);
CREATE INDEX idx_lineage_paths_related ON asset_lineage_paths(related_asset_id);

-- Refresh materialized view periodically (via cron or trigger)
-- REFRESH MATERIALIZED VIEW CONCURRENTLY asset_lineage_paths;
```

### 6.2 Key Rust Structs

```rust
// servo-storage/src/models.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Workflow {
    pub workflow_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub tenant_id: Option<Uuid>,
    pub definition: serde_json::Value,
    pub schedule: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Execution {
    pub execution_id: Uuid,
    pub workflow_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub status: ExecutionStatus,
    pub idempotency_key: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub error: Option<String>,
    pub retry_count: i32,
    pub parent_execution_id: Option<Uuid>,
    pub triggered_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Timeout,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ExecutionStatus::Pending => write!(f, "pending"),
            ExecutionStatus::Running => write!(f, "running"),
            ExecutionStatus::Succeeded => write!(f, "succeeded"),
            ExecutionStatus::Failed => write!(f, "failed"),
            ExecutionStatus::Cancelled => write!(f, "cancelled"),
            ExecutionStatus::Timeout => write!(f, "timeout"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Asset {
    pub asset_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub asset_key: String,
    pub asset_type: AssetType,
    pub location: Option<String>,
    pub schema_snapshot: Option<serde_json::Value>,
    pub partition_key: Option<String>,
    pub last_materialized_at: Option<DateTime<Utc>>,
    pub last_execution_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "text")]
pub enum AssetType {
    Table,
    File,
    Model,
    Report,
    View,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TaskExecution {
    pub task_id: Uuid,
    pub execution_id: Uuid,
    pub asset_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub status: ExecutionStatus,
    pub worker_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<f64>,
    pub retry_count: i32,
    pub max_retries: i32,
    pub error: Option<String>,
    pub output_location: Option<String>,
    pub row_count: Option<i64>,
    pub size_bytes: Option<i64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}
```

---

## 7. API Specifications

### 7.1 REST API (Optional Servo UI/API Service)

**Base URL:** `https://servo-api.{domain}/api/v1`

**Authentication:** Bearer token (JWT) or API key

#### **Workflows**

```
POST /workflows
Create or update a workflow

Request:
{
  "name": "daily_etl",
  "description": "Daily customer analytics pipeline",
  "definition": {
    "assets": [...],
    "schedule": "0 2 * * *",
    "executor": {...}
  },
  "tenant_id": "uuid" (optional)
}

Response: 201 Created
{
  "workflow_id": "uuid",
  "name": "daily_etl",
  "created_at": "2024-01-01T00:00:00Z"
}

---

GET /workflows
List all workflows (filtered by tenant)

Query params:
- tenant_id (optional)
- is_active (optional, default: true)
- limit (default: 100)
- offset (default: 0)

Response: 200 OK
{
  "workflows": [...],
  "total": 42,
  "limit": 100,
  "offset": 0
}

---

GET /workflows/{workflow_id}
Get workflow details

Response: 200 OK
{
  "workflow_id": "uuid",
  "name": "daily_etl",
  "definition": {...},
  ...
}

---

DELETE /workflows/{workflow_id}
Delete a workflow (soft delete, sets is_active=false)

Response: 204 No Content
```

#### **Executions**

```
POST /executions
Trigger a workflow execution

Request:
{
  "workflow_id": "uuid",
  "tenant_id": "uuid" (optional),
  "idempotency_key": "string" (optional),
  "params": {
    "date": "2024-01-01",
    "force_recompute": false
  }
}

Response: 202 Accepted
{
  "execution_id": "uuid",
  "status": "pending",
  "created_at": "2024-01-01T00:00:00Z"
}

---

GET /executions/{execution_id}
Get execution status and details

Response: 200 OK
{
  "execution_id": "uuid",
  "workflow_id": "uuid",
  "status": "running",
  "started_at": "2024-01-01T02:00:00Z",
  "tasks": [
    {
      "task_id": "uuid",
      "asset_key": "clean_customers",
      "status": "succeeded",
      "duration_seconds": 12.5
    },
    {
      "task_id": "uuid",
      "asset_key": "customer_segments",
      "status": "running",
      "started_at": "2024-01-01T02:00:15Z"
    }
  ]
}

---

GET /executions
List executions with filtering

Query params:
- workflow_id (optional)
- tenant_id (optional)
- status (optional): pending|running|succeeded|failed
- start_date (optional): ISO 8601
- end_date (optional): ISO 8601
- limit (default: 100)
- offset (default: 0)

Response: 200 OK
{
  "executions": [...],
  "total": 1234,
  "limit": 100,
  "offset": 0
}

---

POST /executions/{execution_id}/cancel
Cancel a running execution

Response: 200 OK
{
  "execution_id": "uuid",
  "status": "cancelled"
}

---

POST /executions/{execution_id}/retry
Retry a failed execution

Response: 202 Accepted
{
  "execution_id": "uuid",  // New execution ID
  "parent_execution_id": "uuid",
  "status": "pending"
}
```

#### **Assets**

```
GET /assets
List assets

Query params:
- tenant_id (optional)
- asset_type (optional): table|file|model|report|view
- search (optional): search by asset_key
- limit (default: 100)
- offset (default: 0)

Response: 200 OK
{
  "assets": [
    {
      "asset_id": "uuid",
      "asset_key": "customers.clean_customers",
      "asset_type": "table",
      "location": "gs://...",
      "last_materialized_at": "2024-01-01T02:05:00Z",
      "metadata": {
        "owner": "data-team@company.com",
        "sla": "4h"
      }
    },
    ...
  ],
  "total": 156,
  "limit": 100,
  "offset": 0
}

---

GET /assets/{asset_id}
Get asset details

Response: 200 OK
{
  "asset_id": "uuid",
  "asset_key": "customers.clean_customers",
  "asset_type": "table",
  "location": "gs://...",
  "schema": {
    "columns": [
      {"name": "customer_id", "type": "INT64"},
      {"name": "email", "type": "STRING"},
      ...
    ]
  },
  "partitions": [
    {
      "partition_key": "2024-01-01",
      "status": "fresh",
      "last_materialized_at": "2024-01-01T02:05:00Z"
    },
    ...
  ],
  "metadata": {...}
}

---

GET /assets/{asset_id}/lineage
Get asset lineage

Query params:
- direction: upstream|downstream (default: both)
- depth: integer (default: 5)

Response: 200 OK
{
  "asset_id": "uuid",
  "upstream": [
    {
      "asset_id": "uuid",
      "asset_key": "raw_customers",
      "depth": 1
    },
    ...
  ],
  "downstream": [
    {
      "asset_id": "uuid",
      "asset_key": "customer_segments",
      "depth": 1
    },
    ...
  ]
}

---

GET /assets/{asset_id}/history
Get materialization history for an asset

Query params:
- limit (default: 100)
- offset (default: 0)

Response: 200 OK
{
  "materializations": [
    {
      "materialized_at": "2024-01-01T02:05:00Z",
      "execution_id": "uuid",
      "row_count": 15234,
      "size_bytes": 1048576
    },
    ...
  ]
}
```

#### **Lineage**

```
GET /lineage
Get full lineage graph (for visualization)

Query params:
- tenant_id (optional)
- asset_keys (optional, comma-separated): filter to specific assets

Response: 200 OK
{
  "nodes": [
    {
      "id": "asset-uuid-1",
      "key": "raw_customers",
      "type": "table",
      "metadata": {...}
    },
    ...
  ],
  "edges": [
    {
      "source": "asset-uuid-1",
      "target": "asset-uuid-2",
      "transformation": "filter"
    },
    ...
  ]
}
```

#### **Metrics**

```
GET /metrics/executions
Get execution metrics

Query params:
- tenant_id (optional)
- workflow_id (optional)
- start_date: ISO 8601
- end_date: ISO 8601
- granularity: hour|day|week|month (default: day)

Response: 200 OK
{
  "metrics": [
    {
      "date": "2024-01-01",
      "total_executions": 24,
      "succeeded": 22,
      "failed": 2,
      "avg_duration_seconds": 145.6
    },
    ...
  ]
}

---

GET /metrics/costs
Get cost metrics per tenant

Query params:
- tenant_id
- start_date: ISO 8601
- end_date: ISO 8601

Response: 200 OK
{
  "tenant_id": "uuid",
  "period": {
    "start": "2024-01-01",
    "end": "2024-01-31"
  },
  "costs": {
    "compute": 234.56,  // USD
    "storage": 45.78,
    "network": 12.34,
    "total": 292.68
  },
  "breakdown_by_workflow": [
    {
      "workflow_name": "daily_etl",
      "cost": 156.23
    },
    ...
  ]
}
```

### 7.2 Python SDK API

**Core APIs:**

```python
from servo import asset, workflow, executor, Schedule

# Define assets
@asset(name="clean_customers")
def clean_customers(raw: Dataset) -> Dataset:
    return raw.drop_duplicates()

# Define workflow
@workflow(
    name="daily_etl",
    schedule=Schedule.cron("0 2 * * *"),
    executor=executor.CloudRun(project="my-project")
)
def daily_pipeline():
    raw = extract_data()
    clean = clean_customers(raw)
    return clean

# Trigger execution
from servo.client import ServoClient

client = ServoClient(api_url="https://servo-api.example.com")
execution = client.trigger_workflow(
    workflow_name="daily_etl",
    params={"date": "2024-01-01"}
)

# Wait for completion
result = execution.wait(timeout=3600)  # 1 hour
print(f"Status: {result.status}")

# Query lineage
from servo.lineage import LineageGraph

graph = LineageGraph.load(client=client)
upstream = graph.upstream_assets("clean_customers", depth=3)
```

---

## 8. Security Architecture

### 8.1 Authentication & Authorization

**Service-to-Service (Cloud Provider IAM)**

```
GCP:
- Workload Identity: servo-worker@project.iam.gserviceaccount.com
- Permissions: CloudSQL client, GCS admin, Cloud Tasks enqueuer

AWS:
- IAM Role: ServoWorkerRole
- Policies: RDS access, S3 read/write, SQS send/receive

Azure:
- Managed Identity: servo-worker
- Roles: PostgreSQL access, Blob Data Contributor, Queue Contributor
```

**User Authentication (Optional UI)**

```
Options:
1. OAuth 2.0 / OpenID Connect
   - Support Google, GitHub, Microsoft providers
   - JWT tokens for API access

2. API Keys
   - For programmatic access
   - Scoped per tenant
   - Rotation policy: 90 days

3. SAML 2.0 (Enterprise)
   - Integration with corporate SSO
   - Requires Servo Cloud or Enterprise edition
```

**Authorization Model**

```python
# Role-based access control (RBAC)
class Role(Enum):
    ADMIN = "admin"          # Full access
    ENGINEER = "engineer"    # Create/run workflows, view lineage
    VIEWER = "viewer"        # Read-only access
    ANALYST = "analyst"      # Run workflows, view data, no modify

# Permissions
class Permission(Enum):
    WORKFLOW_CREATE = "workflow:create"
    WORKFLOW_UPDATE = "workflow:update"
    WORKFLOW_DELETE = "workflow:delete"
    WORKFLOW_EXECUTE = "workflow:execute"
    EXECUTION_VIEW = "execution:view"
    EXECUTION_CANCEL = "execution:cancel"
    ASSET_VIEW = "asset:view"
    LINEAGE_VIEW = "lineage:view"
    METRICS_VIEW = "metrics:view"

# Role-permission mapping
ROLE_PERMISSIONS = {
    Role.ADMIN: [Permission.ALL],
    Role.ENGINEER: [
        Permission.WORKFLOW_CREATE,
        Permission.WORKFLOW_UPDATE,
        Permission.WORKFLOW_EXECUTE,
        Permission.EXECUTION_VIEW,
        Permission.EXECUTION_CANCEL,
        Permission.ASSET_VIEW,
        Permission.LINEAGE_VIEW,
        Permission.METRICS_VIEW,
    ],
    Role.VIEWER: [
        Permission.EXECUTION_VIEW,
        Permission.ASSET_VIEW,
        Permission.LINEAGE_VIEW,
        Permission.METRICS_VIEW,
    ],
    Role.ANALYST: [
        Permission.WORKFLOW_EXECUTE,
        Permission.EXECUTION_VIEW,
        Permission.ASSET_VIEW,
        Permission.LINEAGE_VIEW,
    ],
}
```

### 8.2 Data Encryption

**At Rest:**
- PostgreSQL: Transparent data encryption (TDE) via cloud provider KMS
- Object Storage: Server-side encryption with KMS-managed keys
- Secrets: Stored in Secret Manager / Key Vault

**In Transit:**
- All connections use TLS 1.3
- mTLS for service-to-service communication (optional, recommended)
- Database connections over SSL/TLS

**Secrets Management:**

```python
# GCP example
from google.cloud import secretmanager

def get_db_password():
    client = secretmanager.SecretManagerServiceClient()
    secret_name = f"projects/{PROJECT_ID}/secrets/servo-db-password/versions/latest"
    response = client.access_secret_version(request={"name": secret_name})
    return response.payload.data.decode("UTF-8")

# Configuration
DATABASE_URL = f"postgresql://servo:{get_db_password()}@{DB_HOST}:5432/servo_metadata"
```

### 8.3 Network Security

**Cloud Architecture:**

```
┌─────────────────────────────────────────────────────────────┐
│  Public Internet                                             │
└─────────────────────────────────────────────────────────────┘
                      │
                      │ HTTPS (TLS 1.3)
                      ▼
┌─────────────────────────────────────────────────────────────┐
│  Load Balancer / API Gateway                                │
│  - TLS termination                                          │
│  - DDoS protection (Cloud Armor / WAF)                      │
│  - Rate limiting                                            │
└─────────────────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────────────┐
│  DMZ / Public Subnet                                        │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Cloud Run / ECS / ACI (servo-api)                  │   │
│  │  - Public endpoint for UI/API                       │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                      │
                      │ Private network
                      ▼
┌─────────────────────────────────────────────────────────────┐
│  Private Subnet                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Cloud Run / Lambda / ACI (servo-worker)            │   │
│  │  - No public IP                                     │   │
│  │  - Access via VPC connector                        │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  CloudSQL / RDS / Azure Database                    │   │
│  │  - Private IP only                                  │   │
│  │  - No public endpoint                               │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Firewall Rules:**

```
Ingress:
- Allow HTTPS (443) from Internet → Load Balancer
- Allow Cloud Tasks → Cloud Run (internal only)
- Allow Cloud Run → CloudSQL (internal only, port 5432)

Egress:
- Allow Cloud Run → Internet (for external API calls)
- Allow Cloud Run → GCS (for artifact storage)
- Deny all other egress by default
```

### 8.4 Audit Logging

**What to Log:**

```python
# All actions logged with structured format
{
    "timestamp": "2024-01-01T02:00:00Z",
    "action": "workflow.execute",
    "actor": {
        "type": "user",
        "id": "user-uuid",
        "email": "user@example.com"
    },
    "resource": {
        "type": "workflow",
        "id": "workflow-uuid",
        "name": "daily_etl"
    },
    "context": {
        "tenant_id": "tenant-uuid",
        "ip_address": "1.2.3.4",
        "user_agent": "servo-cli/1.0.0"
    },
    "result": "success",
    "metadata": {
        "execution_id": "execution-uuid"
    }
}
```

**Logged Events:**
- Workflow create/update/delete
- Workflow execution trigger
- Execution status changes
- Asset materialization
- User login/logout
- API key creation/rotation/deletion
- Permission changes

**Retention:**
- Audit logs: 1 year minimum (compliance requirement)
- Application logs: 30 days
- Execution logs: 90 days (configurable)

### 8.5 Compliance

**Standards:**
- SOC 2 Type II (for Servo Cloud)
- GDPR compliant (data retention, right to deletion)
- HIPAA ready (encryption, audit logging, BAA)
- ISO 27001 (information security management)

**Data Residency:**
- Deploy in customer's preferred region
- Option for single-region deployment
- No cross-border data transfer (configurable)

**Right to Deletion:**

```python
# API endpoint for data deletion (GDPR compliance)
@app.delete("/tenants/{tenant_id}")
async def delete_tenant_data(tenant_id: str):
    """
    Delete all data for a tenant.
    - Soft delete: mark as deleted, actual deletion after 30 days
    - Hard delete: immediate removal (only after soft delete period)
    """
    # Mark tenant as deleted
    await db.execute(
        "UPDATE tenants SET deleted_at = NOW() WHERE tenant_id = $1",
        tenant_id
    )
    
    # Schedule hard deletion job (runs after 30 days)
    await scheduler.schedule(
        job="hard_delete_tenant",
        params={"tenant_id": tenant_id},
        run_at=datetime.now() + timedelta(days=30)
    )
    
    return {"status": "scheduled", "hard_delete_at": "..."}
```

---

## 9. Implementation Roadmap

### 9.1 Phase 1: MVP (Months 1-3)

**Goal:** Minimal viable product with GCP support

**Deliverables:**

**Month 1: Core Infrastructure**
- [x] Project setup (Rust workspace, Python package)
- [x] PostgreSQL schema design and migrations
- [ ] Rust core: Workflow struct, Asset struct, Executor trait
- [ ] Python SDK: @asset, @workflow decorators (basic)
- [ ] GCP executor implementation (Cloud Tasks + Cloud Run)
- [ ] CLI for local testing (`servo run workflow.py`)

**Month 2: Execution Engine**
- [ ] Task execution runtime (state machine, retry logic)
- [ ] Lineage tracking (automatic from dependencies)
- [ ] Worker implementation (Cloud Run service)
- [ ] Metadata storage operations (CRUD for workflows, executions, assets)
- [ ] Basic observability (structured logging)

**Month 3: MVP Polish**
- [ ] End-to-end testing (sample workflows)
- [ ] Documentation (README, quickstart, API docs)
- [ ] Terraform modules for GCP deployment
- [ ] Example project (e.g., customer analytics pipeline)
- [ ] Performance testing (1000 concurrent tasks)

**Success Criteria:**
- Deploy a workflow from definition to execution in <10 minutes
- Execute 1000 tasks concurrently
- Lineage graph for 100 assets renders in <1 second
- 5 early adopters successfully deploy

### 9.2 Phase 2: Production-Ready (Months 4-6)

**Goal:** Production-grade features and AWS support

**Deliverables:**

**Month 4: AWS Support**
- [ ] AWS executor (SQS + Lambda/ECS)
- [ ] AWS Terraform modules
- [ ] Cross-cloud testing

**Month 5: Advanced Features**
- [ ] Partition management (daily, hourly, custom)
- [ ] Backfill logic (materialize missing partitions)
- [ ] Advanced retry policies (exponential backoff, circuit breaker)
- [ ] OpenTelemetry integration (distributed tracing)
- [ ] Cost attribution (cloud billing API integration)

**Month 6: Reliability & Scale**
- [ ] Comprehensive error handling
- [ ] Idempotency guarantees
- [ ] Multi-tenant quotas and rate limiting
- [ ] Load testing (10,000 concurrent executions)
- [ ] Chaos engineering tests

**Success Criteria:**
- Support GCP and AWS
- 99.9% execution success rate
- P99 latency <5 seconds for task enqueue
- 50 production deployments

### 9.3 Phase 3: Ecosystem & UI (Months 7-9)

**Goal:** Rich ecosystem and optional UI

**Deliverables:**

**Month 7: Azure Support & Connectors**
- [ ] Azure executor (Queue Storage + Container Instances)
- [ ] dbt connector
- [ ] Airbyte/Fivetran connector
- [ ] Spark connector
- [ ] Connector plugin system

**Month 8: UI Development**
- [ ] React frontend (execution monitoring, asset catalog)
- [ ] Lineage visualization (Cytoscape.js or D3.js)
- [ ] Cost dashboards
- [ ] REST API backend (FastAPI)

**Month 9: Community & Documentation**
- [ ] Comprehensive documentation site
- [ ] Tutorial videos
- [ ] Blog posts (design decisions, case studies)
- [ ] Community forum / Discord
- [ ] Contributor guidelines

**Success Criteria:**
- Support all 3 major clouds (GCP, AWS, Azure)
- 10+ connectors available
- UI deployed for 20+ users
- 100 production deployments, 1000+ GitHub stars

### 9.4 Phase 4: Servo Cloud (Months 10-12)

**Goal:** Managed service launch

**Deliverables:**

**Month 10: Multi-Tenancy Hardening**
- [ ] Tenant management (signup, provisioning)
- [ ] Billing integration (Stripe)
- [ ] Usage metering and quotas
- [ ] Tenant isolation testing

**Month 11: Enterprise Features**
- [ ] SSO (SAML 2.0, OAuth)
- [ ] Advanced RBAC
- [ ] SLA guarantees (99.9% uptime)
- [ ] Dedicated support channels

**Month 12: Launch**
- [ ] Beta program (10 customers)
- [ ] Public launch
- [ ] Marketing (website, docs, demos)
- [ ] Sales pipeline

**Success Criteria:**
- 50 Servo Cloud customers (10 paying)
- $50K MRR
- 99.9% uptime SLA met
- NPS > 40

---

## 10. Success Metrics

### 10.1 Adoption Metrics

**GitHub Activity:**
- Stars: 1000 in Year 1, 5000 in Year 2
- Forks: 100 in Year 1, 500 in Year 2
- Contributors: 20 active in Year 1, 50 in Year 2
- Issues resolved: 90% within 7 days

**Deployments:**
- Production deployments: 100 in Year 1, 500 in Year 2
- Active users: 500 in Year 1, 2500 in Year 2
- Workflows deployed: 1000 in Year 1, 10000 in Year 2

**Community:**
- Discord/Slack members: 500 in Year 1, 2000 in Year 2
- Stack Overflow questions: 100 in Year 1, 500 in Year 2
- Blog post mentions: 50 in Year 1, 200 in Year 2

### 10.2 Technical Metrics

**Performance:**
- Deploy-to-first-execution: <10 minutes
- Cold start latency: <100ms (Rust worker)
- Lineage query (1000 assets): <100ms
- Task enqueue latency (P99): <500ms
- Task execution latency (P50): <5 seconds

**Reliability:**
- Execution success rate: >99.9%
- Uptime (Servo Cloud): 99.9%
- Data loss: 0 incidents
- Security vulnerabilities: 0 critical

**Scalability:**
- Concurrent executions supported: 10,000+
- Assets in lineage graph: 10,000+
- Tenants per cluster: 1000+
- Tasks per workflow: 1000+

### 10.3 Business Metrics (Servo Cloud)

**Revenue:**
- Year 1: $500K ARR
- Year 2: $2M ARR
- Gross margin: >80%

**Customer Metrics:**
- Paying customers: 50 in Year 1, 200 in Year 2
- Churn rate: <10% monthly
- Net Promoter Score (NPS): >40
- Customer acquisition cost (CAC): <$10K
- Lifetime value (LTV): >$100K

**Cost Efficiency:**
- Total cost of ownership: 50% lower than Dagster/Airflow
- Cost per execution: <$0.01 (at scale)
- Infrastructure costs: <20% of revenue

---

## 11. Risk Assessment

### 11.1 Technical Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| **Rust complexity slows development** | High | Medium | Start with simpler Rust modules, use Python for rapid prototyping, hire experienced Rust developers |
| **PyO3 bindings have performance issues** | Medium | Low | Profile early, optimize hot paths, consider alternative FFI approaches if needed |
| **Cloud provider API changes break adapters** | Medium | Low | Version pin dependencies, comprehensive integration tests, maintain compatibility layer |
| **Scalability issues at 10K+ concurrent executions** | High | Medium | Load testing early, horizontal scaling via multiple workers, connection pooling |
| **PostgreSQL becomes bottleneck** | Medium | Medium | Read replicas for queries, connection pooling, denormalized views for lineage |

### 11.2 Product Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| **Market doesn't adopt asset-centric approach** | High | Low | Validate with early users, emphasize lineage benefits, provide task-centric fallback |
| **Dagster/Airflow add serverless support** | Medium | Medium | Focus on multi-tenancy differentiation, maintain performance advantage, iterate faster |
| **Competitors offer managed services** | High | High | Launch Servo Cloud quickly, provide superior UX, competitive pricing |
| **Users prefer Python-only solutions** | Low | Low | Highlight performance benefits, provide seamless Python SDK, hide Rust complexity |

### 11.3 Operational Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| **Open-source maintenance burden** | Medium | High | Build active community, designate maintainers, automate CI/CD, clear contribution guidelines |
| **Security vulnerabilities** | High | Medium | Regular security audits, dependency scanning, bug bounty program, rapid patching |
| **Data loss in customer deployments** | High | Low | Automated backups, point-in-time recovery, comprehensive testing, disaster recovery procedures |
| **Vendor lock-in concerns** | Medium | Low | Emphasize cloud-agnostic design, provide migration tools, support hybrid deployments |

### 11.4 Business Risks

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| **Insufficient funding for development** | High | Low | Phased development, open-source first, monetize via Servo Cloud, seek VC funding if needed |
| **Slow customer acquisition** | Medium | Medium | Strong developer marketing, community building, free tier, content marketing |
| **High cloud infrastructure costs** | Medium | Low | Optimize resource usage, negotiate cloud credits, pass costs to customers transparently |
| **Team scaling challenges** | Medium | Medium | Hire incrementally, focus on high-leverage areas, use contractors for specialized work |

---

## 12. Appendices

### 12.1 Technology Stack

**Rust Crates:**
- **tokio**: Async runtime
- **sqlx**: PostgreSQL driver
- **serde**: Serialization
- **petgraph**: Graph algorithms (lineage)
- **pyo3**: Python bindings
- **clap**: CLI parsing
- **tracing**: Structured logging
- **uuid**: UUID generation
- **chrono**: Date/time handling

**Python Libraries:**
- **fastapi**: REST API (optional UI backend)
- **uvicorn**: ASGI server
- **sqlalchemy**: ORM (if needed)
- **pandas**: Data manipulation
- **pydantic**: Data validation
- **click**: CLI framework
- **pytest**: Testing

**Cloud SDKs:**
- **google-cloud-tasks**: GCP Cloud Tasks
- **google-cloud-run**: GCP Cloud Run
- **boto3**: AWS SDK
- **azure-sdk**: Azure SDK

**Frontend (Optional UI):**
- **React**: UI framework
- **TypeScript**: Type safety
- **Tailwind CSS**: Styling
- **Cytoscape.js** or **D3.js**: Lineage visualization
- **Recharts**: Metrics charts

### 12.2 Development Environment Setup

```bash
# Prerequisites
# - Rust 1.75+
# - Python 3.11+
# - PostgreSQL 15+
# - Docker

# Clone repository
git clone https://github.com/servo-orchestration/servo.git
cd servo

# Install Rust dependencies
cargo build

# Install Python dependencies
cd servo-python
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
pip install -e .

# Setup local PostgreSQL
docker run -d \
  --name servo-postgres \
  -e POSTGRES_PASSWORD=servo \
  -e POSTGRES_USER=servo \
  -e POSTGRES_DB=servo_metadata \
  -p 5432:5432 \
  postgres:15

# Run migrations
cargo run --bin servo-cli -- migrate up

# Run tests
cargo test
pytest

# Start local worker (for testing)
cargo run --bin servo-worker

# Deploy sample workflow
cd examples/simple-etl
servo deploy workflow.py --executor local
servo run daily_pipeline
```

### 12.3 Example Workflow

```python
# examples/customer-analytics/workflow.py

from servo import asset, workflow, executor, Schedule, RetryPolicy
import pandas as pd

@asset(
    name="raw_customers",
    description="Raw customer data from API",
    retry=RetryPolicy(max_attempts=3)
)
def extract_customers() -> pd.DataFrame:
    """Extract customer data from external API."""
    response = requests.get("https://api.example.com/customers")
    return pd.DataFrame(response.json())

@asset(
    name="clean_customers",
    dependencies=["raw_customers"],
    description="Cleaned and deduplicated customer records",
)
def clean_customers(raw: pd.DataFrame) -> pd.DataFrame:
    """Clean and deduplicate customer data."""
    return (
        raw
        .drop_duplicates(subset=["customer_id"])
        .fillna({"email": "", "phone": ""})
        .query("customer_id > 0")
    )

@asset(
    name="customer_segments",
    dependencies=["clean_customers"],
    description="Customer segmentation using K-means clustering",
)
def segment_customers(clean: pd.DataFrame) -> pd.DataFrame:
    """Segment customers using K-means clustering."""
    from sklearn.cluster import KMeans
    
    features = clean[["age", "purchase_frequency", "lifetime_value"]]
    kmeans = KMeans(n_clusters=5, random_state=42)
    clean["segment"] = kmeans.fit_predict(features)
    
    return clean

@asset(
    name="customer_report",
    dependencies=["customer_segments"],
    description="Summary report of customer segments",
)
def create_report(segments: pd.DataFrame) -> pd.DataFrame:
    """Create summary report by segment."""
    return segments.groupby("segment").agg({
        "customer_id": "count",
        "lifetime_value": ["mean", "sum"],
        "purchase_frequency": "mean"
    })

@workflow(
    name="customer_analytics",
    description="Daily customer analytics pipeline",
    schedule=Schedule.cron("0 2 * * *"),  # Daily at 2 AM
    executor=executor.CloudRun(
        project="my-project",
        region="us-central1"
    )
)
def daily_customer_pipeline():
    """Main workflow: extract, clean, segment, and report."""
    raw = extract_customers()
    clean = clean_customers(raw)
    segments = segment_customers(clean)
    report = create_report(segments)
    return report

if __name__ == "__main__":
    # For local testing
    result = daily_customer_pipeline()
    print(result)
```

### 12.4 Glossary

- **Asset**: A persistent data artifact (table, file, model, report)
- **Workflow**: A collection of assets with defined execution order
- **Execution**: A single run of a workflow
- **Task**: Execution of a single asset within a workflow run
- **Lineage**: Graph of dependencies between assets
- **Materialization**: The act of computing and storing an asset
- **Partition**: A subset of an asset (e.g., daily partition of a table)
- **Executor**: Backend for running tasks (Cloud Run, Lambda, ECS)
- **Idempotency**: Property that allows safe retries without side effects
- **Multi-tenancy**: Support for isolated execution contexts per customer
- **Serverless**: Compute model where infrastructure scales to zero when idle

---

## Document Changelog

**v1.0 - November 2024**
- Initial project design document
- Core architecture and infrastructure designs
- Implementation roadmap and success metrics

---

**End of Document**

---

This document provides a comprehensive blueprint for building Servo. Next steps:
1. Review and approve architecture decisions
2. Set up development environment
3. Begin Phase 1 implementation (MVP)
4. Iterate based on early feedback

Questions or suggestions? Open an issue on GitHub or reach out to the team.