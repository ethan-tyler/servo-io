//! Integration tests for upstream propagation in backfill jobs
//!
//! These tests validate the complete upstream propagation lifecycle:
//! - Discovering upstream assets
//! - Creating child backfill jobs
//! - Parent job waiting for upstream completion
//! - State transitions when children complete/fail
//! - Job tree cancellation
//!
//! Run with: cargo test --package servo-runtime --test upstream_propagation_integration -- --ignored

use chrono::Utc;
use servo_storage::{BackfillJobModel, CreateUpstreamChildJobParams, PostgresStorage, TenantId};
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

/// Create a test asset in the database
async fn create_test_asset(
    storage: &PostgresStorage,
    name: &str,
    tenant: &TenantId,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let asset = servo_storage::AssetModel {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: Some(format!("Test asset: {}", name)),
        asset_type: "table".to_string(),
        owner: Some("test".to_string()),
        tags: Json(vec![]),
        partition_config: None,
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
    // create_asset_dependency(upstream, downstream) means "downstream depends on upstream"
    storage
        .create_asset_dependency(upstream_id, downstream_id, "data", tenant)
        .await?;
    Ok(())
}

/// Create a test backfill job
async fn create_test_backfill_job(
    storage: &PostgresStorage,
    asset_id: Uuid,
    asset_name: &str,
    include_upstream: bool,
    max_upstream_depth: i32,
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
        partition_start: Some("2024-01-01".to_string()),
        partition_end: Some("2024-01-10".to_string()),
        partition_keys: Json(vec![]),
        total_partitions: 10,
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
        max_upstream_depth,
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
async fn test_discover_upstream_assets_linear_chain() {
    // Setup: A -> B -> C (C depends on B, B depends on A)
    // Action: Discover upstream of C with max_depth=10
    // Assert: Returns [A, B] with correct depths

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create assets
    let asset_a = create_test_asset(&storage, &unique_name("asset_a"), &tenant)
        .await
        .unwrap();
    let asset_b = create_test_asset(&storage, &unique_name("asset_b"), &tenant)
        .await
        .unwrap();
    let asset_c = create_test_asset(&storage, &unique_name("asset_c"), &tenant)
        .await
        .unwrap();

    // Create dependencies: C -> B -> A
    create_dependency(&storage, asset_a, asset_b, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_b, asset_c, &tenant)
        .await
        .unwrap();

    // Discover upstream of C
    let upstream = storage
        .discover_upstream_assets(asset_c, 10, &tenant)
        .await
        .expect("Failed to discover upstream");

    // Should find B (depth 1) and A (depth 2)
    assert_eq!(upstream.len(), 2);

    // Find B and A in results
    let b_entry = upstream.iter().find(|(id, _, _)| *id == asset_b);
    let a_entry = upstream.iter().find(|(id, _, _)| *id == asset_a);

    assert!(b_entry.is_some(), "Should find asset B in upstream");
    assert!(a_entry.is_some(), "Should find asset A in upstream");

    // Check depths
    assert_eq!(b_entry.unwrap().2, 1, "Asset B should be at depth 1");
    assert_eq!(a_entry.unwrap().2, 2, "Asset A should be at depth 2");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_discover_upstream_assets_depth_limit() {
    // Setup: A -> B -> C -> D (4 levels)
    // Action: Discover upstream of D with max_depth=2
    // Assert: Only C and B returned, not A

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create assets
    let asset_a = create_test_asset(&storage, &unique_name("asset_a"), &tenant)
        .await
        .unwrap();
    let asset_b = create_test_asset(&storage, &unique_name("asset_b"), &tenant)
        .await
        .unwrap();
    let asset_c = create_test_asset(&storage, &unique_name("asset_c"), &tenant)
        .await
        .unwrap();
    let asset_d = create_test_asset(&storage, &unique_name("asset_d"), &tenant)
        .await
        .unwrap();

    // Create dependencies: D -> C -> B -> A
    create_dependency(&storage, asset_a, asset_b, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_b, asset_c, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_c, asset_d, &tenant)
        .await
        .unwrap();

    // Discover upstream of D with depth limit of 2
    let upstream = storage
        .discover_upstream_assets(asset_d, 2, &tenant)
        .await
        .expect("Failed to discover upstream");

    // Should only find C (depth 1) and B (depth 2), not A (depth 3)
    assert_eq!(upstream.len(), 2);

    let has_a = upstream.iter().any(|(id, _, _)| *id == asset_a);
    let has_b = upstream.iter().any(|(id, _, _)| *id == asset_b);
    let has_c = upstream.iter().any(|(id, _, _)| *id == asset_c);

    assert!(
        !has_a,
        "Asset A should NOT be in results (beyond depth limit)"
    );
    assert!(has_b, "Asset B should be in results");
    assert!(has_c, "Asset C should be in results");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_discover_upstream_assets_diamond_pattern() {
    // Setup: Diamond pattern
    //     A
    //    / \
    //   B   C
    //    \ /
    //     D
    // Action: Discover upstream of D
    // Assert: Returns A, B, C (A appears once, not twice)

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create assets
    let asset_a = create_test_asset(&storage, &unique_name("asset_a"), &tenant)
        .await
        .unwrap();
    let asset_b = create_test_asset(&storage, &unique_name("asset_b"), &tenant)
        .await
        .unwrap();
    let asset_c = create_test_asset(&storage, &unique_name("asset_c"), &tenant)
        .await
        .unwrap();
    let asset_d = create_test_asset(&storage, &unique_name("asset_d"), &tenant)
        .await
        .unwrap();

    // Create dependencies: D -> B -> A, D -> C -> A
    create_dependency(&storage, asset_a, asset_b, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_a, asset_c, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_b, asset_d, &tenant)
        .await
        .unwrap();
    create_dependency(&storage, asset_c, asset_d, &tenant)
        .await
        .unwrap();

    // Discover upstream of D
    let upstream = storage
        .discover_upstream_assets(asset_d, 10, &tenant)
        .await
        .expect("Failed to discover upstream");

    // Should find B, C (depth 1) and A (depth 2) - A should appear only once
    assert_eq!(upstream.len(), 3);

    // Count occurrences of each asset
    let a_count = upstream.iter().filter(|(id, _, _)| *id == asset_a).count();
    assert_eq!(a_count, 1, "Asset A should appear exactly once");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_create_upstream_child_job() {
    // Test creating a child backfill job for an upstream asset

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create parent asset and job
    let parent_asset = create_test_asset(&storage, &unique_name("parent"), &tenant)
        .await
        .unwrap();
    let parent_job = create_test_backfill_job(&storage, parent_asset, "parent", true, 1, &tenant)
        .await
        .unwrap();

    // Create upstream asset
    let upstream_asset = create_test_asset(&storage, &unique_name("upstream"), &tenant)
        .await
        .unwrap();

    // Create child job params
    let params = CreateUpstreamChildJobParams {
        parent_job_id: parent_job.id,
        asset_id: upstream_asset,
        asset_name: "upstream".to_string(),
        idempotency_key: format!("child-{}", Uuid::new_v4()),
        partition_start: Some("2024-01-01".to_string()),
        partition_end: Some("2024-01-10".to_string()),
        execution_order: 0,
    };

    // Create child job
    let child_job_id = storage
        .create_upstream_child_job(params, &tenant)
        .await
        .expect("Failed to create child job");

    // Verify child job was created with correct parent
    let child_job = storage
        .get_backfill_job(child_job_id, &tenant)
        .await
        .expect("Failed to get child job");

    assert_eq!(child_job.parent_job_id, Some(parent_job.id));
    assert_eq!(child_job.execution_order, 0);
    assert!(!child_job.include_upstream); // Child jobs don't propagate further
}

#[tokio::test]
#[ignore] // Requires database
async fn test_increment_completed_upstream() {
    // Test incrementing the completed upstream count

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create asset and parent job
    let asset = create_test_asset(&storage, &unique_name("asset"), &tenant)
        .await
        .unwrap();
    let job = create_test_backfill_job(&storage, asset, "asset", true, 1, &tenant)
        .await
        .unwrap();

    // Update upstream count first
    storage
        .update_backfill_upstream_count(job.id, 3, &tenant)
        .await
        .expect("Failed to update upstream count");

    // Increment completed upstream
    storage
        .increment_completed_upstream(job.id, &tenant)
        .await
        .expect("Failed to increment completed upstream");

    // Verify count was incremented
    let updated_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");

    assert_eq!(updated_job.upstream_job_count, 3);
    assert_eq!(updated_job.completed_upstream_jobs, 1);
}

#[tokio::test]
#[ignore] // Requires database
async fn test_try_transition_parent_to_pending() {
    // Test that parent transitions to pending when all upstream complete

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create asset and parent job
    let asset = create_test_asset(&storage, &unique_name("asset"), &tenant)
        .await
        .unwrap();
    let job = create_test_backfill_job(&storage, asset, "asset", true, 1, &tenant)
        .await
        .unwrap();

    // Transition to running first (simulating claim)
    storage
        .transition_backfill_job_state(job.id, "pending", "running", None, &tenant)
        .await
        .expect("Failed to transition to running");

    // Transition to waiting_upstream
    storage
        .transition_backfill_job_state(job.id, "running", "waiting_upstream", None, &tenant)
        .await
        .expect("Failed to transition to waiting_upstream");

    // Set upstream count to 2
    storage
        .update_backfill_upstream_count(job.id, 2, &tenant)
        .await
        .expect("Failed to update upstream count");

    // First increment - should not transition yet
    storage
        .increment_completed_upstream(job.id, &tenant)
        .await
        .unwrap();
    let transitioned = storage
        .try_transition_parent_to_pending(job.id, &tenant)
        .await
        .expect("Failed to try transition");
    assert!(!transitioned, "Should not transition with 1/2 complete");

    // Second increment - should transition now
    storage
        .increment_completed_upstream(job.id, &tenant)
        .await
        .unwrap();
    let transitioned = storage
        .try_transition_parent_to_pending(job.id, &tenant)
        .await
        .expect("Failed to try transition");
    assert!(transitioned, "Should transition with 2/2 complete");

    // Verify state
    let updated_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(updated_job.state, "pending");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_cancel_job_tree() {
    // Test cancelling a job tree (parent and children)

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create parent asset and job
    let parent_asset = create_test_asset(&storage, &unique_name("parent"), &tenant)
        .await
        .unwrap();
    let parent_job = create_test_backfill_job(&storage, parent_asset, "parent", true, 1, &tenant)
        .await
        .unwrap();

    // Create two child jobs
    let upstream1 = create_test_asset(&storage, &unique_name("upstream1"), &tenant)
        .await
        .unwrap();
    let upstream2 = create_test_asset(&storage, &unique_name("upstream2"), &tenant)
        .await
        .unwrap();

    let params1 = CreateUpstreamChildJobParams {
        parent_job_id: parent_job.id,
        asset_id: upstream1,
        asset_name: "upstream1".to_string(),
        idempotency_key: format!("child1-{}", Uuid::new_v4()),
        partition_start: None,
        partition_end: None,
        execution_order: 0,
    };
    let child1_id = storage
        .create_upstream_child_job(params1, &tenant)
        .await
        .unwrap();

    let params2 = CreateUpstreamChildJobParams {
        parent_job_id: parent_job.id,
        asset_id: upstream2,
        asset_name: "upstream2".to_string(),
        idempotency_key: format!("child2-{}", Uuid::new_v4()),
        partition_start: None,
        partition_end: None,
        execution_order: 1,
    };
    let child2_id = storage
        .create_upstream_child_job(params2, &tenant)
        .await
        .unwrap();

    // Cancel the job tree
    let cancelled_count = storage
        .cancel_job_tree(parent_job.id, "User requested cancellation", &tenant)
        .await
        .expect("Failed to cancel job tree");

    // Should have cancelled all 3 jobs
    assert_eq!(cancelled_count, 3);

    // Verify all jobs are cancelled
    let parent = storage
        .get_backfill_job(parent_job.id, &tenant)
        .await
        .unwrap();
    let child1 = storage.get_backfill_job(child1_id, &tenant).await.unwrap();
    let child2 = storage.get_backfill_job(child2_id, &tenant).await.unwrap();

    assert_eq!(parent.state, "cancelled");
    assert_eq!(child1.state, "cancelled");
    assert_eq!(child2.state, "cancelled");
}

#[tokio::test]
#[ignore] // Requires database
async fn test_fail_parent_on_child_failure() {
    // Test that parent fails when a child job fails

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create parent asset and job
    let parent_asset = create_test_asset(&storage, &unique_name("parent"), &tenant)
        .await
        .unwrap();
    let parent_job = create_test_backfill_job(&storage, parent_asset, "parent", true, 1, &tenant)
        .await
        .unwrap();

    // Transition parent to waiting_upstream
    storage
        .transition_backfill_job_state(parent_job.id, "pending", "running", None, &tenant)
        .await
        .unwrap();
    storage
        .transition_backfill_job_state(parent_job.id, "running", "waiting_upstream", None, &tenant)
        .await
        .unwrap();

    // Create child job
    let upstream = create_test_asset(&storage, &unique_name("upstream"), &tenant)
        .await
        .unwrap();
    let params = CreateUpstreamChildJobParams {
        parent_job_id: parent_job.id,
        asset_id: upstream,
        asset_name: "upstream".to_string(),
        idempotency_key: format!("child-{}", Uuid::new_v4()),
        partition_start: None,
        partition_end: None,
        execution_order: 0,
    };
    let child_id = storage
        .create_upstream_child_job(params, &tenant)
        .await
        .unwrap();

    // Fail the parent due to child failure
    let parent_failed = storage
        .fail_parent_on_child_failure(parent_job.id, child_id, "upstream", &tenant)
        .await
        .expect("Failed to fail parent");

    assert!(parent_failed);

    // Verify parent is failed
    let parent = storage
        .get_backfill_job(parent_job.id, &tenant)
        .await
        .unwrap();
    assert_eq!(parent.state, "failed");
    assert!(parent.error_message.is_some());
    assert!(parent.error_message.unwrap().contains("upstream"));
}

#[tokio::test]
#[ignore] // Requires database
async fn test_get_child_backfill_jobs() {
    // Test getting all child jobs for a parent

    let storage = setup_test_db().await.expect("Failed to setup test db");
    let tenant = unique_tenant();

    // Create parent asset and job
    let parent_asset = create_test_asset(&storage, &unique_name("parent"), &tenant)
        .await
        .unwrap();
    let parent_job = create_test_backfill_job(&storage, parent_asset, "parent", true, 1, &tenant)
        .await
        .unwrap();

    // Create three child jobs with different execution orders
    for i in 0..3 {
        let upstream =
            create_test_asset(&storage, &unique_name(&format!("upstream{}", i)), &tenant)
                .await
                .unwrap();
        let params = CreateUpstreamChildJobParams {
            parent_job_id: parent_job.id,
            asset_id: upstream,
            asset_name: format!("upstream{}", i),
            idempotency_key: format!("child{}-{}", i, Uuid::new_v4()),
            partition_start: None,
            partition_end: None,
            execution_order: i,
        };
        storage
            .create_upstream_child_job(params, &tenant)
            .await
            .unwrap();
    }

    // Get child jobs
    let children = storage
        .get_child_backfill_jobs(parent_job.id, &tenant)
        .await
        .expect("Failed to get child jobs");

    assert_eq!(children.len(), 3);

    // Verify execution order
    for (i, child) in children.iter().enumerate() {
        assert_eq!(child.parent_job_id, Some(parent_job.id));
        assert_eq!(child.execution_order, i as i32);
    }
}
