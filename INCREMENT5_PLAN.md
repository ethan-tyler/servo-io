# Increment 5: Upstream Propagation

## Executive Summary

This increment activates the unused `include_upstream` flag in backfill jobs to automatically backfill upstream dependencies before processing the target asset. When a user creates a backfill with `--include-upstream`, the system will:

1. Discover all upstream assets via the existing lineage tracking
2. Create coordinated child backfill jobs for each upstream asset
3. Execute upstream jobs before the target asset's partitions
4. Track parent-child relationships for observability and cancellation

## Current State Analysis

### Existing Infrastructure (Ready to Use)

| Component | Status | Location |
|-----------|--------|----------|
| `include_upstream` field | EXISTS (always false) | `BackfillJobModel` |
| `get_upstream_dependencies()` | IMPLEMENTED | `postgres.rs:1146` |
| `get_asset_lineage()` | IMPLEMENTED | `postgres.rs:1274` |
| `AssetDependencyModel` | IMPLEMENTED | `models.rs:53` |
| LineageGraph with `ancestors()` | IMPLEMENTED | `servo-lineage/graph.rs` |
| Partition-boundary pause/resume | IMPLEMENTED | Increment 4 |
| EWMA-based ETA tracking | IMPLEMENTED | Increment 4 |

### What Needs Implementation

1. **Database Schema**: Parent-child job relationship tracking
2. **Storage Layer**: Functions for upstream job coordination
3. **CLI**: `--include-upstream` and `--max-upstream-depth` flags
4. **Executor**: Wait for upstream completion before partition processing
5. **Metrics & Observability**: Upstream propagation tracking

## Design Decisions

### Decision 1: Job Hierarchy Model

**Option A**: Single job with all partitions (flat)
- Pros: Simpler state management
- Cons: Loses asset-level granularity, complex ETA calculation

**Option B**: Parent-child job hierarchy (chosen)
- Pros: Per-asset progress tracking, independent cancellation, cleaner state
- Cons: Slightly more complex coordination

**Rationale**: Parent-child model provides better observability and aligns with the "one job per asset" principle established in the existing design.

### Decision 2: Execution Strategy

**Option A**: Parallel upstream execution
- Pros: Faster overall completion
- Cons: Complex dependency resolution, resource contention

**Option B**: Topological order (chosen)
- Pros: Predictable execution, simpler correctness guarantees
- Cons: Serial overhead for deep dependency trees

**Rationale**: Topological ordering ensures upstream data is available when downstream processing begins. Future increments can add parallelism for sibling nodes.

### Decision 3: Partition Key Mapping

**Challenge**: Upstream assets may have different partitioning schemes than downstream.

**Solution**: Use the parent job's partition range to filter upstream partitions that overlap temporally. The executor will:
1. Parse partition keys as dates/timestamps where applicable
2. Filter upstream partitions to those within the parent's date range
3. Fall back to full backfill if partition schemes are incompatible

## Implementation Plan

### Phase 1: Database Schema (Migration 012)

**File**: `servo-storage/migrations/012_add_upstream_propagation.sql`

```sql
-- Add parent-child relationship to backfill_jobs
ALTER TABLE backfill_jobs
  ADD COLUMN parent_job_id UUID REFERENCES backfill_jobs(id) ON DELETE CASCADE,
  ADD COLUMN max_upstream_depth INTEGER DEFAULT 0,
  ADD COLUMN upstream_job_count INTEGER DEFAULT 0,
  ADD COLUMN completed_upstream_jobs INTEGER DEFAULT 0;

-- Index for efficient child lookup
CREATE INDEX idx_backfill_jobs_parent ON backfill_jobs(parent_job_id)
  WHERE parent_job_id IS NOT NULL;

-- Track execution order within a job tree
ALTER TABLE backfill_jobs
  ADD COLUMN execution_order INTEGER DEFAULT 0;

-- Update state constraint to include 'waiting_upstream'
ALTER TABLE backfill_jobs
  DROP CONSTRAINT valid_backfill_state,
  ADD CONSTRAINT valid_backfill_state
    CHECK (state IN ('pending', 'waiting_upstream', 'running', 'paused',
                     'resuming', 'completed', 'failed', 'cancelled'));
```

