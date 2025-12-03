# Servo: Supplementary Design Document (Continued)
## Sections 5-11

---

## 5. Security Hardening

### 5.1 Defense in Depth

**Security Layers:**

```
┌─────────────────────────────────────────────────────────┐
│ Layer 7: Application Security                          │
│ - Input validation                                      │
│ - SQL injection prevention                              │
│ - XSS protection                                        │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 6: Authentication & Authorization                 │
│ - OAuth 2.0 / JWT tokens                                │
│ - RBAC (Role-Based Access Control)                      │
│ - Multi-factor authentication                           │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 5: Data Encryption                                │
│ - TLS 1.3 in transit                                    │
│ - AES-256 at rest                                       │
│ - Field-level encryption for PII                        │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 4: Network Security                               │
│ - VPC isolation                                         │
│ - Security groups / Firewall rules                      │
│ - Private subnets for databases                         │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 3: Infrastructure Security                        │
│ - Workload Identity / IAM roles                         │
│ - Least privilege principle                             │
│ - Resource tagging for audit                            │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 2: Runtime Security                               │
│ - Container image scanning                              │
│ - Read-only file systems                                │
│ - Non-root users                                        │
└─────────────────────────────────────────────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 1: Audit & Monitoring                             │
│ - Comprehensive audit logs                              │
│ - Anomaly detection                                     │
│ - Security incident response                            │
└─────────────────────────────────────────────────────────┘
```

### 5.2 Input Validation and Sanitization

```rust
// servo-core/src/validation.rs

use validator::{Validate, ValidationError};
use regex::Regex;

#[derive(Debug, Clone, Validate)]
pub struct WorkflowInput {
    #[validate(length(min = 1, max = 255))]
    #[validate(regex = "NAME_PATTERN")]
    pub name: String,
    
    #[validate(length(max = 1000))]
    pub description: Option<String>,
    
    #[validate]
    pub assets: Vec<AssetInput>,
    
    #[validate(custom = "validate_schedule")]
    pub schedule: Option<String>,
}

lazy_static! {
    static ref NAME_PATTERN: Regex = Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
}

fn validate_schedule(schedule: &str) -> Result<(), ValidationError> {
    // Validate cron expression
    cron::Schedule::from_str(schedule)
        .map(|_| ())
        .map_err(|e| {
            let mut err = ValidationError::new("invalid_cron");
            err.message = Some(format!("Invalid cron expression: {}", e).into());
            err
        })
}

#[derive(Debug, Clone, Validate)]
pub struct AssetInput {
    #[validate(length(min = 1, max = 255))]
    #[validate(regex = "NAME_PATTERN")]
    pub key: String,
    
    #[validate]
    pub dependencies: Vec<String>,
    
    #[validate(custom = "validate_no_circular_deps")]
    pub compute_fn: ComputeFnInput,
}

// Prevent SQL injection in queries
pub fn sanitize_sql_identifier(identifier: &str) -> Result<String> {
    // Only allow alphanumeric and underscore
    if !identifier.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(Error::InvalidIdentifier(identifier.to_string()));
    }
    
    // Prevent reserved keywords
    let reserved = ["SELECT", "INSERT", "UPDATE", "DELETE", "DROP", "TABLE"];
    if reserved.contains(&identifier.to_uppercase().as_str()) {
        return Err(Error::ReservedKeyword(identifier.to_string()));
    }
    
    Ok(identifier.to_string())
}

// Prevent command injection
pub fn sanitize_shell_arg(arg: &str) -> Result<String> {
    // Use shellwords crate for proper escaping
    Ok(shellwords::escape(arg))
}
```

### 5.3 SQL Injection Prevention

```rust
// ALWAYS use parameterized queries with sqlx

// ❌ NEVER DO THIS (vulnerable to SQL injection)
let query = format!("SELECT * FROM assets WHERE asset_key = '{}'", user_input);

// ✅ ALWAYS DO THIS (parameterized query)
let result = sqlx::query_as!(
    Asset,
    r#"
    SELECT * FROM assets WHERE asset_key = $1
    "#,
    user_input
)
.fetch_one(&pool)
.await?;

// For dynamic queries, use sqlx::QueryBuilder
let mut query_builder = sqlx::QueryBuilder::new(
    "SELECT * FROM assets WHERE 1=1"
);

if let Some(tenant_id) = filters.tenant_id {
    query_builder.push(" AND tenant_id = ");
    query_builder.push_bind(tenant_id);
}

if let Some(asset_type) = filters.asset_type {
    query_builder.push(" AND asset_type = ");
    query_builder.push_bind(asset_type);
}

let assets = query_builder
    .build_query_as::<Asset>()
    .fetch_all(&pool)
    .await?;
```

### 5.4 Rate Limiting

```rust
// servo-api/src/rate_limit.rs

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    limits: Arc<RwLock<HashMap<String, RateLimitState>>>,
    config: RateLimitConfig,
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub requests_per_hour: u32,
    pub burst_size: u32,
}

#[derive(Debug)]
struct RateLimitState {
    tokens: u32,
    last_refill: Instant,
    minute_count: u32,
    hour_count: u32,
    minute_window_start: Instant,
    hour_window_start: Instant,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            limits: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }
    
    pub async fn check_rate_limit(&self, key: &str) -> Result<(), RateLimitError> {
        let mut limits = self.limits.write().await;
        let now = Instant::now();
        
        let state = limits.entry(key.to_string()).or_insert_with(|| {
            RateLimitState {
                tokens: self.config.burst_size,
                last_refill: now,
                minute_count: 0,
                hour_count: 0,
                minute_window_start: now,
                hour_window_start: now,
            }
        });
        
        // Refill tokens (token bucket algorithm)
        let elapsed = now.duration_since(state.last_refill);
        let tokens_to_add = (elapsed.as_secs_f64() / 60.0 
            * self.config.requests_per_minute as f64) as u32;
        
        if tokens_to_add > 0 {
            state.tokens = (state.tokens + tokens_to_add).min(self.config.burst_size);
            state.last_refill = now;
        }
        
        // Check minute window
        if now.duration_since(state.minute_window_start) >= Duration::from_secs(60) {
            state.minute_count = 0;
            state.minute_window_start = now;
        }
        
        // Check hour window
        if now.duration_since(state.hour_window_start) >= Duration::from_secs(3600) {
            state.hour_count = 0;
            state.hour_window_start = now;
        }
        
        // Check limits
        if state.tokens == 0 {
            return Err(RateLimitError::BurstLimitExceeded);
        }
        
        if state.minute_count >= self.config.requests_per_minute {
            return Err(RateLimitError::MinuteLimitExceeded {
                retry_after: 60 - now.duration_since(state.minute_window_start).as_secs(),
            });
        }
        
        if state.hour_count >= self.config.requests_per_hour {
            return Err(RateLimitError::HourLimitExceeded {
                retry_after: 3600 - now.duration_since(state.hour_window_start).as_secs(),
            });
        }
        
        // Consume token
        state.tokens -= 1;
        state.minute_count += 1;
        state.hour_count += 1;
        
        Ok(())
    }
}

// Middleware for API
#[derive(Debug)]
pub enum RateLimitError {
    BurstLimitExceeded,
    MinuteLimitExceeded { retry_after: u64 },
    HourLimitExceeded { retry_after: u64 },
}

// Usage in API handler
async fn api_handler(
    req: Request,
    rate_limiter: Arc<RateLimiter>,
) -> Result<Response> {
    // Extract identifier (API key, IP address, tenant ID)
    let identifier = req.headers()
        .get("X-API-Key")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_else(|| {
            // Fallback to IP address
            req.peer_addr().map(|a| a.to_string()).unwrap_or_default()
        });
    
    // Check rate limit
    match rate_limiter.check_rate_limit(&identifier).await {
        Ok(_) => {
            // Process request normally
            handle_request(req).await
        }
        Err(RateLimitError::MinuteLimitExceeded { retry_after }) => {
            Ok(Response::builder()
                .status(429)
                .header("Retry-After", retry_after)
                .body("Rate limit exceeded. Please retry after indicated time.".into())?)
        }
        Err(_) => {
            Ok(Response::builder()
                .status(429)
                .body("Rate limit exceeded".into())?)
        }
    }
}
```

