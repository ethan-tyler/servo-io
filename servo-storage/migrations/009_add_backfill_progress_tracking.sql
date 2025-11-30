-- Add progress tracking and pause/resume support for backfill jobs
-- This migration adds columns for:
-- 1. Pause/resume state tracking
-- 2. ETA calculation using EWMA
-- 3. Progress checkpoint for resumption
--
-- PAUSE BEHAVIOR:
-- When a job is paused, it will complete the currently executing partition before
-- stopping. This "pause at partition boundary" behavior ensures:
-- - No partial partition execution
-- - Clean checkpoint state for resumption
-- - Atomic progress updates
--
-- STATE TRANSITIONS:
-- pending  -> running   (job claimed by executor)
-- running  -> paused    (user pauses, current partition completes first)
-- running  -> completed (all partitions done)
-- running  -> failed    (unrecoverable failure)
-- running  -> cancelled (user cancels)
-- paused   -> resuming  (user resumes)
-- paused   -> cancelled (user cancels while paused)
-- resuming -> running   (executor claims resumed job)

-- Add new columns to backfill_jobs
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS paused_at TIMESTAMPTZ;
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS checkpoint_partition_key VARCHAR(255);
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS estimated_completion_at TIMESTAMPTZ;
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS avg_partition_duration_ms BIGINT;

-- Drop the existing state constraint and recreate with new states
ALTER TABLE backfill_jobs DROP CONSTRAINT IF EXISTS valid_backfill_state;
ALTER TABLE backfill_jobs ADD CONSTRAINT valid_backfill_state
    CHECK (state IN ('pending', 'running', 'paused', 'resuming', 'completed', 'failed', 'cancelled'));

-- Add constraint: paused_at must be set when state is 'paused'
-- Note: We don't enforce this strictly to avoid migration issues with existing rows
-- The application layer handles this consistently

-- Add index for efficient progress queries (jobs actively being processed)
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_state_updated
    ON backfill_jobs(state, heartbeat_at)
    WHERE state IN ('running', 'paused', 'resuming');

-- Add index for paused jobs (for monitoring/alerting)
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_paused
    ON backfill_jobs(paused_at)
    WHERE state = 'paused';

-- Add index for ETA queries (active jobs with ETA)
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_eta
    ON backfill_jobs(estimated_completion_at)
    WHERE state = 'running' AND estimated_completion_at IS NOT NULL;
