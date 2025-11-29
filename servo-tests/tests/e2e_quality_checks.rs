//! End-to-end data quality check tests
//!
//! These tests validate the complete data quality check lifecycle from
//! check creation to result storage and retrieval.
//!
//! ## Running these tests
//! ```bash
//! # Set up test database
//! export TEST_DATABASE_URL="postgresql://servo_app:servo_app@localhost:5432/servo_test"
//!
//! # Run E2E tests
//! cargo test -p servo-tests --test e2e_quality_checks -- --ignored --test-threads=1
//! ```

use chrono::Utc;
use servo_runtime::{orchestrator::ExecutionOrchestrator, retry::RetryPolicy};
use servo_storage::models::{AssetCheckModel, AssetModel, CheckResultModel, WorkflowModel};
use servo_storage::{postgres::PostgresStorage, TenantId};
use sqlx::types::Json;
use std::sync::Arc;
use uuid::Uuid;

/// E2E test harness for quality check testing
struct QualityTestHarness {
    storage: Arc<PostgresStorage>,
    orchestrator: ExecutionOrchestrator,
    tenant: TenantId,
}

impl QualityTestHarness {
    async fn setup() -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .map_err(|_| "TEST_DATABASE_URL or DATABASE_URL must be set for E2E tests")?;

        let storage = Arc::new(PostgresStorage::new(&database_url).await?);
        let tenant = TenantId::new(format!("quality-test-{}", Uuid::new_v4()));
        let retry_policy = RetryPolicy::default();
        let orchestrator = ExecutionOrchestrator::new(storage.clone(), retry_policy);

        Ok(Self {
            storage,
            orchestrator,
            tenant,
        })
    }

    async fn create_workflow(
        &self,
        name: &str,
        description: &str,
    ) -> Result<Uuid, Box<dyn std::error::Error>> {
        let workflow_id = Uuid::new_v4();
        let workflow = WorkflowModel {
            id: workflow_id,
            name: name.to_string(),
            description: Some(description.to_string()),
            owner: Some("test-owner".to_string()),
            tags: Json(vec![]),
            tenant_id: Some(self.tenant.as_str().to_string()),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.storage
            .create_workflow(&workflow, &self.tenant)
            .await?;
        Ok(workflow_id)
    }

    async fn run_workflow(&self, workflow_id: Uuid) -> Result<Uuid, Box<dyn std::error::Error>> {
        let execution_id = self
            .orchestrator
            .start_execution(workflow_id, &self.tenant, None)
            .await?;
        Ok(execution_id)
    }
}

/// Test the complete check lifecycle: create asset, create check, query check
#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -p servo-tests --test e2e_quality_checks -- --ignored
async fn test_check_lifecycle_create_and_query() {
    let harness = QualityTestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // 1. Create an asset
    let asset_id = Uuid::new_v4();
    let asset = AssetModel {
        id: asset_id,
        name: "quality-test-asset".to_string(),
        description: Some("Test asset for quality checks".to_string()),
        asset_type: "table".to_string(),
        owner: Some("qa-team".to_string()),
        tags: Json(vec!["test".to_string()]),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset(&asset, &harness.tenant)
        .await
        .expect("Failed to create asset");

    // 2. Create a data quality check for the asset
    let check_id = Uuid::new_v4();
    let check = AssetCheckModel {
        id: check_id,
        name: "not_null_email".to_string(),
        description: Some("Email column must not contain nulls".to_string()),
        asset_id,
        check_type: Json(serde_json::json!({
            "type": "not_null",
            "columns": ["email"]
        })),
        severity: "error".to_string(),
        blocking: true,
        enabled: true,
        tags: Json(vec!["pii".to_string(), "email".to_string()]),
        owner: Some("qa-team".to_string()),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset_check(&check, &harness.tenant)
        .await
        .expect("Failed to create asset check");

    // 3. Verify check was created
    let fetched_check = harness
        .storage
        .get_asset_check(check_id, &harness.tenant)
        .await
        .expect("Failed to fetch asset check");

    assert_eq!(fetched_check.name, "not_null_email");
    assert!(fetched_check.blocking);
    assert_eq!(fetched_check.severity, "error");

    // 4. List checks for the asset
    let checks = harness
        .storage
        .list_asset_checks(asset_id, &harness.tenant, true)
        .await
        .expect("Failed to list asset checks");

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].id, check_id);
}

