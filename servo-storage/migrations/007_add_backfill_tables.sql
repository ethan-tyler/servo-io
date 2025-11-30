-- Backfill jobs and partition tracking migration
-- Adds tables for managing intelligent backfill operations

-- Main backfill jobs table
CREATE TABLE IF NOT EXISTS backfill_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id VARCHAR(255) NOT NULL,
    asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    asset_name VARCHAR(255) NOT NULL,
    idempotency_key VARCHAR(255) NOT NULL,
    state VARCHAR(50) NOT NULL DEFAULT 'pending',
    execution_strategy JSONB NOT NULL DEFAULT '{"type": "sequential"}',
    partition_start VARCHAR(255),
    partition_end VARCHAR(255),
    partition_keys JSONB NOT NULL DEFAULT '[]',
    total_partitions INTEGER NOT NULL DEFAULT 0,
    completed_partitions INTEGER NOT NULL DEFAULT 0,
    failed_partitions INTEGER NOT NULL DEFAULT 0,
    skipped_partitions INTEGER NOT NULL DEFAULT 0,
    include_upstream BOOLEAN NOT NULL DEFAULT FALSE,
    error_message TEXT,
    created_by VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    version INTEGER NOT NULL DEFAULT 1,

    UNIQUE(tenant_id, idempotency_key),
    CONSTRAINT valid_backfill_state CHECK (state IN ('pending', 'running', 'completed', 'failed', 'cancelled'))
);

-- Indexes for backfill_jobs
CREATE INDEX idx_backfill_jobs_tenant_state ON backfill_jobs(tenant_id, state);
CREATE INDEX idx_backfill_jobs_asset_id ON backfill_jobs(asset_id);
CREATE INDEX idx_backfill_jobs_created_at ON backfill_jobs(created_at DESC);
CREATE INDEX idx_backfill_jobs_heartbeat ON backfill_jobs(heartbeat_at) WHERE state = 'running';

-- Partition-level status tracking table
CREATE TABLE IF NOT EXISTS backfill_partitions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    backfill_job_id UUID NOT NULL REFERENCES backfill_jobs(id) ON DELETE CASCADE,
    partition_key VARCHAR(255) NOT NULL,
    state VARCHAR(50) NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    execution_id UUID REFERENCES executions(id) ON DELETE SET NULL,
    error_message TEXT,
    duration_ms BIGINT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    tenant_id VARCHAR(255) NOT NULL,

    UNIQUE(backfill_job_id, partition_key),
    CONSTRAINT valid_partition_state CHECK (state IN ('pending', 'running', 'completed', 'failed', 'skipped'))
);

-- Indexes for backfill_partitions
CREATE INDEX idx_backfill_partitions_job_state ON backfill_partitions(backfill_job_id, state);
CREATE INDEX idx_backfill_partitions_tenant_id ON backfill_partitions(tenant_id);
CREATE INDEX idx_backfill_partitions_execution_id ON backfill_partitions(execution_id);

-- Enable Row Level Security for multi-tenancy
ALTER TABLE backfill_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE backfill_partitions ENABLE ROW LEVEL SECURITY;

-- RLS policy for backfill_jobs: tenant isolation
CREATE POLICY backfill_jobs_tenant_isolation ON backfill_jobs
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- RLS policy for backfill_partitions: tenant isolation
CREATE POLICY backfill_partitions_tenant_isolation ON backfill_partitions
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- Force RLS even for table owners
ALTER TABLE backfill_jobs FORCE ROW LEVEL SECURITY;
ALTER TABLE backfill_partitions FORCE ROW LEVEL SECURITY;

-- Grant privileges to application role
GRANT SELECT, INSERT, UPDATE, DELETE ON backfill_jobs TO servo_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON backfill_partitions TO servo_app;
