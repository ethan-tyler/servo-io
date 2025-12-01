-- Add SLA tracking and job priority support for backfill jobs
-- This migration adds columns for:
-- 1. SLA deadline tracking for backfill completion targets
-- 2. Job priority for scheduling prioritization
--
-- SLA TRACKING BEHAVIOR:
-- - sla_deadline_at is optional; when set, alerts/dashboards can track compliance
-- - Jobs with NOW() > sla_deadline_at are considered "breached"
-- - Jobs with ETA > sla_deadline_at are considered "at-risk"
--
-- PRIORITY SCHEDULING:
-- - Priority ranges from -10 (lowest) to 10 (highest), default 0
-- - Higher priority jobs should be claimed/processed first
-- - Index supports priority-based job claiming

-- Add SLA deadline column
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS sla_deadline_at TIMESTAMPTZ;

-- Add priority column for scheduling
ALTER TABLE backfill_jobs ADD COLUMN IF NOT EXISTS priority INTEGER NOT NULL DEFAULT 0;

-- Constraint: priority must be in reasonable range (-10 to 10)
ALTER TABLE backfill_jobs ADD CONSTRAINT valid_priority
    CHECK (priority >= -10 AND priority <= 10);

-- Index for SLA deadline queries (finding breached/at-risk jobs)
-- Only indexes active jobs with deadlines
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_sla_deadline
    ON backfill_jobs(sla_deadline_at)
    WHERE sla_deadline_at IS NOT NULL
      AND state IN ('pending', 'running', 'waiting_upstream', 'resuming', 'paused');

-- Index for priority-based job scheduling
-- Supports claiming highest priority pending jobs first
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_priority
    ON backfill_jobs(priority DESC, created_at)
    WHERE state = 'pending';

-- Composite index for tenant SLA monitoring
-- Supports queries like "all active jobs for tenant X with SLA deadlines"
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_tenant_sla
    ON backfill_jobs(tenant_id, sla_deadline_at)
    WHERE sla_deadline_at IS NOT NULL
      AND state IN ('pending', 'running', 'waiting_upstream', 'resuming', 'paused');

-- Comments for documentation
COMMENT ON COLUMN backfill_jobs.sla_deadline_at IS
    'Optional deadline for SLA tracking; job should complete before this time';
COMMENT ON COLUMN backfill_jobs.priority IS
    'Job priority for scheduling (-10 to 10, higher = more urgent, default 0)';