### 5.5 Secure Container Images

**Dockerfile Security Best Practices:**

```dockerfile
# Use specific version tags, not 'latest'
FROM rust:1.75-slim as builder

# Run as non-root user
RUN useradd -m -u 1000 servo

# Copy only necessary files
WORKDIR /app
COPY --chown=servo:servo Cargo.toml Cargo.lock ./
COPY --chown=servo:servo servo-core ./servo-core
COPY --chown=servo:servo servo-runtime ./servo-runtime

# Build with security flags
ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --release

# Runtime stage
FROM gcr.io/distroless/cc-debian12

# Copy binary from builder
COPY --from=builder /app/target/release/servo-worker /usr/local/bin/

# Use non-root user
USER 1000:1000

# Read-only root filesystem
# (configured in Kubernetes/Cloud Run deployment)

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/usr/local/bin/servo-worker", "health"]

ENTRYPOINT ["/usr/local/bin/servo-worker"]
```

**Image Scanning in CI:**

```yaml
# .github/workflows/security-scan.yml

name: Security Scan

on:
  push:
    branches: [main, develop]
  pull_request:

jobs:
  scan-image:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Build Docker image
        run: docker build -t servo-worker:${{ github.sha }} .
      
      - name: Run Trivy vulnerability scanner
        uses: aquasecurity/trivy-action@master
        with:
          image-ref: 'servo-worker:${{ github.sha }}'
          format: 'sarif'
          output: 'trivy-results.sarif'
          severity: 'CRITICAL,HIGH'
      
      - name: Upload Trivy results to GitHub Security
        uses: github/codeql-action/upload-sarif@v2
        with:
          sarif_file: 'trivy-results.sarif'
      
      - name: Fail on high/critical vulnerabilities
        uses: aquasecurity/trivy-action@master
        with:
          image-ref: 'servo-worker:${{ github.sha }}'
          exit-code: '1'
          severity: 'CRITICAL,HIGH'
```

### 5.6 Secrets Rotation

```rust
// servo-core/src/secrets/rotation.rs

use chrono::{DateTime, Utc, Duration};

pub struct SecretRotationManager {
    secrets_backend: Arc<dyn SecretsBackend>,
    rotation_policy: RotationPolicy,
}

#[derive(Debug, Clone)]
pub struct RotationPolicy {
    pub rotation_period: Duration,
    pub grace_period: Duration,  // Old secret still valid during grace period
    pub notify_before: Duration, // Notify X days before rotation
}

impl SecretRotationManager {
    pub async fn rotate_database_password(&self) -> Result<()> {
        tracing::info!("Starting database password rotation");
        
        // Generate new password
        let new_password = generate_secure_password();
        
        // Store new password with version suffix
        let secret_name = "servo-db-password-new";
        self.secrets_backend
            .set_secret(secret_name, &new_password)
            .await?;
        
        // Update database user password
        let pool = get_admin_db_pool().await?;
        sqlx::query("ALTER USER servo WITH PASSWORD $1")
            .bind(&new_password)
            .execute(&pool)
            .await?;
        
        // Wait for grace period to allow running processes to finish
        tracing::info!("Waiting grace period before promoting new password");
        tokio::time::sleep(self.rotation_policy.grace_period.to_std()?).await;
        
        // Promote new password to primary
        let old_password = self.secrets_backend
            .get_secret("servo-db-password")
            .await?;
        
        self.secrets_backend
            .set_secret("servo-db-password-old", &old_password)
            .await?;
        
        self.secrets_backend
            .set_secret("servo-db-password", &new_password)
            .await?;
        
        // Delete temporary secret
        self.secrets_backend
            .delete_secret(secret_name)
            .await?;
        
        tracing::info!("Database password rotation completed");
        
        Ok(())
    }
    
    pub async fn check_rotation_needed(&self, secret_name: &str) -> Result<bool> {
        // Check last rotation timestamp
        let metadata_key = format!("{}-last-rotated", secret_name);
        
        let last_rotated: DateTime<Utc> = match self.secrets_backend
            .get_secret(&metadata_key)
            .await
        {
            Ok(timestamp) => timestamp.parse()?,
            Err(_) => {
                // Never rotated, needs rotation
                return Ok(true);
            }
        };
        
        let time_since_rotation = Utc::now() - last_rotated;
        
        Ok(time_since_rotation > self.rotation_policy.rotation_period)
    }
}

fn generate_secure_password() -> String {
    use rand::Rng;
    use rand::distributions::Alphanumeric;
    
    // Generate 32-character password with special characters
    let mut rng = rand::thread_rng();
    let alphanumeric: String = (0..24)
        .map(|_| rng.sample(Alphanumeric) as char)
        .collect();
    
    let special: String = (0..8)
        .map(|_| {
            let specials = "!@#$%^&*()-_=+[]{}|;:,.<>?";
            specials.chars().nth(rng.gen_range(0..specials.len())).unwrap()
        })
        .collect();
    
    format!("{}{}", alphanumeric, special)
}
```

---

## 6. Cost Monitoring and Optimization

### 6.1 Cost Attribution

**Tagging Strategy:**

```rust
// servo-cloud-gcp/src/cost.rs

pub struct CostTagger {
    project: String,
}

impl CostTagger {
    pub fn generate_tags(&self, context: &ExecutionContext) -> HashMap<String, String> {
        let mut tags = HashMap::new();
        
        // Core identifiers
        tags.insert("project".to_string(), self.project.clone());
        tags.insert("service".to_string(), "servo".to_string());
        tags.insert("component".to_string(), "worker".to_string());
        
        // Tenant identification
        if let Some(tenant_id) = &context.tenant_id {
            tags.insert("tenant_id".to_string(), tenant_id.to_string());
        }
        
        // Workflow identification
        tags.insert("workflow_id".to_string(), context.workflow_id.to_string());
        tags.insert("workflow_name".to_string(), context.workflow_name.clone());
        
        // Execution identification
        tags.insert("execution_id".to_string(), context.execution_id.to_string());
        
        // Environment
        tags.insert("environment".to_string(), 
            std::env::var("ENVIRONMENT").unwrap_or_else(|_| "production".to_string()));
        
        // Cost center (if provided)
        if let Some(cost_center) = context.metadata.get("cost_center") {
            tags.insert("cost_center".to_string(), cost_center.clone());
        }
        
        tags
    }
    
    pub fn apply_tags_to_cloud_run(&self, tags: &HashMap<String, String>) -> CloudRunLabels {
        // GCP labels have restrictions: lowercase, max 63 chars
        tags.iter()
            .map(|(k, v)| {
                let key = k.to_lowercase().chars().take(63).collect();
                let value = v.to_lowercase().chars().take(63).collect();
                (key, value)
            })
            .collect()
    }
}
```