### Phase 2: Model Updates

**File**: `servo-storage/src/models.rs`

```rust
pub struct BackfillJobModel {
    // ... existing fields ...

    /// Parent job ID for upstream propagation (null for root jobs)
    pub parent_job_id: Option<Uuid>,
    /// Maximum depth for upstream discovery (0 = direct dependencies only)
    pub max_upstream_depth: i32,
    /// Total number of upstream jobs created
    pub upstream_job_count: i32,
    /// Number of completed upstream jobs
    pub completed_upstream_jobs: i32,
    /// Execution order in topological sort (0 = root)
    pub execution_order: i32,
}

/// Summary of upstream job status for a parent job
#[derive(Debug, Clone)]
pub struct UpstreamJobsSummary {
    pub total: i32,
    pub completed: i32,
    pub failed: i32,
    pub running: i32,
    pub pending: i32,
}
```

### Phase 3: Storage Layer Functions

**File**: `servo-storage/src/postgres.rs`

New functions required:

```rust
/// Discover all upstream assets within depth limit
pub async fn discover_upstream_assets(
    &self,
    asset_id: Uuid,
    max_depth: usize,
    tenant_id: &TenantId,
) -> Result<Vec<(Uuid, String, i32)>>  // (asset_id, asset_name, depth)

/// Create child backfill jobs for upstream assets
pub async fn create_upstream_backfill_jobs(
    &self,
    parent_job_id: Uuid,
    upstream_assets: Vec<(Uuid, String, i32)>,
    partition_start: Option<&str>,
    partition_end: Option<&str>,
    tenant_id: &TenantId,
) -> Result<Vec<Uuid>>  // child job IDs

/// Get upstream jobs summary for a parent job
pub async fn get_upstream_jobs_summary(
    &self,
    parent_job_id: Uuid,
    tenant_id: &TenantId,
) -> Result<UpstreamJobsSummary>

/// Check if all upstream jobs are complete (for gating)
pub async fn are_upstream_jobs_complete(
    &self,
    parent_job_id: Uuid,
    tenant_id: &TenantId,
) -> Result<bool>

/// Get next job to process in topological order
pub async fn claim_next_ready_job(
    &self,
    root_job_id: Uuid,
    tenant_id: &TenantId,
) -> Result<Option<BackfillJobModel>>

/// Update upstream completion count on child completion
pub async fn increment_completed_upstream(
    &self,
    parent_job_id: Uuid,
    tenant_id: &TenantId,
) -> Result<()>

/// Cancel all jobs in a tree (cascade)
pub async fn cancel_job_tree(
    &self,
    root_job_id: Uuid,
    reason: &str,
    tenant_id: &TenantId,
) -> Result<i32>  // count of cancelled jobs
```

### Phase 4: CLI Updates

**File**: `servo-cli/src/commands/backfill.rs`

```rust
/// Create a backfill job
#[derive(Parser)]
pub struct CreateCommand {
    // ... existing fields ...

    /// Include upstream dependencies in backfill
    #[arg(long, default_value = "false")]
    include_upstream: bool,

    /// Maximum depth for upstream discovery (0 = direct only, -1 = unlimited)
    #[arg(long, default_value = "1")]
    max_upstream_depth: i32,
}
```

CLI behavior changes:
- `servo backfill create asset_name --include-upstream`: Creates jobs for direct upstreams
- `servo backfill create asset_name --include-upstream --max-upstream-depth 3`: Traverse up to 3 levels
- `servo backfill status <job_id>`: Show child jobs status
- `servo backfill cancel <job_id>`: Cancel entire job tree

### Phase 5: Executor Updates

