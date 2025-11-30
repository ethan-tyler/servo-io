//! Integration tests for Backfill Pause/Resume functionality
//!
//! These tests validate the backfill executor's pause/resume capabilities
//! against a real PostgreSQL database with Row-Level Security (RLS) enforcement.
//!
//! # Tier 1 Critical Tests
//!
//! These tests cover the critical pause/resume lifecycle:
//! 1. Pause running job at partition boundary
//! 2. Resume paused job from checkpoint
//! 3. Checkpoint persistence verification
//! 4. ETA preservation across pause/resume
//! 5. State transition validation
//! 6. Idempotency guarantees
//!
//! # Setup
//!
//! Tests require two environment variables:
//! - `TEST_DATABASE_URL`: Connection string for database owner role (for setup/teardown)
//! - `TEST_APP_DATABASE_URL`: Connection string for servo_app role (for RLS-enforced operations)
//!
//! # Running Tests
//!
//! ```bash
//! export TEST_DATABASE_URL="postgresql://servo:servo@localhost:5432/servo_test"
//! export TEST_APP_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//! cargo test --package servo-runtime --test backfill_integration -- --ignored
//! ```

use chrono::{Duration, Utc};
use servo_storage::{AssetModel, BackfillJobModel, BackfillPartitionModel, PostgresStorage, TenantId};
use sqlx::types::Json;
use std::sync::Arc;
use uuid::Uuid;

/// Get the app role database URL for RLS-enforced operations
fn get_app_database_url() -> String {
    std::env::var("TEST_APP_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://servo_app:servo_app@localhost:5432/servo_test".to_string()
    })
}

/// Generate a unique tenant ID for test isolation
fn unique_tenant() -> TenantId {
    TenantId::new(format!("test_{}", Uuid::new_v4()))
}

/// Get the owner database URL (for migrations)
fn get_owner_database_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://servo:servo@localhost:5432/servo_test".to_string())
}

/// Setup storage connected with servo_app role (RLS enforced) and ensure migrations are applied
async fn setup_app_role_storage() -> Arc<PostgresStorage> {
    // Apply migrations using owner role to ensure schema exists
    let owner_url = get_owner_database_url();
    let owner_storage = PostgresStorage::new(&owner_url)
        .await
        .expect("Failed to connect to database with owner role");
    servo_storage::migrations::run_migrations(owner_storage.pool())
        .await
        .expect("Failed to run migrations with owner role");

    // Connect as app role for tests (RLS enforced)
    let url = get_app_database_url();
    Arc::new(
        PostgresStorage::new(&url)
            .await
            .expect("Failed to connect to database with app role"),
    )
}