**Cost Tracking in Execution:**

```rust
// servo-runtime/src/cost_tracking.rs

use rust_decimal::Decimal;

pub struct CostTracker {
    storage: Arc<dyn MetadataStore>,
    pricing: CloudPricing,
}

#[derive(Debug, Clone)]
pub struct CloudPricing {
    pub compute_per_vcpu_second: Decimal,
    pub memory_per_gb_second: Decimal,
    pub network_egress_per_gb: Decimal,
    pub storage_per_gb_month: Decimal,
}

impl CostTracker {
    pub async fn record_execution_cost(
        &self,
        execution_id: Uuid,
        metrics: &ExecutionMetrics,
    ) -> Result<Decimal> {
        // Calculate compute cost
        let compute_cost = self.calculate_compute_cost(metrics);
        
        // Calculate storage cost (prorated)
        let storage_cost = self.calculate_storage_cost(metrics);
        
        // Calculate network cost
        let network_cost = self.calculate_network_cost(metrics);
        
        let total_cost = compute_cost + storage_cost + network_cost;
        
        // Store in database
        sqlx::query!(
            r#"
            INSERT INTO execution_costs (
                execution_id, compute_cost, storage_cost, network_cost, total_cost
            ) VALUES ($1, $2, $3, $4, $5)
            "#,
            execution_id,
            compute_cost,
            storage_cost,
            network_cost,
            total_cost
        )
        .execute(&self.storage.pool())
        .await?;
        
        Ok(total_cost)
    }
    
    fn calculate_compute_cost(&self, metrics: &ExecutionMetrics) -> Decimal {
        // Cloud Run: charged per vCPU-second and GB-second
        let vcpu_seconds = Decimal::from(metrics.cpu_vcpu) 
            * Decimal::from_f64(metrics.duration.as_secs_f64()).unwrap();
        
        let memory_gb_seconds = Decimal::from_f64(metrics.memory_gb).unwrap()
            * Decimal::from_f64(metrics.duration.as_secs_f64()).unwrap();
        
        (vcpu_seconds * self.pricing.compute_per_vcpu_second)
            + (memory_gb_seconds * self.pricing.memory_per_gb_second)
    }
    
    fn calculate_storage_cost(&self, metrics: &ExecutionMetrics) -> Decimal {
        // Prorated storage cost
        let storage_gb = Decimal::from_f64(metrics.output_size_bytes as f64 / 1_073_741_824.0).unwrap();
        let hours_in_month = Decimal::from(730); // Average
        let duration_hours = Decimal::from_f64(metrics.duration.as_secs_f64() / 3600.0).unwrap();
        
        storage_gb * self.pricing.storage_per_gb_month * (duration_hours / hours_in_month)
    }
    
    fn calculate_network_cost(&self, metrics: &ExecutionMetrics) -> Decimal {
        let egress_gb = Decimal::from_f64(metrics.network_egress_bytes as f64 / 1_073_741_824.0).unwrap();
        egress_gb * self.pricing.network_egress_per_gb
    }
}

#[derive(Debug)]
pub struct ExecutionMetrics {
    pub duration: Duration,
    pub cpu_vcpu: u32,
    pub memory_gb: f64,
    pub output_size_bytes: u64,
    pub network_egress_bytes: u64,
}
```

### 6.2 Cost Alerting

```rust
// servo-core/src/cost_alerts.rs

pub struct CostAlertManager {
    storage: Arc<dyn MetadataStore>,
    notification: Arc<dyn NotificationService>,
}

#[derive(Debug, Clone)]
pub struct CostAlert {
    pub alert_type: CostAlertType,
    pub threshold: Decimal,
    pub period: Duration,
    pub notification_channels: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum CostAlertType {
    TenantHourlySpend,
    TenantDailySpend,
    TenantMonthlySpend,
    WorkflowExecutionCost,
    TotalPlatformCost,
}

impl CostAlertManager {
    pub async fn check_alerts(&self) -> Result<()> {
        // Check tenant hourly spend
        let tenants = self.get_active_tenants().await?;
        
        for tenant_id in tenants {
            let hourly_spend = self.get_tenant_spend(
                &tenant_id,
                Utc::now() - chrono::Duration::hours(1),
                Utc::now()
            ).await?;
            
            if hourly_spend > Decimal::from(100) {  // $100/hour threshold
                self.send_alert(
                    &tenant_id,
                    CostAlertType::TenantHourlySpend,
                    hourly_spend
                ).await?;
            }
        }
        
        Ok(())
    }
    
    async fn send_alert(
        &self,
        tenant_id: &Uuid,
        alert_type: CostAlertType,
        amount: Decimal,
    ) -> Result<()> {
        let message = format!(
            "Cost alert for tenant {}: {:?} = ${}",
            tenant_id, alert_type, amount
        );
        
        self.notification.send_slack(
            "#cost-alerts",
            &message
        ).await?;
        
        self.notification.send_email(
            &self.get_tenant_admin_email(tenant_id).await?,
            "Servo Cost Alert",
            &message
        ).await?;
        
        Ok(())
    }
}
```

### 6.3 Cost Optimization Strategies

**1. Spot Instances for Non-Critical Workloads:**

```rust
// servo-core/src/execution.rs

#[derive(Debug, Clone)]
pub struct ExecutionPolicy {
    pub priority: Priority,
    pub use_spot_instances: bool,
    pub max_execution_time: Duration,
}

#[derive(Debug, Clone)]
pub enum Priority {
    Critical,   // Run on regular instances, never preempt
    Standard,   // Can use spot if available
    Low,        // Always use spot, can wait in queue
}

impl ExecutionPolicy {
    pub fn should_use_spot(&self) -> bool {
        match self.priority {
            Priority::Critical => false,
            Priority::Standard => self.use_spot_instances,
            Priority::Low => true,
        }
    }
}
```

**2. Intelligent Caching:**

```rust
// servo-runtime/src/cache.rs

pub struct AssetCache {
    storage: Arc<dyn CacheBackend>,
}

impl AssetCache {
    pub async fn get_cached_asset(
        &self,
        asset_key: &str,
        dependencies_hash: &str,
    ) -> Option<CachedAsset> {
        let cache_key = format!("{}:{}", asset_key, dependencies_hash);
        
        self.storage.get(&cache_key).await.ok()
    }
    
    pub async fn should_recompute(
        &self,
        asset: &Asset,
        execution_policy: &ExecutionPolicy,
    ) -> bool {
        // Check if asset is fresh enough
        if let Some(cached) = self.get_cached_asset(
            &asset.key,
            &self.compute_dependencies_hash(asset)
        ).await {
            let age = Utc::now() - cached.materialized_at;
            
            // If asset has freshness policy, respect it
            if let Some(max_age) = asset.metadata.freshness_policy.max_age {
                return age > max_age;
            }
            
            // For low-priority executions, use cached data more aggressively
            if execution_policy.priority == Priority::Low {
                return age > chrono::Duration::hours(24);
            }
        }
        
        true // No cache or too old
    }
}
```

