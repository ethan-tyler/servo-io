-- Add upstream propagation support for backfill jobs
-- This migration adds columns for:
-- 1. Parent-child job relationships for dependency tracking
-- 2. Upstream depth configuration and tracking
-- 3. Execution ordering within job trees
--
-- UPSTREAM PROPAGATION BEHAVIOR:
-- When include_upstream=true, the system will:
-- 1. Discover all upstream assets within max_upstream_depth
-- 2. Create child backfill jobs for each upstream asset
-- 3. Process upstream jobs before the target asset
-- 4. Track completion via upstream_job_count/completed_upstream_jobs
--
-- STATE TRANSITIONS (new):
-- pending         -> waiting_upstream  (when creating upstream jobs)
-- waiting_upstream -> pending          (when all upstream jobs complete)
-- waiting_upstream -> cancelled        (user cancellation)
--
-- JOB HIERARCHY:
-- - Root job: parent_job_id IS NULL, has child jobs for upstream assets
-- - Child job: parent_job_id set, created for upstream dependency
-- - execution_order: topological order (0 = furthest upstream, higher = closer to root)

-- Add parent-child relationship column
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS parent_job_id UUID
    REFERENCES backfill_jobs(id) ON DELETE CASCADE;

-- Add upstream depth configuration
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS max_upstream_depth INTEGER NOT NULL DEFAULT 0;

-- Add upstream job tracking counters
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS upstream_job_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS completed_upstream_jobs INTEGER NOT NULL DEFAULT 0;

-- Add execution order for topological processing
-- 0 = furthest upstream (execute first), higher numbers execute later
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS execution_order INTEGER NOT NULL DEFAULT 0;

-- Drop the existing state constraint and recreate with waiting_upstream state
ALTER TABLE backfill_jobs DROP CONSTRAINT IF EXISTS valid_backfill_state;
ALTER TABLE backfill_jobs ADD CONSTRAINT valid_backfill_state
    CHECK (state IN ('pending', 'waiting_upstream', 'running', 'paused', 'resuming', 'completed', 'failed', 'cancelled'));

-- Index for efficient child job lookup
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_parent
    ON backfill_jobs(parent_job_id)
    WHERE parent_job_id IS NOT NULL;

-- Index for finding ready-to-execute jobs in a tree (topological order processing)
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_tree_execution
    ON backfill_jobs(parent_job_id, execution_order, state)
    WHERE state IN ('pending', 'resuming');

-- Index for jobs waiting on upstream completion
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_waiting_upstream
    ON backfill_jobs(state, parent_job_id)
    WHERE state = 'waiting_upstream';

-- Add constraint: upstream counters must be non-negative
ALTER TABLE backfill_jobs ADD CONSTRAINT valid_upstream_counts
    CHECK (upstream_job_count >= 0 AND completed_upstream_jobs >= 0 AND completed_upstream_jobs <= upstream_job_count);

-- Add constraint: max_upstream_depth must be reasonable (0-10)
ALTER TABLE backfill_jobs ADD CONSTRAINT valid_upstream_depth
    CHECK (max_upstream_depth >= 0 AND max_upstream_depth <= 10);

-- Comments for documentation
COMMENT ON COLUMN backfill_jobs.parent_job_id IS 'Parent job ID for upstream propagation hierarchy (NULL for root jobs)';
COMMENT ON COLUMN backfill_jobs.max_upstream_depth IS 'Maximum depth for upstream discovery (0 = direct dependencies only)';
COMMENT ON COLUMN backfill_jobs.upstream_job_count IS 'Total number of upstream child jobs created';
COMMENT ON COLUMN backfill_jobs.completed_upstream_jobs IS 'Number of upstream jobs that have completed successfully';
COMMENT ON COLUMN backfill_jobs.execution_order IS 'Topological order for execution (0 = furthest upstream, execute first)';
