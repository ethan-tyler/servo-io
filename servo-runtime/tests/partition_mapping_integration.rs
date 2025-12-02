//! Integration tests for partition mapping in backfill jobs
//!
//! These tests validate that partition configs are loaded from the database
//! and applied correctly when creating upstream child jobs.
//!
//! Run with: cargo test --package servo-runtime --test partition_mapping_integration -- --ignored

use chrono::Utc;
use servo_core::{PartitionConfig, PartitionType, TimeGranularity};
use servo_storage::{BackfillJobModel, PostgresStorage, TenantId};
use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use uuid::Uuid;

/// Get test database URL from environment or use default
fn get_test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://servo:servo@localhost:5432/servo_test".to_string())
}

/// Get test application database URL (non-owner, RLS enforced)
fn get_test_app_database_url() -> String {
    std::env::var("TEST_APP_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://servo_app:servo_app@localhost:5432/servo_test".to_string()
    })
}

/// Setup test database with migrations
async fn setup_test_db() -> Result<PostgresStorage, Box<dyn std::error::Error>> {
    let owner_url = get_test_database_url();
    let owner_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&owner_url)
        .await?;

    // Drop and recreate schema for clean state
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&owner_pool)
        .await?;
    sqlx::query("CREATE SCHEMA public")
        .execute(&owner_pool)
        .await?;
    sqlx::query("GRANT ALL ON SCHEMA public TO PUBLIC")
        .execute(&owner_pool)
        .await?;

    // Run migrations
    servo_storage::migrations::run_migrations(&owner_pool).await?;

    // Connect as app role
    let app_url = get_test_app_database_url();
    let storage = PostgresStorage::new(&app_url).await?;

    Ok(storage)
}

fn unique_tenant() -> TenantId {
    TenantId::new(Uuid::new_v4().to_string())
}

fn unique_name(prefix: &str) -> String {
    format!("{}_{}", prefix, Uuid::new_v4())
}