**3. Automatic Resource Rightsizing:**

```rust
// servo-runtime/src/resource_optimizer.rs

pub struct ResourceOptimizer {
    storage: Arc<dyn MetadataStore>,
}

impl ResourceOptimizer {
    pub async fn recommend_resources(
        &self,
        asset_key: &str,
    ) -> Result<ResourceRecommendation> {
        // Analyze past executions
        let past_executions = self.storage
            .get_asset_execution_history(asset_key, 100)
            .await?;
        
        if past_executions.is_empty() {
            // No history, use defaults
            return Ok(ResourceRecommendation::default());
        }
        
        // Calculate P95 usage
        let mut cpu_usages: Vec<f64> = past_executions
            .iter()
            .map(|e| e.metrics.cpu_usage)
            .collect();
        cpu_usages.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95_cpu = cpu_usages[(cpu_usages.len() as f64 * 0.95) as usize];
        
        let mut memory_usages: Vec<f64> = past_executions
            .iter()
            .map(|e| e.metrics.memory_usage_gb)
            .collect();
        memory_usages.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95_memory = memory_usages[(memory_usages.len() as f64 * 0.95) as usize];
        
        // Add 20% headroom
        let recommended_cpu = (p95_cpu * 1.2).ceil() as u32;
        let recommended_memory = (p95_memory * 1.2).ceil();
        
        Ok(ResourceRecommendation {
            cpu_vcpu: recommended_cpu,
            memory_gb: recommended_memory,
            confidence: if past_executions.len() >= 50 { 0.9 } else { 0.6 },
        })
    }
}

#[derive(Debug)]
pub struct ResourceRecommendation {
    pub cpu_vcpu: u32,
    pub memory_gb: f64,
    pub confidence: f64,  // 0.0 - 1.0
}
```

---

## 7. High Availability and Disaster Recovery

### 7.1 Multi-Region Architecture

```
┌─────────────────────────────────────────────────────────┐
│ Global Load Balancer                                     │
│ - Health checks                                          │
│ - Traffic routing (latency-based)                        │
└─────────────────────────────────────────────────────────┘
                    │                    │
        ┌───────────┴────────┐  ┌───────┴────────────┐
        ▼                     ▼  ▼                    ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│ Region: us-west1 │  │ Region: us-east1 │  │ Region: eu-west1 │
│                  │  │                  │  │                  │
│ ┌──────────────┐│  │ ┌──────────────┐│  │ ┌──────────────┐│
│ │ API Service  ││  │ │ API Service  ││  │ │ API Service  ││
│ └──────────────┘│  │ └──────────────┘│  │ └──────────────┘│
│                  │  │                  │  │                  │
│ ┌──────────────┐│  │ ┌──────────────┐│  │ ┌──────────────┐│
│ │ Workers      ││  │ │ Workers      ││  │ │ Workers      ││
│ └──────────────┘│  │ └──────────────┘│  │ └──────────────┘│
│                  │  │                  │  │                  │
│ ┌──────────────┐│  │ ┌──────────────┐│  │ ┌──────────────┐│
│ │ Cloud Tasks  ││  │ │ Cloud Tasks  ││  │ │ Cloud Tasks  ││
│ └──────────────┘│  │ └──────────────┘│  │ └──────────────┘│
└──────────────────┘  └──────────────────┘  └──────────────────┘
        │                     │                      │
        └─────────────────────┴──────────────────────┘
                              │
                              ▼
                ┌──────────────────────────┐
                │ CloudSQL (Primary)       │
                │ Region: us-central1      │
                │                          │
                │ Read Replicas:           │
                │ - us-west1               │
                │ - us-east1               │
                │ - eu-west1               │
                └──────────────────────────┘
```

### 7.2 Database High Availability

**PostgreSQL Configuration:**

```yaml
# CloudSQL High Availability Configuration
availability_type: REGIONAL  # Synchronous replication to standby

backup_configuration:
  enabled: true
  start_time: "03:00"  # UTC
  point_in_time_recovery_enabled: true
  transaction_log_retention_days: 7
  backup_retention_settings:
    retained_backups: 30
    retention_unit: COUNT

maintenance_window:
  day: 7  # Sunday
  hour: 2  # 2 AM UTC

database_flags:
  - name: max_connections
    value: "200"
  - name: shared_buffers
    value: "4GB"
  - name: effective_cache_size
    value: "12GB"
  - name: work_mem
    value: "64MB"
  - name: maintenance_work_mem
    value: "512MB"
  - name: checkpoint_completion_target
    value: "0.9"
  - name: wal_buffers
    value: "16MB"
  - name: default_statistics_target
    value: "100"
```

**Read Replica Routing:**

```rust
// servo-storage/src/read_replica.rs

pub struct ReplicaRouter {
    primary: PgPool,
    replicas: Vec<PgPool>,
}

impl ReplicaRouter {
    pub fn get_pool_for_query(&self, query_type: QueryType) -> &PgPool {
        match query_type {
            QueryType::Write => &self.primary,
            QueryType::ReadConsistent => &self.primary,  // Needs latest data
            QueryType::ReadEventual => {
                // Load balance across replicas
                let idx = rand::thread_rng().gen_range(0..self.replicas.len());
                &self.replicas[idx]
            }
        }
    }
}

#[derive(Debug)]
pub enum QueryType {
    Write,
    ReadConsistent,  // Must read from primary
    ReadEventual,    // Can tolerate replication lag
}

// Usage
impl MetadataStore {
    pub async fn get_execution(&self, execution_id: &Uuid) -> Result<Execution> {
        // Reading execution status should be consistent
        let pool = self.router.get_pool_for_query(QueryType::ReadConsistent);
        
        sqlx::query_as!(
            Execution,
            "SELECT * FROM executions WHERE execution_id = $1",
            execution_id
        )
        .fetch_one(pool)
        .await
    }
    
    pub async fn list_assets(&self, tenant_id: &Uuid) -> Result<Vec<Asset>> {
        // Listing assets can tolerate slight lag
        let pool = self.router.get_pool_for_query(QueryType::ReadEventual);
        
        sqlx::query_as!(
            Asset,
            "SELECT * FROM assets WHERE tenant_id = $1",
            tenant_id
        )
        .fetch_all(pool)
        .await
    }
}
```

### 7.3 Disaster Recovery

**Backup Strategy:**