**File**: `servo-runtime/src/backfill_executor.rs`

```rust
/// Extended job claiming for upstream-aware execution
async fn claim_next_job(&self, tenant_id: &TenantId) -> Result<Option<BackfillJobModel>> {
    // 1. Find jobs in 'pending' or 'resuming' state
    // 2. For jobs with include_upstream=true, check upstream completion
    // 3. Only claim jobs where all upstream jobs are complete
    // 4. Use FOR UPDATE SKIP LOCKED for atomicity
}

/// Process a job with upstream awareness
async fn process_job(&self, job: BackfillJobModel) -> Result<()> {
    if job.include_upstream && job.parent_job_id.is_none() {
        // Root job: discover and create upstream jobs first
        self.create_upstream_jobs(&job).await?;
        // Transition to waiting_upstream state
        self.transition_to_waiting_upstream(&job).await?;
        return Ok(());
    }

    // Regular job processing (existing logic)
    self.process_partitions(&job).await
}

/// Called when a child job completes
async fn on_child_job_complete(&self, child_job: &BackfillJobModel) -> Result<()> {
    if let Some(parent_id) = child_job.parent_job_id {
        // Increment parent's completed_upstream_jobs counter
        self.storage.increment_completed_upstream(parent_id, &self.tenant_id).await?;

        // Check if all upstream complete - transition parent to pending
        if self.storage.are_upstream_jobs_complete(parent_id, &self.tenant_id).await? {
            self.transition_parent_to_pending(parent_id).await?;
        }
    }
    Ok(())
}
```

### Phase 6: State Machine Updates

**New states**:
- `waiting_upstream`: Job created, waiting for upstream jobs to complete

**New transitions**:
- `pending -> waiting_upstream` (when creating upstream jobs)
- `waiting_upstream -> pending` (when all upstream jobs complete)
- `waiting_upstream -> cancelled` (user cancellation)

### Phase 7: Metrics & Observability

**New metrics** (low-cardinality):

```rust
// Gauge: Jobs by state including waiting_upstream
servo_backfill_jobs_active{state="waiting_upstream"}

// Counter: Upstream jobs created
servo_backfill_upstream_jobs_created_total

// Histogram: Time spent waiting for upstream
servo_backfill_upstream_wait_duration_seconds

// Counter: Upstream propagation depth distribution
servo_backfill_upstream_depth{depth="1|2|3|4+"}
```

**Trace spans**:
- `backfill.discover_upstream`
- `backfill.create_upstream_jobs`
- `backfill.wait_for_upstream`
- `backfill.check_upstream_completion`

## Testing Strategy

### Unit Tests

1. **Upstream discovery**
   - Empty dependency graph
   - Linear chain (A -> B -> C)
   - Diamond pattern (A -> B, A -> C, B -> D, C -> D)
   - Cycle detection (should error)
   - Depth limiting

2. **Job creation**
   - Creates correct number of child jobs
   - Assigns correct execution order
   - Idempotency with same idempotency key prefix

3. **State transitions**
   - pending -> waiting_upstream
   - waiting_upstream -> pending (all upstream complete)
   - waiting_upstream -> cancelled

### Integration Tests

**File**: `servo-runtime/tests/upstream_propagation_integration.rs`

```rust
#[tokio::test]
#[ignore] // Requires database
async fn test_upstream_propagation_linear_chain() {
    // Setup: A -> B -> C (C depends on B, B depends on A)
    // Action: Backfill C with include_upstream=true
    // Assert: Jobs created for A, B, C; executed in order A, B, C
}

#[tokio::test]
#[ignore]
async fn test_upstream_propagation_depth_limit() {
    // Setup: A -> B -> C -> D (4 levels)
    // Action: Backfill D with max_depth=2
    // Assert: Only C and B jobs created, not A
}

#[tokio::test]
#[ignore]
async fn test_upstream_job_failure_blocks_downstream() {
    // Setup: A -> B (B depends on A)
    // Action: Backfill B with include_upstream, make A fail
    // Assert: B stays in waiting_upstream, never processes partitions
}

#[tokio::test]
#[ignore]
async fn test_cancel_job_tree() {
    // Setup: Root job with 3 upstream jobs
    // Action: Cancel root job
    // Assert: All 4 jobs cancelled
}

#[tokio::test]
#[ignore]
async fn test_pause_resume_with_upstream() {
    // Setup: A -> B with B partially complete
    // Action: Pause B, resume B
    // Assert: A's completion preserved, B resumes correctly
}
```

