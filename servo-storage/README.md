# servo-storage

PostgreSQL-based metadata storage layer for Servo with multi-tenant isolation.

## Features

- **Full CRUD operations** for assets, workflows, and executions
- **Pagination support** for list operations
- **Multi-tenant isolation** with row-level security (RLS)
- **Asset dependency tracking** for lineage and provenance
- **Connection pooling** with configurable retry logic
- **Async/await** with tokio and sqlx

## Database Schema

The storage layer uses PostgreSQL with the following tables:

- `assets` - Asset metadata with partition configuration
- `workflows` - Workflow definitions with versioning
- `executions` - Workflow execution state
- `asset_dependencies` - Asset lineage relationships

All tables have row-level security (RLS) enabled for tenant isolation.

## Running Integration Tests

Integration tests require a PostgreSQL database. Set up the test database:

```bash
# 1. Start PostgreSQL (if using Docker)
docker run -d \
  --name servo-postgres-test \
  -e POSTGRES_PASSWORD=servo \
  -e POSTGRES_USER=servo \
  -e POSTGRES_DB=servo_test \
  -p 5433:5432 \
  postgres:15

# 2. Set the test database URL (optional, defaults shown)
export TEST_DATABASE_URL=postgresql://servo:servo@localhost:5433/servo_test

# 3. Run integration tests
cargo test --package servo-storage -- --ignored

# Run specific test
cargo test --package servo-storage test_tenant_isolation -- --ignored
```

## Usage Example

```rust
use servo_storage::{PostgresStorage, TenantId, AssetModel};
use chrono::Utc;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to database
    let storage = PostgresStorage::new("postgresql://servo:servo@localhost/servo_dev").await?;

    // Create an asset with tenant isolation
    let tenant = TenantId::new("tenant_123");
    let asset = AssetModel {
        id: Uuid::new_v4(),
        name: "customer_events".to_string(),
        description: Some("Daily customer event logs".to_string()),
        asset_type: "table".to_string(),
        owner: Some("data_team".to_string()),
        tags: sqlx::types::Json(vec!["pii".to_string(), "daily".to_string()]),
        tenant_id: Some("tenant_123".to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    storage.create_asset(&asset, Some(&tenant)).await?;

    // List assets for tenant with pagination
    let assets = storage.list_assets(Some(&tenant), 10, 0).await?;
    println!("Found {} assets for tenant", assets.len());

    // Get asset by ID
    let retrieved = storage.get_asset(asset.id, Some(&tenant)).await?;
    println!("Asset: {}", retrieved.name);

    Ok(())
}
```

## Configuration

Connection pool can be configured with custom settings:

```rust
let storage = PostgresStorage::with_config(
    "postgresql://servo:servo@localhost/servo_dev",
    10,  // max_connections
    30,  // connection_timeout_secs
).await?;
```

## Multi-Tenancy

All operations accept an optional `TenantId` parameter. When provided:

- Data is automatically filtered by tenant
- Cross-tenant access is prevented by RLS policies
- NULL tenant_id represents global/system data

```rust
let tenant1 = TenantId::new("tenant_1");
let tenant2 = TenantId::new("tenant_2");

// Tenant 1 can only see their data
let assets = storage.list_assets(Some(&tenant1), 10, 0).await?;

// Trying to access tenant 2's data will fail
let result = storage.get_asset(tenant2_asset_id, Some(&tenant1)).await;
assert!(result.is_err());
```

## Asset Dependencies & Lineage

Track dependencies between assets for lineage and provenance:

```rust
// Create dependency relationship
storage.create_asset_dependency(
    upstream_asset_id,
    downstream_asset_id,
    "data",  // dependency type: "data", "metadata", or "control"
).await?;

// Get full lineage graph
let lineage = storage.get_asset_lineage(asset_id, 10).await?;  // max depth 10
```

## Testing

Unit tests (no database required):

```bash
cargo test --package servo-storage --lib
```

Integration tests (requires PostgreSQL):

```bash
cargo test --package servo-storage -- --ignored
```

## Migrations

Migrations are managed with sqlx:

```bash
# Create new migration
sqlx migrate add <name>

# Run migrations
sqlx migrate run --database-url postgresql://servo:servo@localhost/servo_dev
```

Or programmatically:

```rust
use servo_storage::migrations::run_migrations;

let pool = storage.pool();
run_migrations(pool).await?;
```

## Logging & Observability

The storage layer provides comprehensive structured logging using the `tracing` crate.

### Logged Events

- **Database Errors**: All database errors are logged with error codes and context
- **Slow Queries**: Operations taking >100ms are logged as warnings with tenant_id and duration
- **Connection Pool Issues**: Pool exhaustion and connection failures are logged as errors
- **Constraint Violations**: Validation errors (unique violations, FK violations, CHECK failures) are logged as warnings

### JSON Logging (Production)

Configure JSON-structured logging for production environments:

```rust
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// Initialize JSON logging
tracing_subscriber::registry()
    .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
    .with(tracing_subscriber::fmt::layer().json())
    .init();
```

### Log Levels

- `ERROR`: Critical errors (connection failures, pool exhausted, unexpected database errors)
- `WARN`: Expected errors (constraint violations, slow queries >100ms)
- `INFO`: Normal operations (via tracing spans)

### Example Log Output

```json
{
  "timestamp": "2025-11-22T10:30:45.123Z",
  "level": "WARN",
  "message": "Slow database operation detected",
  "tenant_id": "acme-corp",
  "duration_ms": 150,
  "span": {
    "name": "create_asset",
    "tenant": "acme-corp",
    "asset_id": "123e4567-e89b-12d3-a456-426614174000"
  }
}
```
