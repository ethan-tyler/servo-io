-- Add scheduling columns to workflows table
-- Enables cron-based scheduled execution via GCP Cloud Scheduler

ALTER TABLE workflows
ADD COLUMN schedule_cron VARCHAR(255),
ADD COLUMN schedule_timezone VARCHAR(50) DEFAULT 'UTC',
ADD COLUMN schedule_enabled BOOLEAN DEFAULT true,
ADD COLUMN scheduler_job_name VARCHAR(255),
ADD COLUMN last_scheduled_run TIMESTAMPTZ,
ADD COLUMN next_scheduled_run TIMESTAMPTZ;

-- Index for efficient lookup of workflows with pending scheduled runs
CREATE INDEX idx_workflows_next_run ON workflows(next_scheduled_run)
WHERE schedule_enabled = true;

-- Index for looking up workflows by their Cloud Scheduler job name
CREATE INDEX idx_workflows_scheduler_job_name ON workflows(scheduler_job_name)
WHERE scheduler_job_name IS NOT NULL;

COMMENT ON COLUMN workflows.schedule_cron IS 'Cron expression for scheduled execution (e.g., "0 2 * * *" for daily at 2 AM)';
COMMENT ON COLUMN workflows.schedule_timezone IS 'Timezone for cron schedule interpretation (e.g., "America/New_York")';
COMMENT ON COLUMN workflows.schedule_enabled IS 'Whether the schedule is active';
COMMENT ON COLUMN workflows.scheduler_job_name IS 'GCP Cloud Scheduler job name for this workflow';
COMMENT ON COLUMN workflows.last_scheduled_run IS 'Timestamp of the last scheduled execution';
COMMENT ON COLUMN workflows.next_scheduled_run IS 'Computed next execution time based on cron';
