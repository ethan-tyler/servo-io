-- Add indexes to prevent overlapping active backfills
-- This speeds up overlap detection when multiple requests try to backfill
-- the same asset/partition simultaneously

-- Overlap prevention strategy:
-- 1. Application-level check (find_active_backfill_for_partition)
-- 2. Transactional INSERT with conflict detection
-- 3. RLS ensures tenant isolation

-- Add index to speed up the overlap lookup query
CREATE INDEX IF NOT EXISTS idx_backfill_jobs_active_by_asset
ON backfill_jobs (asset_id, state)
WHERE state IN ('pending', 'running');
