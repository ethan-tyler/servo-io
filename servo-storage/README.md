# servo-storage

Metadata storage for Servo using PostgreSQL with row-level security for multi-tenancy.

## Overview

The `servo-storage` crate provides PostgreSQL-backed persistence for Servo metadata:

- **Models**: Database models for assets, workflows, and executions
- **Migrations**: SQL migrations for schema management
- **Tenant Support**: Row-level security for multi-tenant isolation
- **Lineage**: Asset dependency tracking

## Usage

```rust
use servo_storage::{PostgresStorage, models::AssetModel, TenantId};

// Connect to database
let storage = PostgresStorage::new("postgresql://localhost/servo").await?;

// Run migrations
servo_storage::migrations::run_migrations(storage.pool()).await?;

// Create an asset
let asset = AssetModel {
    id: Uuid::new_v4(),
    name: "customer_data".to_string(),
    asset_type: "table".to_string(),
    // ...
};

let tenant = TenantId::new("tenant_123");
storage.create_asset(&asset, Some(&tenant)).await?;

// Retrieve asset
let retrieved = storage.get_asset(asset.id, Some(&tenant)).await?;
```

## Database Schema

The initial migration creates:

- `assets` - Data assets (tables, files, models)
- `workflows` - Workflow definitions
- `executions` - Execution runs
- `asset_dependencies` - Lineage relationships

All tables include row-level security for multi-tenant isolation.

## Migrations

Migrations are managed using `sqlx-cli`:

```bash
# Install sqlx-cli
cargo install sqlx-cli --features postgres

# Run migrations
sqlx migrate run --database-url postgresql://localhost/servo

# Create new migration
sqlx migrate add migration_name
```

## Testing

```bash
# Set up test database
export DATABASE_URL=postgresql://localhost/servo_test
sqlx database create
sqlx migrate run

# Run tests
cargo test -p servo-storage
```

## Documentation

For more details, see the [main Servo documentation](https://docs.servo.dev).
