-- Migration: Add compute function metadata to assets table
-- This enables real asset execution in LocalExecutor and Cloud Run Worker

-- Add Python compute function metadata columns
ALTER TABLE assets
ADD COLUMN IF NOT EXISTS compute_fn_module VARCHAR(255),
ADD COLUMN IF NOT EXISTS compute_fn_function VARCHAR(255);

-- Document the columns
COMMENT ON COLUMN assets.compute_fn_module IS
    'Python module path containing the asset compute function (e.g., "myproject.workflows")';
COMMENT ON COLUMN assets.compute_fn_function IS
    'Python function name to invoke for this asset (e.g., "clean_customers")';

-- Index for efficient lookups by module (useful for batch operations)
CREATE INDEX IF NOT EXISTS idx_assets_compute_fn_module
    ON assets(compute_fn_module)
    WHERE compute_fn_module IS NOT NULL;