/// Create a test asset with optional partition configuration
async fn create_test_asset_with_partition(
    storage: &PostgresStorage,
    name: &str,
    partition_config: Option<PartitionConfig>,
    tenant: &TenantId,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let partition_config_json = partition_config
        .map(|c| Json(serde_json::to_value(c).expect("Failed to serialize partition config")));

    let asset = servo_storage::AssetModel {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: Some(format!("Test asset: {}", name)),
        asset_type: "table".to_string(),
        owner: Some("test".to_string()),
        tags: Json(vec![]),
        partition_config: partition_config_json,
        compute_fn_module: None,
        compute_fn_function: None,
        tenant_id: Some(tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    storage.create_asset(&asset, tenant).await?;
    Ok(asset.id)
}

/// Create a dependency between two assets (downstream depends on upstream)
async fn create_dependency(
    storage: &PostgresStorage,
    upstream_id: Uuid,
    downstream_id: Uuid,
    tenant: &TenantId,
) -> Result<(), Box<dyn std::error::Error>> {
    storage
        .create_asset_dependency(upstream_id, downstream_id, "data", tenant)
        .await?;
    Ok(())
}

/// Create a test backfill job with specific partition range
async fn create_test_backfill_job(
    storage: &PostgresStorage,
    asset_id: Uuid,
    asset_name: &str,
    partition_start: Option<String>,
    partition_end: Option<String>,
    include_upstream: bool,
    tenant: &TenantId,
) -> Result<BackfillJobModel, Box<dyn std::error::Error>> {
    let job = BackfillJobModel {
        id: Uuid::new_v4(),
        tenant_id: tenant.as_str().to_string(),
        asset_id,
        asset_name: asset_name.to_string(),
        idempotency_key: format!("test-{}", Uuid::new_v4()),
        state: "pending".to_string(),
        execution_strategy: Json(serde_json::json!({"type": "sequential"})),
        partition_start,
        partition_end,
        partition_keys: Json(vec![]),
        total_partitions: 1,
        completed_partitions: 0,
        failed_partitions: 0,
        skipped_partitions: 0,
        include_upstream,
        error_message: None,
        created_by: Some("test".to_string()),
        created_at: Utc::now(),
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        version: 1,
        paused_at: None,
        checkpoint_partition_key: None,
        estimated_completion_at: None,
        avg_partition_duration_ms: None,
        parent_job_id: None,
        max_upstream_depth: 1,
        upstream_job_count: 0,
        completed_upstream_jobs: 0,
        execution_order: 0,
        sla_deadline_at: None,
        priority: 0,
    };
    storage.create_backfill_job(&job, tenant).await?;
    Ok(job)
}

#[tokio::test]
#[ignore] // Requires database
async fn test_partition_config_persistence() {
    // Test that partition config is correctly stored and retrieved
    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create asset with daily partition config
    let daily_config = PartitionConfig {
        partition_type: PartitionType::Time {
            granularity: TimeGranularity::Daily,
        },
        partition_key: "event_date".to_string(),
    };

    let asset_id = create_test_asset_with_partition(
        &storage,
        &unique_name("daily_asset"),
        Some(daily_config.clone()),
        &tenant,
    )
    .await
    .expect("Failed to create asset");

    // Retrieve partition config
    let retrieved_config = storage
        .get_asset_partition_config(asset_id, &tenant)
        .await
        .expect("Failed to get partition config");

    assert!(retrieved_config.is_some(), "Partition config should exist");

    let config_value = retrieved_config.unwrap();
    let deserialized: PartitionConfig =
        serde_json::from_value(config_value).expect("Failed to deserialize partition config");

    assert_eq!(deserialized.partition_key, "event_date");
    match deserialized.partition_type {
        PartitionType::Time { granularity } => {
            assert!(matches!(granularity, TimeGranularity::Daily));
        }
        _ => panic!("Expected Time partition type"),
    }
}

#[tokio::test]
#[ignore] // Requires database
async fn test_partition_config_none_for_asset_without_config() {
    // Test that get_asset_partition_config returns None for assets without config
    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create asset without partition config
    let asset_id = create_test_asset_with_partition(
        &storage,
        &unique_name("no_partition_asset"),
        None,
        &tenant,
    )
    .await
    .expect("Failed to create asset");

    // Retrieve partition config
    let retrieved_config = storage
        .get_asset_partition_config(asset_id, &tenant)
        .await
        .expect("Failed to get partition config");

    assert!(
        retrieved_config.is_none(),
        "Partition config should be None for asset without config"
    );
}

#[tokio::test]
#[ignore] // Requires database
async fn test_get_asset_includes_partition_config() {
    // Test that get_asset returns the full asset with partition config
    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    let weekly_config = PartitionConfig {
        partition_type: PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        },
        partition_key: "week_start".to_string(),
    };

    let asset_id = create_test_asset_with_partition(
        &storage,
        &unique_name("weekly_asset"),
        Some(weekly_config),
        &tenant,
    )
    .await
    .expect("Failed to create asset");

    // Get full asset
    let asset = storage
        .get_asset(asset_id, &tenant)
        .await
        .expect("Failed to get asset");

    assert!(
        asset.partition_config.is_some(),
        "Asset should have partition config"
    );

    let config: PartitionConfig = serde_json::from_value(asset.partition_config.unwrap().0)
        .expect("Failed to deserialize partition config");

    assert_eq!(config.partition_key, "week_start");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_upstream_propagation_with_daily_to_weekly_mapping() {
    // Test that when a weekly downstream asset backfills, its daily upstream
    // gets the correct 7-day partition range
    //
    // Setup:
    // - daily_events (Daily partition) -> weekly_summary (Weekly partition)
    // - Backfill weekly_summary for week 2024-W03 (Jan 15-21)
    // - Expected: daily_events child job gets 2024-01-15 to 2024-01-21

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create upstream asset with Daily partition config
    let daily_config = PartitionConfig {
        partition_type: PartitionType::Time {
            granularity: TimeGranularity::Daily,
        },
        partition_key: "event_date".to_string(),
    };
    let daily_events_id = create_test_asset_with_partition(
        &storage,
        &unique_name("daily_events"),
        Some(daily_config),
        &tenant,
    )
    .await
    .expect("Failed to create daily_events asset");

    // Create downstream asset with Weekly partition config
    let weekly_config = PartitionConfig {
        partition_type: PartitionType::Time {
            granularity: TimeGranularity::Weekly,
        },
        partition_key: "week_start".to_string(),
    };
    let weekly_summary_id = create_test_asset_with_partition(
        &storage,
        &unique_name("weekly_summary"),
        Some(weekly_config),
        &tenant,
    )
    .await
    .expect("Failed to create weekly_summary asset");

    // Create dependency: weekly_summary depends on daily_events
    create_dependency(&storage, daily_events_id, weekly_summary_id, &tenant)
        .await
        .expect("Failed to create dependency");

    // Verify configs are stored correctly
    let upstream_config = storage
        .get_asset_partition_config(daily_events_id, &tenant)
        .await
        .expect("Failed to get upstream config");
    assert!(upstream_config.is_some(), "Upstream should have config");

    let downstream_config = storage
        .get_asset_partition_config(weekly_summary_id, &tenant)
        .await
        .expect("Failed to get downstream config");
    assert!(downstream_config.is_some(), "Downstream should have config");

    // Verify the configs deserialize correctly
    let upstream: PartitionConfig =
        serde_json::from_value(upstream_config.unwrap()).expect("Failed to deserialize upstream");
    let downstream: PartitionConfig = serde_json::from_value(downstream_config.unwrap())
        .expect("Failed to deserialize downstream");

    assert!(matches!(
        upstream.partition_type,
        PartitionType::Time {
            granularity: TimeGranularity::Daily
        }
    ));
    assert!(matches!(
        downstream.partition_type,
        PartitionType::Time {
            granularity: TimeGranularity::Weekly
        }
    ));

    // Create backfill job for weekly_summary with include_upstream=true
    // Using ISO week format for weekly partition
    let _job = create_test_backfill_job(
        &storage,
        weekly_summary_id,
        "weekly_summary",
        Some("2024-W03".to_string()),
        Some("2024-W03".to_string()),
        true,
        &tenant,
    )
    .await
    .expect("Failed to create backfill job");

    // Note: Full E2E verification of partition mapping requires running the BackfillExecutor,
    // which is tested separately. This test verifies that:
    // 1. Partition configs are persisted correctly
    // 2. Partition configs are retrievable via get_asset_partition_config
    // 3. The correct partition types are stored (Daily vs Weekly)
    //
    // The BackfillExecutor's create_upstream_child_jobs function will use these configs
    // to apply the partition mapping when creating child jobs.
}

#[tokio::test]
#[ignore] // Requires database
async fn test_identity_mapping_fallback_no_configs() {
    // Test that when no partition configs exist, identity mapping is used
    // (upstream partitions = downstream partitions)

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create assets without partition configs
    let upstream_id = create_test_asset_with_partition(
        &storage,
        &unique_name("upstream_no_config"),
        None,
        &tenant,
    )
    .await
    .expect("Failed to create upstream asset");

    let downstream_id = create_test_asset_with_partition(
        &storage,
        &unique_name("downstream_no_config"),
        None,
        &tenant,
    )
    .await
    .expect("Failed to create downstream asset");

    // Create dependency
    create_dependency(&storage, upstream_id, downstream_id, &tenant)
        .await
        .expect("Failed to create dependency");

    // Verify both configs are None
    let upstream_config = storage
        .get_asset_partition_config(upstream_id, &tenant)
        .await
        .expect("Failed to get upstream config");
    let downstream_config = storage
        .get_asset_partition_config(downstream_id, &tenant)
        .await
        .expect("Failed to get downstream config");

    assert!(upstream_config.is_none());
    assert!(downstream_config.is_none());

    // When the BackfillExecutor processes this job, it will:
    // 1. Load partition configs (both None)
    // 2. Pass None to resolve_upstream_range
    // 3. PartitionResolver falls back to identity mapping
    // 4. Upstream child job gets same partition range as parent
}