```rust
// servo-storage/src/backup.rs

pub struct BackupManager {
    storage: Arc<dyn MetadataStore>,
    backup_location: String,  // GCS bucket
}

impl BackupManager {
    pub async fn create_backup(&self) -> Result<BackupInfo> {
        let timestamp = Utc::now();
        let backup_id = Uuid::new_v4();
        
        tracing::info!(backup_id = %backup_id, "Starting database backup");
        
        // Trigger CloudSQL backup via API
        let backup_operation = self.trigger_cloudsql_backup().await?;
        
        // Wait for completion
        self.wait_for_backup_completion(&backup_operation).await?;
        
        // Export metadata to GCS
        self.export_metadata_to_gcs(&backup_id, timestamp).await?;
        
        // Record backup info
        let backup_info = BackupInfo {
            backup_id,
            timestamp,
            size_bytes: self.get_backup_size(&backup_id).await?,
            location: format!("{}/backups/{}", self.backup_location, backup_id),
            status: BackupStatus::Completed,
        };
        
        self.storage.record_backup(&backup_info).await?;
        
        tracing::info!(
            backup_id = %backup_id,
            size_gb = backup_info.size_bytes / 1_073_741_824,
            "Backup completed successfully"
        );
        
        Ok(backup_info)
    }
    
    pub async fn restore_from_backup(&self, backup_id: &Uuid) -> Result<()> {
        tracing::warn!(backup_id = %backup_id, "Starting database restore");
        
        // Validate backup exists
        let backup_info = self.storage.get_backup_info(backup_id).await?;
        
        if backup_info.status != BackupStatus::Completed {
            return Err(Error::InvalidBackup);
        }
        
        // Trigger CloudSQL restore
        self.trigger_cloudsql_restore(&backup_info).await?;
        
        // Wait for completion
        self.wait_for_restore_completion().await?;
        
        // Verify integrity
        self.verify_restore_integrity().await?;
        
        tracing::info!(backup_id = %backup_id, "Restore completed successfully");
        
        Ok(())
    }
    
    pub async fn test_backup_restore(&self) -> Result<()> {
        // Regularly test backup restore in isolated environment
        tracing::info!("Starting backup restore test");
        
        // Get latest backup
        let latest_backup = self.storage.get_latest_backup().await?;
        
        // Restore to test instance
        self.restore_to_test_instance(&latest_backup).await?;
        
        // Run validation queries
        self.validate_test_instance().await?;
        
        // Clean up test instance
        self.cleanup_test_instance().await?;
        
        tracing::info!("Backup restore test completed successfully");
        
        Ok(())
    }
}
```

**RPO and RTO Targets:**

```
Recovery Point Objective (RPO): 5 minutes
- Point-in-time recovery enabled
- Transaction logs retained for 7 days
- Maximum data loss: 5 minutes

Recovery Time Objective (RTO): 30 minutes
- Automated failover to standby: < 1 minute
- Manual restore from backup: < 30 minutes
```

### 7.4 Chaos Engineering

**Chaos Testing Framework:**

```rust
// servo-tests/src/chaos.rs

pub struct ChaosScenario {
    pub name: String,
    pub description: String,
    pub blast_radius: BlastRadius,
    pub duration: Duration,
}

#[derive(Debug)]
pub enum BlastRadius {
    SingleWorker,
    MultipleWorkers { percentage: f64 },
    EntireRegion,
    Database,
    Queue,
}

pub struct ChaosEngine {
    cloud_provider: Arc<dyn CloudProvider>,
}

impl ChaosEngine {
    pub async fn run_scenario(&self, scenario: ChaosScenario) -> Result<ChaosResult> {
        tracing::warn!(
            scenario = %scenario.name,
            "Starting chaos engineering scenario"
        );
        
        // Record baseline metrics
        let baseline = self.capture_metrics().await?;
        
        // Inject failure
        self.inject_failure(&scenario).await?;
        
        // Monitor system behavior
        tokio::time::sleep(scenario.duration).await;
        
        // Capture metrics during chaos
        let during_chaos = self.capture_metrics().await?;
        
        // Remove failure injection
        self.remove_failure(&scenario).await?;
        
        // Wait for recovery
        tokio::time::sleep(Duration::from_secs(60)).await;
        
        // Capture metrics after recovery
        let after_recovery = self.capture_metrics().await?;
        
        // Analyze results
        let result = ChaosResult {
            scenario: scenario.name.clone(),
            baseline_metrics: baseline,
            chaos_metrics: during_chaos,
            recovery_metrics: after_recovery,
            passed: self.validate_resilience(&baseline, &after_recovery),
        };
        
        tracing::info!(
            scenario = %scenario.name,
            passed = result.passed,
            "Chaos engineering scenario completed"
        );
        
        Ok(result)
    }
    
    async fn inject_failure(&self, scenario: &ChaosScenario) -> Result<()> {
        match &scenario.blast_radius {
            BlastRadius::SingleWorker => {
                // Kill a random worker instance
                self.cloud_provider.terminate_random_instance().await?;
            }
            BlastRadius::Database => {
                // Inject latency into database connections
                self.inject_database_latency(Duration::from_millis(500)).await?;
            }
            BlastRadius::Queue => {
                // Temporarily disable queue processing
                self.disable_queue_processing().await?;
            }
            _ => {}
        }
        
        Ok(())
    }
}

// Example chaos scenarios
pub fn database_latency_scenario() -> ChaosScenario {
    ChaosScenario {
        name: "Database Latency".to_string(),
        description: "Inject 500ms latency into all database queries".to_string(),
        blast_radius: BlastRadius::Database,
        duration: Duration::from_secs(300),  // 5 minutes
    }
}

pub fn worker_failure_scenario() -> ChaosScenario {
    ChaosScenario {
        name: "Worker Node Failure".to_string(),
        description: "Terminate 30% of worker instances".to_string(),
        blast_radius: BlastRadius::MultipleWorkers { percentage: 0.3 },
        duration: Duration::from_secs(600),  // 10 minutes
    }
}
```

---

## 8. Extensibility and Plugin System

### 8.1 Plugin Architecture

```rust
// servo-core/src/plugin.rs

use async_trait::async_trait;
use serde_json::Value;

#[async_trait]
pub trait Plugin: Send + Sync {
    /// Plugin metadata
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn description(&self) -> &str;
    
    /// Initialize plugin with configuration
    async fn initialize(&mut self, config: Value) -> Result<()>;
    
    /// Execute plugin
    async fn execute(&self, input: PluginInput) -> Result<PluginOutput>;
    
    /// Validate plugin configuration
    fn validate_config(&self, config: &Value) -> Result<()>;
    
    /// Shutdown hook
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PluginInput {
    pub execution_id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub parameters: HashMap<String, Value>,
    pub context: ExecutionContext,
}

#[derive(Debug, Clone)]
pub struct PluginOutput {
    pub data: Value,
    pub artifacts: Vec<Artifact>,
    pub metrics: HashMap<String, f64>,
}

pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }
    
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<()> {
        let name = plugin.name().to_string();
        
        if self.plugins.contains_key(&name) {
            return Err(Error::PluginAlreadyRegistered(name));
        }
        
        tracing::info!(plugin = %name, "Registering plugin");
        self.plugins.insert(name, plugin);
        
        Ok(())
    }
    
    pub fn get(&self, name: &str) -> Option<&Box<dyn Plugin>> {
        self.plugins.get(name)
    }
    
    pub async fn execute_plugin(
        &self,
        name: &str,
        input: PluginInput,
    ) -> Result<PluginOutput> {
        let plugin = self.get(name)
            .ok_or_else(|| Error::PluginNotFound(name.to_string()))?;
        
        plugin.execute(input).await
    }
}
```

### 8.2 Example: dbt Plugin

