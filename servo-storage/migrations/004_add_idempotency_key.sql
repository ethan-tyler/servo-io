-- Add idempotency key support for execution deduplication
--
-- Idempotency keys allow clients to safely retry execution creation
-- without creating duplicate executions. When provided, the same key
-- within a tenant and workflow will return the existing execution
-- instead of creating a new one.

-- Add idempotency_key column to executions table
ALTER TABLE executions
    ADD COLUMN idempotency_key TEXT;

-- Add validation constraint: idempotency keys must be 1-512 characters
ALTER TABLE executions
    ADD CONSTRAINT executions_idempotency_key_length
    CHECK (idempotency_key IS NULL OR (length(idempotency_key) >= 1 AND length(idempotency_key) <= 512));

-- Create partial unique index to enforce uniqueness per (tenant, workflow, key)
-- when idempotency_key is not null. This allows:
-- 1. Multiple executions with null idempotency_key (normal case)
-- 2. Only one execution per (tenant_id, workflow_id, idempotency_key) when key is provided
-- 3. Same key can be reused across different workflows within a tenant
CREATE UNIQUE INDEX executions_tenant_workflow_idempotency_key
    ON executions(tenant_id, workflow_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Add index for efficient lookup by (workflow_id, idempotency_key)
CREATE INDEX idx_executions_workflow_idempotency_key
    ON executions(workflow_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
