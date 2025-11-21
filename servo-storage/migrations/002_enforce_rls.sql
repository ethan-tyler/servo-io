-- Enforce RLS policies with WITH CHECK clauses
-- This migration adds WITH CHECK clauses to existing RLS policies to enforce
-- tenant isolation on INSERT and UPDATE operations, not just SELECT.

-- Assets table: Add WITH CHECK clause
DROP POLICY IF EXISTS assets_tenant_isolation ON assets;
CREATE POLICY assets_tenant_isolation ON assets
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- Workflows table: Add WITH CHECK clause
DROP POLICY IF EXISTS workflows_tenant_isolation ON workflows;
CREATE POLICY workflows_tenant_isolation ON workflows
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- Executions table: Add WITH CHECK clause
DROP POLICY IF EXISTS executions_tenant_isolation ON executions;
CREATE POLICY executions_tenant_isolation ON executions
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    )
    WITH CHECK (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant', true)
    );

-- Asset dependencies: Add missing RLS policy
-- Dependencies inherit tenant context from their referenced assets
ALTER TABLE asset_dependencies ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS asset_deps_access ON asset_dependencies;
CREATE POLICY asset_deps_access ON asset_dependencies
    USING (
        EXISTS (
            SELECT 1
            FROM assets a_up
            WHERE a_up.id = asset_dependencies.upstream_asset_id
              AND (a_up.tenant_id IS NULL OR a_up.tenant_id = current_setting('app.current_tenant', true))
        )
        AND EXISTS (
            SELECT 1
            FROM assets a_down
            WHERE a_down.id = asset_dependencies.downstream_asset_id
              AND (a_down.tenant_id IS NULL OR a_down.tenant_id = current_setting('app.current_tenant', true))
        )
    )
    WITH CHECK (
        EXISTS (
            SELECT 1
            FROM assets a_up
            WHERE a_up.id = asset_dependencies.upstream_asset_id
              AND (a_up.tenant_id IS NULL OR a_up.tenant_id = current_setting('app.current_tenant', true))
        )
        AND EXISTS (
            SELECT 1
            FROM assets a_down
            WHERE a_down.id = asset_dependencies.downstream_asset_id
              AND (a_down.tenant_id IS NULL OR a_down.tenant_id = current_setting('app.current_tenant', true))
        )
    );

ALTER TABLE asset_dependencies FORCE ROW LEVEL SECURITY;