```rust
// servo-plugins/dbt/src/lib.rs

use servo_core::plugin::*;
use async_trait::async_trait;

pub struct DbtPlugin {
    config: DbtConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct DbtConfig {
    project_dir: String,
    profiles_dir: String,
    target: String,
}

#[async_trait]
impl Plugin for DbtPlugin {
    fn name(&self) -> &str {
        "dbt"
    }
    
    fn version(&self) -> &str {
        "1.0.0"
    }
    
    fn description(&self) -> &str {
        "Execute dbt models and tests"
    }
    
    async fn initialize(&mut self, config: Value) -> Result<()> {
        self.config = serde_json::from_value(config)?;
        
        // Validate dbt project exists
        if !std::path::Path::new(&self.config.project_dir).exists() {
            return Err(Error::InvalidConfig(
                format!("dbt project not found: {}", self.config.project_dir)
            ));
        }
        
        Ok(())
    }
    
    async fn execute(&self, input: PluginInput) -> Result<PluginOutput> {
        let models = input.parameters.get("models")
            .and_then(|v| v.as_str())
            .unwrap_or("all");
        
        let command = format!(
            "dbt run --project-dir {} --profiles-dir {} --target {} --models {}",
            self.config.project_dir,
            self.config.profiles_dir,
            self.config.target,
            models
        );
        
        tracing::info!(command = %command, "Executing dbt command");
        
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .output()
            .await?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::PluginExecutionFailed(stderr.to_string()));
        }
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Parse dbt output for metrics
        let metrics = self.parse_dbt_output(&stdout)?;
        
        Ok(PluginOutput {
            data: serde_json::json!({
                "status": "success",
                "output": stdout.to_string()
            }),
            artifacts: vec![],
            metrics,
        })
    }
    
    fn validate_config(&self, config: &Value) -> Result<()> {
        // Validate required fields
        if !config.get("project_dir").is_some() {
            return Err(Error::InvalidConfig("project_dir is required".to_string()));
        }
        
        Ok(())
    }
}

impl DbtPlugin {
    fn parse_dbt_output(&self, output: &str) -> Result<HashMap<String, f64>> {
        let mut metrics = HashMap::new();
        
        // Extract model counts, execution time, etc.
        if let Some(models_run) = self.extract_models_run(output) {
            metrics.insert("models_run".to_string(), models_run as f64);
        }
        
        if let Some(duration) = self.extract_duration(output) {
            metrics.insert("duration_seconds".to_string(), duration);
        }
        
        Ok(metrics)
    }
}
```

**Using Plugin in Workflow:**

```python
# Python SDK integration
from servo import asset, plugin

@asset(name="dbt_models")
def run_dbt_models():
    """Execute dbt models using plugin."""
    return plugin.execute(
        "dbt",
        config={
            "project_dir": "/path/to/dbt/project",
            "profiles_dir": "~/.dbt",
            "target": "production"
        },
        parameters={
            "models": "mart.*"  # Run all models in mart folder
        }
    )
```

### 8.3 Plugin Discovery and Loading

```rust
// servo-core/src/plugin/loader.rs

pub struct PluginLoader {
    plugin_dirs: Vec<PathBuf>,
}

impl PluginLoader {
    pub fn new() -> Self {
        Self {
            plugin_dirs: vec![
                PathBuf::from("/usr/local/lib/servo/plugins"),
                PathBuf::from("~/.servo/plugins"),
                PathBuf::from("./plugins"),
            ],
        }
    }
    
    pub async fn load_all_plugins(&self) -> Result<PluginRegistry> {
        let mut registry = PluginRegistry::new();
        
        for plugin_dir in &self.plugin_dirs {
            if !plugin_dir.exists() {
                continue;
            }
            
            for entry in std::fs::read_dir(plugin_dir)? {
                let entry = entry?;
                let path = entry.path();
                
                if path.extension() == Some(std::ffi::OsStr::new("so")) {
                    // Load shared library plugin
                    match self.load_plugin_from_lib(&path).await {
                        Ok(plugin) => {
                            registry.register(plugin)?;
                        }
                        Err(e) => {
                            tracing::warn!(
                                path = ?path,
                                error = %e,
                                "Failed to load plugin"
                            );
                        }
                    }
                }
            }
        }
        
        Ok(registry)
    }
    
    async fn load_plugin_from_lib(&self, path: &Path) -> Result<Box<dyn Plugin>> {
        // Use libloading to dynamically load plugin
        unsafe {
            let lib = libloading::Library::new(path)?;
            let constructor: libloading::Symbol<unsafe extern fn() -> *mut dyn Plugin> =
                lib.get(b"_plugin_create")?;
            
            let plugin_ptr = constructor();
            Ok(Box::from_raw(plugin_ptr))
        }
    }
}

// Plugin authors implement this function
#[no_mangle]
pub extern "C" fn _plugin_create() -> *mut dyn Plugin {
    let plugin = MyPlugin::new();
    Box::into_raw(Box::new(plugin))
}
```

---

## 9. Developer Experience Enhancements

### 9.1 Local Development Mode

```rust
// servo-cli/src/commands/dev.rs

pub struct DevServer {
    config: DevConfig,
    registry: PluginRegistry,
    storage: Arc<dyn MetadataStore>,
}

#[derive(Debug, Clone)]
pub struct DevConfig {
    pub port: u16,
    pub auto_reload: bool,
    pub mock_cloud_services: bool,
}

impl DevServer {
    pub async fn start(&self) -> Result<()> {
        tracing::info!(port = self.config.port, "Starting Servo dev server");
        
        // Use in-memory SQLite for dev
        let storage = SqliteMetadataStore::in_memory().await?;
        
        // Run migrations
        storage.migrate().await?;
        
        // Mock cloud services if requested
        let executor: Box<dyn Executor> = if self.config.mock_cloud_services {
            Box::new(LocalExecutor::new())
        } else {
            // Use real cloud services
            Box::new(CloudRunExecutor::new("dev-project".to_string(), "us-central1".to_string()).await?)
        };
        
        // Watch for file changes
        if self.config.auto_reload {
            self.watch_for_changes().await?;
        }
        
        // Start HTTP server
        self.start_http_server().await?;
        
        Ok(())
    }
    
    async fn watch_for_changes(&self) -> Result<()> {
        use notify::{Watcher, RecursiveMode};
        
        let (tx, rx) = std::sync::mpsc::channel();
        
        let mut watcher = notify::watcher(tx, Duration::from_secs(1))?;
        watcher.watch(".", RecursiveMode::Recursive)?;
        
        tokio::spawn(async move {
            loop {
                match rx.recv() {
                    Ok(event) => {
                        tracing::info!(?event, "File changed, reloading");
                        // Reload workflows
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Watch error");
                        break;
                    }
                }
            }
        });
        
        Ok(())
    }
}

// Local executor for dev mode
pub struct LocalExecutor {
    tasks: Arc<RwLock<HashMap<TaskId, Task>>>,
}

#[async_trait]
impl Executor for LocalExecutor {
    async fn enqueue_task(&self, task: Task) -> Result<TaskId> {
        tracing::info!(
            task_id = %task.id,
            asset_key = %task.asset.key,
            "Enqueuing task (local mode)"
        );
        
        let task_id = task.id.clone();
        
        // Store task
        self.tasks.write().await.insert(task_id.clone(), task.clone());
        
        // Execute immediately in background
        let tasks = self.tasks.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::execute_task_local(task).await {
                tracing::error!(error = %e, "Local task execution failed");
            }
            
            // Remove from pending
            tasks.write().await.remove(&task_id);
        });
        
        Ok(task_id)
    }
}
```