## Rollout Plan

### Feature Flag

Add configuration flag for gradual rollout:

```rust
pub struct BackfillConfig {
    /// Enable upstream propagation feature
    pub upstream_propagation_enabled: bool,
    /// Default max upstream depth when not specified
    pub default_max_upstream_depth: i32,
    /// Maximum allowed upstream depth (safety limit)
    pub max_allowed_upstream_depth: i32,
}
```

### Rollout Phases

1. **Alpha**: Enabled only via explicit environment variable
2. **Beta**: Available with `--include-upstream` flag, default off
3. **GA**: Flag available, documented, default off (opt-in behavior)

## Documentation Updates

1. **CLI README**: Add `--include-upstream` flag documentation
2. **Runbook**: Add upstream propagation troubleshooting section
3. **Architecture docs**: Update backfill flow diagram

## Estimated Effort

| Task | Complexity | Dependencies |
|------|------------|--------------|
| Migration (Phase 1) | Low | None |
| Models (Phase 2) | Low | Phase 1 |
| Storage functions (Phase 3) | Medium | Phase 2 |
| CLI updates (Phase 4) | Low | Phase 3 |
| Executor updates (Phase 5) | High | Phase 3, 4 |
| State machine (Phase 6) | Low | Phase 5 |
| Metrics (Phase 7) | Low | Phase 5 |
| Unit tests | Medium | Phases 1-6 |
| Integration tests | Medium | All phases |
| Documentation | Low | All phases |

## Risk Assessment

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Cyclic dependencies | High | Low | Cycle detection before job creation |
| Large dependency trees | Medium | Medium | max_upstream_depth limit, depth histogram alerts |
| Partition key mismatch | Medium | Medium | Fallback to full backfill, warn in logs |
| Cascading failures | High | Low | Independent job failure handling |
| Resource exhaustion | Medium | Low | Job concurrency limits per tenant |

## Success Criteria

1. **Functional**: `servo backfill create X --include-upstream` creates and executes upstream jobs first
2. **Correctness**: No partition processed before all upstream data available
3. **Observability**: Clear visibility into job tree status via CLI and metrics
4. **Safety**: Cycle detection prevents infinite loops; depth limits prevent explosion
5. **Consistency**: Pause/resume/cancel work correctly across job trees

## Out of Scope

- Automatic partition key mapping between different partition schemes
- Parallel execution of independent upstream branches (future increment)
- Downstream propagation (inverse feature)
- Cross-tenant dependency tracking

## Appendix: State Diagram

```
                    ┌─────────────────────┐
                    │       pending       │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              │ include_upstream=false          │ include_upstream=true
              │                                 │
              ▼                                 ▼
     ┌────────────────┐              ┌──────────────────┐
     │    running     │              │ waiting_upstream │
     └───────┬────────┘              └────────┬─────────┘
             │                                │
             │                                │ all upstream complete
             │                                ▼
             │                       ┌────────────────┐
             │                       │    pending     │ (re-enters normal flow)
             │                       └────────────────┘
             │
    ┌────────┴────────┬─────────────┬────────────────┐
    │                 │             │                │
    ▼                 ▼             ▼                ▼
┌─────────┐    ┌──────────┐   ┌──────────┐    ┌───────────┐
│completed│    │  failed  │   │ paused   │    │ cancelled │
└─────────┘    └──────────┘   └──────────┘    └───────────┘
```
