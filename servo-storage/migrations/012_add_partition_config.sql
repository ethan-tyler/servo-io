-- Migration: Add partition configuration to assets table
-- This enables partition-aware backfills with non-identity mappings

-- Add partition configuration column to assets table
ALTER TABLE assets ADD COLUMN IF NOT EXISTS partition_config JSONB;

-- Add partial GIN index for efficient JSON queries on assets with partition config
CREATE INDEX IF NOT EXISTS idx_assets_partition_config
    ON assets USING GIN (partition_config)
    WHERE partition_config IS NOT NULL;

-- Document the column
COMMENT ON COLUMN assets.partition_config IS
    'Optional partition configuration (type, granularity, key) for partition-aware backfills. JSON schema: {partition_type: {Time: {granularity: string} | Custom: {expression: string} | Range: {lower_bound: string, upper_bound: string}}, partition_key: string}';
