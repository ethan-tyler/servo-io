-- Add workflow_assets join table for many-to-many relationship
-- This enables workflow compilation with execution plans

CREATE TABLE IF NOT EXISTS workflow_assets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    asset_id UUID NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, -- Order in which assets appear in workflow
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(workflow_id, asset_id),
    UNIQUE(workflow_id, position)
);

CREATE INDEX idx_workflow_assets_workflow_id ON workflow_assets(workflow_id);
CREATE INDEX idx_workflow_assets_asset_id ON workflow_assets(asset_id);
CREATE INDEX idx_workflow_assets_position ON workflow_assets(workflow_id, position);

-- Comments for documentation
COMMENT ON TABLE workflow_assets IS 'Many-to-many relationship between workflows and assets';
COMMENT ON COLUMN workflow_assets.position IS 'Defines the order of assets within the workflow (0-indexed)';
