-- Initial schema for Servo metadata
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-- Assets table
CREATE TABLE IF NOT EXISTS assets (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    asset_type VARCHAR(100) NOT NULL,
    owner VARCHAR(255),
    tags JSONB NOT NULL DEFAULT '[]',
    tenant_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(name, tenant_id)
);

CREATE INDEX idx_assets_tenant_id ON assets(tenant_id);
CREATE INDEX idx_assets_name ON assets(name);
CREATE INDEX idx_assets_created_at ON assets(created_at);

-- Workflows table
CREATE TABLE IF NOT EXISTS workflows (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    owner VARCHAR(255),
    tags JSONB NOT NULL DEFAULT '[]',
    tenant_id VARCHAR(255),
    version INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(name, tenant_id, version)
);

CREATE INDEX idx_workflows_tenant_id ON workflows(tenant_id);
CREATE INDEX idx_workflows_name ON workflows(name);
CREATE INDEX idx_workflows_created_at ON workflows(created_at);

-- Executions table
CREATE TABLE IF NOT EXISTS executions (
    id UUID PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    state VARCHAR(50) NOT NULL,
    tenant_id VARCHAR(255),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_executions_workflow_id ON executions(workflow_id);
CREATE INDEX idx_executions_tenant_id ON executions(tenant_id);
CREATE INDEX idx_executions_state ON executions(state);
CREATE INDEX idx_executions_created_at ON executions(created_at);

-- Asset dependencies table (for lineage)
CREATE TABLE IF NOT EXISTS asset_dependencies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    upstream_asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    downstream_asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    dependency_type VARCHAR(50) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(upstream_asset_id, downstream_asset_id, dependency_type)
);

CREATE INDEX idx_asset_deps_upstream ON asset_dependencies(upstream_asset_id);
CREATE INDEX idx_asset_deps_downstream ON asset_dependencies(downstream_asset_id);

-- Enable Row Level Security (for multi-tenancy)
ALTER TABLE assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE workflows ENABLE ROW LEVEL SECURITY;
ALTER TABLE executions ENABLE ROW LEVEL SECURITY;

-- Enforce tenant isolation. Access is granted only when the session sets
-- app.current_tenant, or when the row is global (tenant_id IS NULL).
-- NOTE: The application must set `SET LOCAL app.current_tenant = '<tenant>';`
-- before issuing queries.
CREATE POLICY assets_tenant_isolation ON assets
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

CREATE POLICY workflows_tenant_isolation ON workflows
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

CREATE POLICY executions_tenant_isolation ON executions
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

ALTER TABLE assets FORCE ROW LEVEL SECURITY;
ALTER TABLE workflows FORCE ROW LEVEL SECURITY;
ALTER TABLE executions FORCE ROW LEVEL SECURITY;