**CLI Dev Commands:**

```bash
# Start local dev server
servo dev --auto-reload --mock-cloud

# Run workflow locally
servo run workflow.py --local

# Validate workflow without executing
servo validate workflow.py

# Generate sample workflow
servo init workflow --template simple-etl

# Interactive debugging
servo debug execution-id-here
```

### 9.2 Workflow Testing Framework

```python
# servo-python/servo/testing.py

from servo import asset, workflow
from servo.testing import WorkflowTestCase, mock_asset

class TestCustomerPipeline(WorkflowTestCase):
    def test_clean_customers_removes_duplicates(self):
        """Test that clean_customers removes duplicate records."""
        
        # Mock input data
        raw_data = [
            {"id": 1, "name": "Alice"},
            {"id": 1, "name": "Alice"},  # Duplicate
            {"id": 2, "name": "Bob"},
        ]
        
        # Mock the extract asset
        with mock_asset("extract_customers", return_value=raw_data):
            # Execute the clean_customers asset
            result = self.execute_asset("clean_customers")
        
        # Verify duplicates removed
        self.assertEqual(len(result), 2)
        self.assertIn({"id": 1, "name": "Alice"}, result)
        self.assertIn({"id": 2, "name": "Bob"}, result)
    
    def test_workflow_execution_order(self):
        """Test that workflow executes assets in correct order."""
        
        execution_order = []
        
        @asset(name="step1")
        def step1():
            execution_order.append("step1")
            return 1
        
        @asset(name="step2", dependencies=["step1"])
        def step2(step1_result):
            execution_order.append("step2")
            return step1_result + 1
        
        @asset(name="step3", dependencies=["step2"])
        def step3(step2_result):
            execution_order.append("step3")
            return step2_result + 1
        
        @workflow(name="test_workflow")
        def my_workflow():
            a = step1()
            b = step2(a)
            c = step3(b)
            return c
        
        # Execute workflow
        result = self.execute_workflow(my_workflow)
        
        # Verify execution order
        self.assertEqual(execution_order, ["step1", "step2", "step3"])
        self.assertEqual(result, 3)
    
    def test_workflow_with_failure(self):
        """Test workflow failure handling."""
        
        @asset(name="failing_asset")
        def failing_asset():
            raise ValueError("Intentional failure")
        
        @workflow(name="test_workflow")
        def my_workflow():
            failing_asset()
        
        # Verify workflow fails appropriately
        with self.assertRaises(ValueError):
            self.execute_workflow(my_workflow)
```

### 9.3 Documentation Generation

```rust
// servo-cli/src/commands/docs.rs

pub struct DocsGenerator {
    workflow_dir: PathBuf,
    output_dir: PathBuf,
}

impl DocsGenerator {
    pub async fn generate_workflow_docs(&self) -> Result<()> {
        tracing::info!("Generating workflow documentation");
        
        // Discover all workflows
        let workflows = self.discover_workflows().await?;
        
        // Generate markdown docs
        for workflow in workflows {
            let doc = self.generate_workflow_doc(&workflow)?;
            
            let output_path = self.output_dir
                .join(format!("{}.md", workflow.name));
            
            std::fs::write(&output_path, doc)?;
        }
        
        // Generate index
        self.generate_index(&workflows)?;
        
        // Generate lineage diagram
        self.generate_lineage_diagram(&workflows)?;
        
        Ok(())
    }
    
    fn generate_workflow_doc(&self, workflow: &Workflow) -> Result<String> {
        let mut doc = String::new();
        
        // Title
        doc.push_str(&format!("# {}\n\n", workflow.name));
        
        // Description
        if let Some(desc) = &workflow.description {
            doc.push_str(&format!("{}\n\n", desc));
        }
        
        // Metadata table
        doc.push_str("## Metadata\n\n");
        doc.push_str("| Property | Value |\n");
        doc.push_str("|----------|-------|\n");
        doc.push_str(&format!("| Schedule | {} |\n", 
            workflow.schedule.as_ref().map(|s| s.as_str()).unwrap_or("Manual")));
        doc.push_str(&format!("| Assets | {} |\n", workflow.assets.len()));
        doc.push_str("\n");
        
        // Assets
        doc.push_str("## Assets\n\n");
        for asset in &workflow.assets {
            doc.push_str(&format!("### {}\n\n", asset.key));
            
            if let Some(desc) = &asset.metadata.description {
                doc.push_str(&format!("{}\n\n", desc));
            }
            
            if !asset.dependencies.is_empty() {
                doc.push_str("**Dependencies:**\n");
                for dep in &asset.dependencies {
                    doc.push_str(&format!("- `{}`\n", dep));
                }
                doc.push_str("\n");
            }
            
            // Metadata
            if let Some(owner) = asset.metadata.get("owner") {
                doc.push_str(&format!("**Owner:** {}\n\n", owner));
            }
            
            if let Some(sla) = asset.metadata.get("sla") {
                doc.push_str(&format!("**SLA:** {}\n\n", sla));
            }
        }
        
        // Lineage diagram (Mermaid)
        doc.push_str("## Lineage\n\n");
        doc.push_str("```mermaid\n");
        doc.push_str("graph LR\n");
        for asset in &workflow.assets {
            for dep in &asset.dependencies {
                doc.push_str(&format!("  {}[{}] --> {}[{}]\n", 
                    dep, dep, asset.key, asset.key));
            }
        }
        doc.push_str("```\n");
        
        Ok(doc)
    }
}
```

---

## 10. Optional Web Dashboard

### 10.1 Dashboard Architecture

```
┌─────────────────────────────────────────────────────────┐
│ Frontend (React + TypeScript)                           │
│ ┌─────────────────────────────────────────────────────┐ │
│ │ Pages:                                               │ │
│ │ - Dashboard (overview)                               │ │
│ │ - Workflows (list, detail)                           │ │
│ │ - Executions (list, detail, logs)                    │ │
│ │ - Assets (catalog, lineage viz)                      │ │
│ │ - Metrics (charts, cost attribution)                 │ │
│ └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
                          │ REST API
                          ▼
┌─────────────────────────────────────────────────────────┐
│ Backend (FastAPI + Rust via PyO3)                       │
│ ┌─────────────────────────────────────────────────────┐ │
│ │ Endpoints:                                           │ │
│ │ - /api/v1/workflows                                  │ │
│ │ - /api/v1/executions                                 │ │
│ │ - /api/v1/assets                                     │ │
│ │ - /api/v1/lineage                                    │ │
│ │ - /api/v1/metrics                                    │ │
│ └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│ Metadata Database (PostgreSQL)                          │
└─────────────────────────────────────────────────────────┘
```

### 10.2 Key Dashboard Features

**1. Execution Monitoring:**
- Real-time execution status
- Task-level progress visualization
- Log streaming
- Error details with stack traces

**2. Lineage Visualization:**
- Interactive asset dependency graph
- Impact analysis ("what breaks if I change this?")
- Column-level lineage (when tracked)
- Time-travel queries

**3. Cost Dashboard:**
- Per-tenant cost breakdown
- Per-workflow cost trends
- Resource usage charts
- Cost anomaly detection

**4. Asset Catalog:**
- Searchable asset inventory
- Schema evolution history
- Data quality metrics
- Ownership and SLA tracking

### 10.3 Example: Lineage Visualization Component

```typescript
// servo-ui/src/components/LineageGraph.tsx