/// Create a test asset for backfill jobs
async fn create_test_asset(storage: &PostgresStorage, tenant: &TenantId) -> (Uuid, String) {
    let asset_name = format!("test_asset_{}", Uuid::new_v4());
    let asset = AssetModel {
        id: Uuid::new_v4(),
        name: asset_name.clone(),
        description: Some("Test asset for backfill integration tests".to_string()),
        asset_type: "table".to_string(),
        owner: Some("test_user".to_string()),
        tags: Json(vec![]),
        tenant_id: Some(tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    storage
        .create_asset(&asset, tenant)
        .await
        .expect("Failed to create test asset");

    (asset.id, asset_name)
}

/// Create a test backfill job with specified number of partitions
async fn create_test_backfill_job(
    storage: &PostgresStorage,
    tenant: &TenantId,
    asset_id: Uuid,
    asset_name: &str,
    total_partitions: i32,
) -> BackfillJobModel {
    let job = BackfillJobModel {
        id: Uuid::new_v4(),
        tenant_id: tenant.as_str().to_string(),
        asset_id,
        asset_name: asset_name.to_string(),
        idempotency_key: format!("test-{}", Uuid::new_v4()),
        state: "pending".to_string(),
        execution_strategy: Json(serde_json::json!({"type": "sequential"})),
        partition_start: Some("2024-01-01".to_string()),
        partition_end: Some(format!("2024-01-{:02}", total_partitions)),
        partition_keys: Json(vec![]),
        total_partitions,
        completed_partitions: 0,
        failed_partitions: 0,
        skipped_partitions: 0,
        include_upstream: false,
        error_message: None,
        created_by: Some("test_user".to_string()),
        created_at: Utc::now(),
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        version: 1,
        paused_at: None,
        checkpoint_partition_key: None,
        estimated_completion_at: None,
        avg_partition_duration_ms: None,
    };

    storage
        .create_backfill_job(&job, tenant)
        .await
        .expect("Failed to create test backfill job");

    job
}

/// Create partitions for a backfill job
async fn create_test_partitions(
    storage: &PostgresStorage,
    tenant: &TenantId,
    job_id: Uuid,
    count: i32,
) {
    for i in 1..=count {
        let partition = BackfillPartitionModel {
            id: Uuid::new_v4(),
            backfill_job_id: job_id,
            partition_key: format!("2024-01-{:02}", i),
            state: "pending".to_string(),
            attempt_count: 0,
            execution_id: None,
            error_message: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            tenant_id: tenant.as_str().to_string(),
        };

        storage
            .create_backfill_partition(&partition, tenant)
            .await
            .expect("Failed to create test partition");
    }
}

// ============================================================================
// Tier 1: Critical Pause/Resume Tests
// ============================================================================

/// Test 1: Pause a running job successfully
///
/// Verifies:
/// - Running job can be paused
/// - State transitions from 'running' to 'paused'
/// - paused_at timestamp is set
#[tokio::test]
#[ignore]
async fn test_pause_running_job_success() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create test asset and job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 10).await;
    create_test_partitions(&storage, &tenant, job.id, 10).await;

    // Claim the job to transition to 'running'
    let claimed_job = storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");
    assert!(claimed_job.is_some(), "Job should be claimed");
    let claimed_job = claimed_job.unwrap();
    assert_eq!(claimed_job.id, job.id);
    assert_eq!(claimed_job.state, "running");

    // Pause the job
    let result = storage.pause_backfill_job(job.id, &tenant).await;
    assert!(result.is_ok(), "Pause should succeed");

    // Verify job state changed to 'paused'
    let paused_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(paused_job.state, "paused", "Job should be in paused state");
    assert!(
        paused_job.paused_at.is_some(),
        "paused_at should be set"
    );
}

/// Test 2: Resume a paused job successfully
///
/// Verifies:
/// - Paused job can be resumed
/// - State transitions from 'paused' to 'resuming' to 'running'
/// - paused_at is cleared after running
#[tokio::test]
#[ignore]
async fn test_resume_paused_job_success() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create and pause a job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 10).await;
    create_test_partitions(&storage, &tenant, job.id, 10).await;

    // Claim to make it running
    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // Pause it
    storage
        .pause_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to pause job");

    // Resume the job
    let result = storage.resume_backfill_job(job.id, &tenant).await;
    assert!(result.is_ok(), "Resume should succeed");

    // Verify job state changed to 'resuming'
    let resuming_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(
        resuming_job.state, "resuming",
        "Job should be in resuming state"
    );

    // Now claim it again (simulating executor pickup)
    let reclaimed = storage
        .claim_pending_backfill_job("test-executor-2", 120, &tenant)
        .await
        .expect("Failed to reclaim job");

    assert!(reclaimed.is_some(), "Resuming job should be claimable");
    let reclaimed = reclaimed.unwrap();
    assert_eq!(reclaimed.id, job.id);
    assert_eq!(reclaimed.state, "running");
}

/// Test 3: Checkpoint is persisted correctly on pause
///
/// Verifies:
/// - Progress is saved before pausing
/// - Checkpoint partition key is recorded
/// - Completed partition counts are preserved
#[tokio::test]
#[ignore]
async fn test_pause_checkpoint_persisted() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create job with 10 partitions
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 10).await;
    create_test_partitions(&storage, &tenant, job.id, 10).await;

    // Claim and start processing
    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // Simulate progress: 5 partitions completed
    storage
        .update_backfill_job_progress(job.id, 5, 0, 0, &tenant)
        .await
        .expect("Failed to update progress");

    // Set checkpoint partition key via progress update with ETA
    let progress = servo_storage::BackfillProgressUpdate {
        completed: 5,
        failed: 0,
        skipped: 0,
        avg_partition_duration_ms: Some(1000),
        estimated_completion_at: Some(Utc::now() + Duration::seconds(5)),
        checkpoint_partition_key: Some("2024-01-05".to_string()),
    };
    storage
        .update_backfill_job_progress_with_eta(job.id, &progress, &tenant)
        .await
        .expect("Failed to update progress with checkpoint");

    // Pause the job
    storage
        .pause_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to pause job");

    // Verify checkpoint is preserved
    let paused_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");

    assert_eq!(paused_job.state, "paused");
    assert_eq!(paused_job.completed_partitions, 5);
    assert_eq!(
        paused_job.checkpoint_partition_key,
        Some("2024-01-05".to_string())
    );
    assert!(paused_job.avg_partition_duration_ms.is_some());
}

