-- Data quality checks migration
-- Adds tables for defining asset checks and storing check execution results

-- Asset check definitions table
CREATE TABLE IF NOT EXISTS asset_checks (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    check_type JSONB NOT NULL,
    severity VARCHAR(20) NOT NULL DEFAULT 'error',
    blocking BOOLEAN NOT NULL DEFAULT TRUE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    tags JSONB NOT NULL DEFAULT '[]',
    owner VARCHAR(255),
    tenant_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(asset_id, name),
    CONSTRAINT valid_check_severity CHECK (severity IN ('info', 'warning', 'error'))
);

CREATE INDEX idx_asset_checks_asset_id ON asset_checks(asset_id);
CREATE INDEX idx_asset_checks_tenant_id ON asset_checks(tenant_id);
CREATE INDEX idx_asset_checks_severity ON asset_checks(severity);
CREATE INDEX idx_asset_checks_enabled ON asset_checks(enabled) WHERE enabled = TRUE;

-- Check execution results table
CREATE TABLE IF NOT EXISTS check_results (
    id UUID PRIMARY KEY,
    check_id UUID NOT NULL REFERENCES asset_checks(id) ON DELETE CASCADE,
    execution_id UUID NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    outcome VARCHAR(20) NOT NULL,
    severity VARCHAR(20) NOT NULL,
    blocking BOOLEAN NOT NULL,
    failed_row_count BIGINT,
    total_row_count BIGINT,
    error_message TEXT,
    failed_samples JSONB,
    duration_ms BIGINT NOT NULL,
    executed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB,
    tenant_id VARCHAR(255),

    CONSTRAINT valid_check_outcome CHECK (outcome IN ('passed', 'failed', 'skipped', 'error'))
);

CREATE INDEX idx_check_results_check_id ON check_results(check_id);
CREATE INDEX idx_check_results_execution_id ON check_results(execution_id);
CREATE INDEX idx_check_results_asset_id ON check_results(asset_id);
CREATE INDEX idx_check_results_outcome ON check_results(outcome);
CREATE INDEX idx_check_results_executed_at ON check_results(executed_at DESC);
CREATE INDEX idx_check_results_tenant_id ON check_results(tenant_id);

-- Enable Row Level Security for multi-tenancy
ALTER TABLE asset_checks ENABLE ROW LEVEL SECURITY;
ALTER TABLE check_results ENABLE ROW LEVEL SECURITY;

-- RLS policy for asset_checks: tenant isolation
CREATE POLICY asset_checks_tenant_isolation ON asset_checks
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- RLS policy for check_results: tenant isolation
CREATE POLICY check_results_tenant_isolation ON check_results
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- Force RLS even for table owners
ALTER TABLE asset_checks FORCE ROW LEVEL SECURITY;
ALTER TABLE check_results FORCE ROW LEVEL SECURITY;

-- Grant privileges to application role
GRANT SELECT, INSERT, UPDATE, DELETE ON asset_checks TO servo_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON check_results TO servo_app;