/// Test check result storage and retrieval
#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -p servo-tests --test e2e_quality_checks -- --ignored
async fn test_check_result_storage() {
    let harness = QualityTestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Setup: Create workflow, asset, check, and execution
    let workflow_id = harness
        .create_workflow("result-test-workflow", "Test workflow for check results")
        .await
        .expect("Failed to create workflow");

    let asset_id = Uuid::new_v4();
    let asset = AssetModel {
        id: asset_id,
        name: "result-test-asset".to_string(),
        description: Some("Test asset for check results".to_string()),
        asset_type: "table".to_string(),
        owner: None,
        tags: Json(vec![]),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset(&asset, &harness.tenant)
        .await
        .expect("Failed to create asset");

    let check_id = Uuid::new_v4();
    let check = AssetCheckModel {
        id: check_id,
        name: "unique_user_id".to_string(),
        description: Some("User ID must be unique".to_string()),
        asset_id,
        check_type: Json(serde_json::json!({
            "type": "unique",
            "columns": ["user_id"]
        })),
        severity: "warning".to_string(),
        blocking: false,
        enabled: true,
        tags: Json(vec![]),
        owner: None,
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset_check(&check, &harness.tenant)
        .await
        .expect("Failed to create asset check");

    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start execution");

    // Create check result
    let result_id = Uuid::new_v4();
    let check_result = CheckResultModel {
        id: result_id,
        check_id,
        execution_id,
        asset_id,
        outcome: "passed".to_string(),
        severity: "warning".to_string(),
        blocking: false,
        failed_row_count: Some(0),
        total_row_count: Some(1000),
        error_message: None,
        failed_samples: None,
        duration_ms: 45,
        executed_at: Utc::now(),
        metadata: Some(Json(serde_json::json!({
            "query_time_ms": 12,
            "rows_scanned": 1000
        }))),
        tenant_id: Some(harness.tenant.as_str().to_string()),
    };

    harness
        .storage
        .create_check_result(&check_result, &harness.tenant)
        .await
        .expect("Failed to create check result");

    // Retrieve and verify via execution results list
    let results = harness
        .storage
        .list_check_results_for_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to list check results");

    assert_eq!(results.len(), 1);
    let fetched_result = &results[0];

    assert_eq!(fetched_result.id, result_id);
    assert_eq!(fetched_result.outcome, "passed");
    assert_eq!(fetched_result.failed_row_count, Some(0));
    assert_eq!(fetched_result.total_row_count, Some(1000));
    assert_eq!(fetched_result.duration_ms, 45);
}

/// Test failed check result with PII samples (verifies storage accepts filtered data)
#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -p servo-tests --test e2e_quality_checks -- --ignored
async fn test_failed_check_result_with_samples() {
    let harness = QualityTestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Setup
    let workflow_id = harness
        .create_workflow("failed-check-workflow", "Test workflow for failed checks")
        .await
        .expect("Failed to create workflow");

    let asset_id = Uuid::new_v4();
    let asset = AssetModel {
        id: asset_id,
        name: "pii-test-asset".to_string(),
        description: Some("Test asset for PII samples".to_string()),
        asset_type: "table".to_string(),
        owner: None,
        tags: Json(vec!["pii".to_string()]),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset(&asset, &harness.tenant)
        .await
        .expect("Failed to create asset");

    let check_id = Uuid::new_v4();
    let check = AssetCheckModel {
        id: check_id,
        name: "valid_email_format".to_string(),
        description: Some("Email must be valid format".to_string()),
        asset_id,
        check_type: Json(serde_json::json!({
            "type": "regex",
            "column": "email",
            "pattern": r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
        })),
        severity: "error".to_string(),
        blocking: true,
        enabled: true,
        tags: Json(vec!["pii".to_string()]),
        owner: None,
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset_check(&check, &harness.tenant)
        .await
        .expect("Failed to create asset check");

    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start execution");

    // Create failed check result with PII-filtered samples
    // Note: In production, PII filter would have replaced actual emails with [EMAIL]
    let result_id = Uuid::new_v4();
    let check_result = CheckResultModel {
        id: result_id,
        check_id,
        execution_id,
        asset_id,
        outcome: "failed".to_string(),
        severity: "error".to_string(),
        blocking: true,
        failed_row_count: Some(5),
        total_row_count: Some(100),
        error_message: Some("5 rows have invalid email format".to_string()),
        failed_samples: Some(Json(serde_json::json!([
            {"row_id": 42, "email": "[EMAIL]", "reason": "missing @ symbol"},
            {"row_id": 67, "email": "[EMAIL]", "reason": "invalid domain"}
        ]))),
        duration_ms: 123,
        executed_at: Utc::now(),
        metadata: None,
        tenant_id: Some(harness.tenant.as_str().to_string()),
    };

    harness
        .storage
        .create_check_result(&check_result, &harness.tenant)
        .await
        .expect("Failed to create check result");

    // Verify failed result via execution results list
    let results = harness
        .storage
        .list_check_results_for_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to list check results");

    let fetched_result = results.iter().find(|r| r.id == result_id).unwrap();

    assert_eq!(fetched_result.outcome, "failed");
    assert!(fetched_result.blocking);
    assert_eq!(fetched_result.failed_row_count, Some(5));
    assert!(fetched_result.failed_samples.is_some());

    // Verify samples contain redacted PII
    let samples = fetched_result.failed_samples.as_ref().unwrap();
    let samples_array = samples.0.as_array().unwrap();
    assert_eq!(samples_array.len(), 2);
    assert_eq!(samples_array[0]["email"], "[EMAIL]");
}

/// Test batch check result creation
#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -p servo-tests --test e2e_quality_checks -- --ignored
async fn test_batch_check_result_creation() {
    let harness = QualityTestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Setup
    let workflow_id = harness
        .create_workflow("batch-result-workflow", "Test workflow for batch results")
        .await
        .expect("Failed to create workflow");

    let asset_id = Uuid::new_v4();
    let asset = AssetModel {
        id: asset_id,
        name: "batch-test-asset".to_string(),
        description: Some("Test asset for batch results".to_string()),
        asset_type: "table".to_string(),
        owner: None,
        tags: Json(vec![]),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset(&asset, &harness.tenant)
        .await
        .expect("Failed to create asset");

    // Create multiple checks
    let check_ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    let check_names = ["not_null_order_id", "positive_amount", "valid_status"];

    for (i, check_id) in check_ids.iter().enumerate() {
        let check = AssetCheckModel {
            id: *check_id,
            name: check_names[i].to_string(),
            description: None,
            asset_id,
            check_type: Json(serde_json::json!({"type": "custom"})),
            severity: "warning".to_string(),
            blocking: false,
            enabled: true,
            tags: Json(vec![]),
            owner: None,
            tenant_id: Some(harness.tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        harness
            .storage
            .create_asset_check(&check, &harness.tenant)
            .await
            .expect("Failed to create asset check");
    }

    let execution_id = harness
        .run_workflow(workflow_id)
        .await
        .expect("Failed to start execution");

    // Create batch of check results
    let results: Vec<CheckResultModel> = check_ids
        .iter()
        .enumerate()
        .map(|(i, check_id)| CheckResultModel {
            id: Uuid::new_v4(),
            check_id: *check_id,
            execution_id,
            asset_id,
            outcome: if i == 1 { "failed" } else { "passed" }.to_string(),
            severity: "warning".to_string(),
            blocking: false,
            failed_row_count: if i == 1 { Some(3) } else { Some(0) },
            total_row_count: Some(500),
            error_message: if i == 1 {
                Some("3 orders have negative amounts".to_string())
            } else {
                None
            },
            failed_samples: None,
            duration_ms: (i as i64 + 1) * 10,
            executed_at: Utc::now(),
            metadata: None,
            tenant_id: Some(harness.tenant.as_str().to_string()),
        })
        .collect();

    harness
        .storage
        .create_check_results_batch(&results, &harness.tenant)
        .await
        .expect("Failed to create batch check results");

    // Verify results were created
    let execution_results = harness
        .storage
        .list_check_results_for_execution(execution_id, &harness.tenant)
        .await
        .expect("Failed to list check results");

    assert_eq!(execution_results.len(), 3);

    let passed_count = execution_results
        .iter()
        .filter(|r| r.outcome == "passed")
        .count();
    let failed_count = execution_results
        .iter()
        .filter(|r| r.outcome == "failed")
        .count();

    assert_eq!(passed_count, 2);
    assert_eq!(failed_count, 1);
}

/// Test check filtering by enabled status
#[tokio::test]
#[ignore] // Requires test database - run with: cargo test -p servo-tests --test e2e_quality_checks -- --ignored
async fn test_check_enabled_filtering() {
    let harness = QualityTestHarness::setup()
        .await
        .expect("Failed to setup test harness");

    // Setup
    let asset_id = Uuid::new_v4();
    let asset = AssetModel {
        id: asset_id,
        name: "filter-test-asset".to_string(),
        description: Some("Test asset for enabled filtering".to_string()),
        asset_type: "table".to_string(),
        owner: None,
        tags: Json(vec![]),
        tenant_id: Some(harness.tenant.as_str().to_string()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    harness
        .storage
        .create_asset(&asset, &harness.tenant)
        .await
        .expect("Failed to create asset");

    // Create enabled and disabled checks
    for (i, enabled) in [true, false, true].iter().enumerate() {
        let check = AssetCheckModel {
            id: Uuid::new_v4(),
            name: format!("check_{}", i),
            description: None,
            asset_id,
            check_type: Json(serde_json::json!({"type": "custom"})),
            severity: "info".to_string(),
            blocking: false,
            enabled: *enabled,
            tags: Json(vec![]),
            owner: None,
            tenant_id: Some(harness.tenant.as_str().to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        harness
            .storage
            .create_asset_check(&check, &harness.tenant)
            .await
            .expect("Failed to create asset check");
    }

    // Query enabled only
    let enabled_checks = harness
        .storage
        .list_asset_checks(asset_id, &harness.tenant, true)
        .await
        .expect("Failed to list enabled checks");

    assert_eq!(enabled_checks.len(), 2);

    // Query all checks
    let all_checks = harness
        .storage
        .list_asset_checks(asset_id, &harness.tenant, false)
        .await
        .expect("Failed to list all checks");

    assert_eq!(all_checks.len(), 3);
}