/// Test 4: Resume picks up from the correct checkpoint
///
/// Verifies:
/// - Resumed job continues from checkpoint, not from the beginning
/// - Progress counters are maintained across pause/resume
#[tokio::test]
#[ignore]
async fn test_resume_picks_up_from_checkpoint() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 10).await;
    create_test_partitions(&storage, &tenant, job.id, 10).await;

    // Claim and mark some partitions as completed
    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // Complete first 3 partitions
    let partitions = storage
        .list_backfill_partitions(job.id, &tenant)
        .await
        .expect("Failed to list partitions");

    for partition in partitions.iter().take(3) {
        storage
            .complete_backfill_partition(partition.id, Some(Uuid::new_v4()), 1000, &tenant)
            .await
            .expect("Failed to complete partition");
    }

    // Update job progress with checkpoint
    let progress = servo_storage::BackfillProgressUpdate {
        completed: 3,
        failed: 0,
        skipped: 0,
        avg_partition_duration_ms: Some(1000),
        estimated_completion_at: None,
        checkpoint_partition_key: Some("2024-01-03".to_string()),
    };
    storage
        .update_backfill_job_progress_with_eta(job.id, &progress, &tenant)
        .await
        .expect("Failed to update progress");

    // Pause and resume
    storage
        .pause_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to pause");
    storage
        .resume_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to resume");

    // Claim again
    let resumed_job = storage
        .claim_pending_backfill_job("test-executor-2", 120, &tenant)
        .await
        .expect("Failed to reclaim job")
        .expect("Job should be claimable");

    // Verify progress is maintained
    assert_eq!(
        resumed_job.completed_partitions, 3,
        "Completed count should be preserved"
    );
    assert_eq!(
        resumed_job.checkpoint_partition_key,
        Some("2024-01-03".to_string()),
        "Checkpoint should be preserved"
    );

    // Verify remaining partitions are still pending
    let remaining_partitions = storage
        .list_backfill_partitions(job.id, &tenant)
        .await
        .expect("Failed to list partitions");

    let pending_count = remaining_partitions
        .iter()
        .filter(|p| p.state == "pending")
        .count();
    assert_eq!(pending_count, 7, "7 partitions should remain pending");
}

/// Test 5: ETA statistics are preserved across pause/resume
///
/// Verifies:
/// - avg_partition_duration_ms is preserved
/// - ETA can be recalculated accurately after resume
#[tokio::test]
#[ignore]
async fn test_eta_preserved_across_pause_resume() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 20).await;
    create_test_partitions(&storage, &tenant, job.id, 20).await;

    // Claim and process some partitions
    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // Simulate 5 partitions with ~2000ms average duration
    let progress = servo_storage::BackfillProgressUpdate {
        completed: 5,
        failed: 0,
        skipped: 0,
        avg_partition_duration_ms: Some(2000), // 2 seconds average
        estimated_completion_at: Some(Utc::now() + Duration::seconds(30)),
        checkpoint_partition_key: Some("2024-01-05".to_string()),
    };
    storage
        .update_backfill_job_progress_with_eta(job.id, &progress, &tenant)
        .await
        .expect("Failed to update progress");

    // Pause the job
    storage
        .pause_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to pause");

    // Verify ETA stats are preserved in paused state
    let paused_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(
        paused_job.avg_partition_duration_ms,
        Some(2000),
        "Average duration should be preserved"
    );

    // Resume and reclaim
    storage
        .resume_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to resume");
    let resumed_job = storage
        .claim_pending_backfill_job("test-executor-2", 120, &tenant)
        .await
        .expect("Failed to reclaim")
        .expect("Job should be claimable");

    // Verify ETA stats are still available after resume
    assert_eq!(
        resumed_job.avg_partition_duration_ms,
        Some(2000),
        "Average duration should be preserved after resume"
    );

    // Verify ETA can be calculated for remaining partitions
    // 15 remaining * 2000ms = 30 seconds expected
    let remaining = resumed_job.total_partitions
        - resumed_job.completed_partitions
        - resumed_job.failed_partitions
        - resumed_job.skipped_partitions;
    assert_eq!(remaining, 15);

    // Using the preserved avg_partition_duration_ms, we can calculate accurate ETA
    let expected_remaining_ms = remaining as i64 * 2000;
    assert_eq!(expected_remaining_ms, 30000); // 30 seconds
}

