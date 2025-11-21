-- Additional constraints to harden data integrity

-- Prevent self-referential dependencies
ALTER TABLE asset_dependencies
    ADD CONSTRAINT asset_dependencies_no_self_ref
    CHECK (upstream_asset_id <> downstream_asset_id);

-- Restrict dependency types
ALTER TABLE asset_dependencies
    ADD CONSTRAINT asset_dependencies_type_check
    CHECK (dependency_type IN ('data', 'metadata', 'control'));

-- Restrict execution states
ALTER TABLE executions
    ADD CONSTRAINT executions_state_check
    CHECK (state IN ('pending', 'running', 'succeeded', 'failed', 'cancelled', 'timeout'));

-- Enforce monotonic timestamps for assets
ALTER TABLE assets
    ADD CONSTRAINT assets_updated_after_created
    CHECK (updated_at >= created_at);

-- Enforce monotonic timestamps for workflows
ALTER TABLE workflows
    ADD CONSTRAINT workflows_updated_after_created
    CHECK (updated_at >= created_at);

-- Enforce monotonic timestamps for executions
ALTER TABLE executions
    ADD CONSTRAINT executions_updated_after_created
    CHECK (updated_at >= created_at);

-- Completed_at must be >= started_at when present
ALTER TABLE executions
    ADD CONSTRAINT executions_completed_after_started
    CHECK (completed_at IS NULL OR started_at IS NULL OR completed_at >= started_at);

-- Workflow version must be positive
ALTER TABLE workflows
    ADD CONSTRAINT workflows_version_positive
    CHECK (version > 0);