import React, { useEffect, useRef } from 'react';
import Cytoscape from 'cytoscape';
import dagre from 'cytoscape-dagre';

interface Asset {
  id: string;
  key: string;
  type: string;
  metadata: Record<string, any>;
}

interface LineageEdge {
  source: string;
  target: string;
  transformation: string;
}

interface LineageGraphProps {
  assets: Asset[];
  edges: LineageEdge[];
  selectedAsset?: string;
  onAssetClick?: (assetId: string) => void;
}

export const LineageGraph: React.FC<LineageGraphProps> = ({
  assets,
  edges,
  selectedAsset,
  onAssetClick,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<Cytoscape.Core | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    // Register dagre layout
    Cytoscape.use(dagre);

    // Initialize Cytoscape
    cyRef.current = Cytoscape({
      container: containerRef.current,
      
      elements: [
        // Nodes (assets)
        ...assets.map(asset => ({
          data: {
            id: asset.id,
            label: asset.key,
            type: asset.type,
          },
          classes: selectedAsset === asset.id ? 'selected' : '',
        })),
        
        // Edges (dependencies)
        ...edges.map((edge, idx) => ({
          data: {
            id: `edge-${idx}`,
            source: edge.source,
            target: edge.target,
            transformation: edge.transformation,
          },
        })),
      ],
      
      layout: {
        name: 'dagre',
        rankDir: 'LR',  // Left to right
        nodeSep: 50,
        rankSep: 100,
      },
      
      style: [
        {
          selector: 'node',
          style: {
            'label': 'data(label)',
            'background-color': '#3b82f6',
            'color': '#fff',
            'text-valign': 'center',
            'text-halign': 'center',
            'font-size': '12px',
            'width': '80px',
            'height': '40px',
            'shape': 'roundrectangle',
          },
        },
        {
          selector: 'node.selected',
          style: {
            'background-color': '#ef4444',
            'border-width': '3px',
            'border-color': '#991b1b',
          },
        },
        {
          selector: 'edge',
          style: {
            'width': 2,
            'line-color': '#94a3b8',
            'target-arrow-color': '#94a3b8',
            'target-arrow-shape': 'triangle',
            'curve-style': 'bezier',
          },
        },
        {
          selector: 'edge:selected',
          style: {
            'line-color': '#3b82f6',
            'target-arrow-color': '#3b82f6',
            'width': 3,
          },
        },
      ],
    });

    // Handle click events
    cyRef.current.on('tap', 'node', (evt) => {
      const node = evt.target;
      onAssetClick?.(node.id());
    });

    return () => {
      cyRef.current?.destroy();
    };
  }, [assets, edges, selectedAsset, onAssetClick]);

  return (
    <div
      ref={containerRef}
      style={{
        width: '100%',
        height: '600px',
        border: '1px solid #e5e7eb',
        borderRadius: '8px',
      }}
    />
  );
};
```

---

## 11. Implementation Priorities

### 11.1 Phase 1: Foundation (Months 1-3)

**Must-Have:**
1. ✅ Core testing infrastructure (unit, integration tests)
2. ✅ Configuration management (YAML config, env overrides)
3. ✅ Basic secrets management (Secret Manager integration)
4. ✅ Structured logging with OpenTelemetry
5. ✅ Idempotency for executions
6. ✅ Basic retry policies

**Nice-to-Have:**
- Local development mode
- Simple cost tracking
- Basic documentation generation

### 11.2 Phase 2: Production Readiness (Months 4-6)

**Must-Have:**
1. ✅ Advanced observability (metrics, tracing, alerting)
2. ✅ Comprehensive retry and circuit breaker
3. ✅ Multi-region support
4. ✅ Database HA and backups
5. ✅ Security hardening (rate limiting, input validation)
6. ✅ Cost monitoring and attribution

**Nice-to-Have:**
- Plugin system MVP
- Chaos engineering tests
- Workflow testing framework

### 11.3 Phase 3: Developer Experience (Months 7-9)

**Must-Have:**
1. ✅ Plugin system (dbt, Airbyte, Spark connectors)
2. ✅ Local dev server with auto-reload
3. ✅ Comprehensive documentation
4. ✅ Example workflows and tutorials

**Nice-to-Have:**
- Optional web dashboard
- Interactive debugging
- Secrets rotation automation

### 11.4 Phase 4: Enterprise Features (Months 10-12)

**Must-Have:**
1. ✅ Disaster recovery procedures
2. ✅ High availability validation
3. ✅ Performance optimization
4. ✅ Security audit

**Nice-to-Have:**
- Advanced web dashboard
- Custom plugin marketplace
- Enterprise SSO integration

---

## Summary: Best Practices Checklist

### Testing
- ✅ Unit tests for all core functions
- ✅ Integration tests with real dependencies
- ✅ Contract tests for cloud APIs
- ✅ End-to-end workflow tests
- ✅ Chaos engineering scenarios
- ✅ Performance benchmarks

### Configuration & Secrets
- ✅ Hierarchical configuration (env → file → defaults)
- ✅ Environment-specific configs (dev, staging, prod)
- ✅ Secrets backend abstraction (GCP, AWS, Vault)
- ✅ Secrets rotation automation
- ✅ No hardcoded credentials

### Observability
- ✅ Structured logging with correlation IDs
- ✅ Distributed tracing (OpenTelemetry)
- ✅ Prometheus metrics
- ✅ Alerting for key failures
- ✅ Cost tracking per tenant

### Idempotency & Retries
- ✅ Execution-level idempotency keys
- ✅ Task-level deduplication
- ✅ Exponential backoff with jitter
- ✅ Selective retry based on error type
- ✅ Circuit breaker for external services

### Security
- ✅ Input validation and sanitization
- ✅ SQL injection prevention
- ✅ Rate limiting (token bucket)
- ✅ Container image scanning
- ✅ Non-root containers
- ✅ Encryption at rest and in transit
- ✅ Audit logging

### Cost Optimization
- ✅ Resource tagging for attribution
- ✅ Cost tracking per tenant/workflow
- ✅ Alerting on cost thresholds
- ✅ Spot instance support
- ✅ Intelligent caching
- ✅ Resource rightsizing recommendations

### High Availability
- ✅ Multi-region deployment
- ✅ Database read replicas
- ✅ Automated backups (7-day retention)
- ✅ Point-in-time recovery
- ✅ Disaster recovery procedures
- ✅ Regular DR testing

### Extensibility
- ✅ Plugin system architecture
- ✅ Plugin registry and discovery
- ✅ Standard plugin interface
- ✅ Example plugins (dbt, Airbyte)

### Developer Experience
- ✅ Local development mode
- ✅ Workflow testing framework
- ✅ Auto-generated documentation
- ✅ Comprehensive examples
- ✅ Clear error messages

---

**This supplementary document should be used in conjunction with the main Project Design Document to ensure Servo meets enterprise-grade standards.**