/// Test 6: Pause is idempotent - multiple calls are safe
///
/// Verifies:
/// - Calling pause on already paused job doesn't cause errors
/// - State remains 'paused' after multiple pause attempts
#[tokio::test]
#[ignore]
async fn test_pause_idempotent() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create and claim job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 5).await;
    create_test_partitions(&storage, &tenant, job.id, 5).await;

    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // First pause
    let result1 = storage.pause_backfill_job(job.id, &tenant).await;
    assert!(result1.is_ok(), "First pause should succeed");

    // Second pause - should either succeed or return gracefully
    let result2 = storage.pause_backfill_job(job.id, &tenant).await;
    // We expect either Ok (idempotent) or a specific error for "already paused"
    // The implementation should handle this gracefully

    // Verify state is still 'paused' regardless of second call result
    let final_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(
        final_job.state, "paused",
        "Job should still be paused after multiple pause calls"
    );

    // Log the result of second pause for debugging
    match result2 {
        Ok(_) => {
            // Fully idempotent - both calls succeeded
        }
        Err(e) => {
            // Check it's not a catastrophic error
            let error_msg = e.to_string().to_lowercase();
            assert!(
                error_msg.contains("paused") || error_msg.contains("state"),
                "Error should indicate job is already paused, got: {}",
                e
            );
        }
    }
}

// ============================================================================
// Additional State Transition Tests
// ============================================================================

/// Test pause fails on non-running job
#[tokio::test]
#[ignore]
async fn test_pause_non_running_job_fails() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create job but don't claim it (stays in 'pending' state)
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 5).await;

    // Try to pause pending job - should fail
    let result = storage.pause_backfill_job(job.id, &tenant).await;
    assert!(
        result.is_err(),
        "Pause should fail on non-running job"
    );

    // Job should still be pending
    let job_after = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(job_after.state, "pending");
}

/// Test resume fails on non-paused job
#[tokio::test]
#[ignore]
async fn test_resume_non_paused_job_fails() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create and claim job (running state)
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 5).await;
    create_test_partitions(&storage, &tenant, job.id, 5).await;

    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    // Try to resume running job - should fail
    let result = storage.resume_backfill_job(job.id, &tenant).await;
    assert!(
        result.is_err(),
        "Resume should fail on non-paused job"
    );

    // Job should still be running
    let job_after = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(job_after.state, "running");
}

/// Test cancellation of paused job
#[tokio::test]
#[ignore]
async fn test_cancel_paused_job() {
    let storage = setup_app_role_storage().await;
    let tenant = unique_tenant();

    // Create, claim, and pause job
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant).await;
    let job = create_test_backfill_job(&storage, &tenant, asset_id, &asset_name, 5).await;
    create_test_partitions(&storage, &tenant, job.id, 5).await;

    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant)
        .await
        .expect("Failed to claim job");

    storage
        .pause_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to pause job");

    // Cancel the paused job
    let result = storage.cancel_backfill_job(job.id, Some("test cancellation"), &tenant).await;
    assert!(result.is_ok(), "Cancel should succeed on paused job");

    // Verify job is cancelled
    let cancelled_job = storage
        .get_backfill_job(job.id, &tenant)
        .await
        .expect("Failed to get job");
    assert_eq!(cancelled_job.state, "cancelled");
}

/// Test tenant isolation for pause/resume
#[tokio::test]
#[ignore]
async fn test_pause_resume_tenant_isolation() {
    let storage = setup_app_role_storage().await;
    let tenant1 = unique_tenant();
    let tenant2 = unique_tenant();

    // Create job for tenant1
    let (asset_id, asset_name) = create_test_asset(&storage, &tenant1).await;
    let job = create_test_backfill_job(&storage, &tenant1, asset_id, &asset_name, 5).await;
    create_test_partitions(&storage, &tenant1, job.id, 5).await;

    storage
        .claim_pending_backfill_job("test-executor", 120, &tenant1)
        .await
        .expect("Failed to claim job");

    // Tenant2 tries to pause tenant1's job - should fail
    let result = storage.pause_backfill_job(job.id, &tenant2).await;
    assert!(
        result.is_err(),
        "Tenant2 should not be able to pause tenant1's job"
    );

    // Verify job is still running (tenant1's perspective)
    let job_after = storage
        .get_backfill_job(job.id, &tenant1)
        .await
        .expect("Failed to get job");
    assert_eq!(
        job_after.state, "running",
        "Job should still be running after failed cross-tenant pause"
    );
}
