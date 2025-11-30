# Backfill Operations Runbook

## Overview

This runbook covers operational procedures for managing backfill jobs, including pause/resume operations, progress monitoring, ETA tracking, and recovery from common failure scenarios.

### Related Alerts

| Alert | Threshold | Description |
|-------|-----------|-------------|
| `ServoBackfillJobStuck` | Heartbeat >5 min stale | Job not making progress |
| `ServoBackfillHighFailureRate` | >20% partition failures | High failure rate |
| `ServoBackfillLongRunning` | >4 hours duration | Unusually long backfill |
| `ServoBackfillQueueDepth` | >10 jobs pending | Jobs queued waiting |

## Impact Assessment

### Affected Components
- Data assets not being backfilled as expected
- Downstream dependencies waiting for historical data
- Resource consumption by stuck jobs
- Potential data inconsistency if resumed incorrectly

### Symptoms
- Jobs showing "running" state but no partition progress
- ETA estimates not decreasing (or increasing)
- Paused jobs not resuming when expected
- Jobs stuck in "resuming" state

## Job States Reference

| State | Description | Valid Transitions |
|-------|-------------|-------------------|
| `pending` | Waiting to be claimed | `running` |
| `running` | Being processed | `paused`, `completed`, `failed`, `cancelled` |
| `paused` | Manually paused | `resuming`, `cancelled` |
| `resuming` | Requested resume, waiting claim | `running` |
| `completed` | All partitions done | (terminal) |
| `failed` | Job failed | (terminal) |
| `cancelled` | User cancelled | (terminal) |

## Common Operations

### 1. Pause a Running Backfill Job

Use when you need to temporarily stop a backfill (e.g., for maintenance or to reduce load).

```bash
# Via CLI
servo backfill pause <job_id>

# Verify pause was successful
servo backfill status <job_id>
```

**What happens:**
- Job state transitions from `running` to `paused`
- Current partition completes before stopping (partition-boundary pause)
- Checkpoint is saved at last completed partition
- ETA statistics (avg_partition_duration_ms) are preserved
- `paused_at` timestamp is recorded

**Direct database approach (emergency only):**
```sql
-- WARNING: Only use if CLI is unavailable
UPDATE backfill_jobs
SET state = 'paused',
    paused_at = NOW(),
    version = version + 1
WHERE id = '<job_id>'
  AND state = 'running'
  AND tenant_id = '<tenant_id>';
```

### 2. Resume a Paused Backfill Job

```bash
# Via CLI
servo backfill resume <job_id>

# Verify job is running again
servo backfill status <job_id>
```

**What happens:**
1. Job state transitions from `paused` to `resuming`
2. Next available executor claims the job
3. Job transitions to `running`
4. Processing continues from `checkpoint_partition_key`
5. ETA calculation resumes using preserved `avg_partition_duration_ms`

**Monitor resume progress:**
```bash
# Check job was claimed
servo backfill status <job_id> --watch

# Or via database
psql $DATABASE_URL -c "
SELECT state, checkpoint_partition_key, completed_partitions, total_partitions,
       estimated_completion_at
FROM backfill_jobs
WHERE id = '<job_id>'"
```

### 3. Cancel a Backfill Job

```bash
# Via CLI
servo backfill cancel <job_id> --reason "maintenance window"

# Verify cancellation
servo backfill status <job_id>
```

**Note:** Cancellation is allowed from `pending`, `running`, `paused`, or `resuming` states.

### 4. Monitor Backfill Progress

```bash
# List active backfills
servo backfill list --state running

# Check specific job with ETA
servo backfill status <job_id>

# Watch progress in real-time
servo backfill status <job_id> --watch
```

**Key metrics to monitor:**
- `completed_partitions / total_partitions` - Progress ratio
- `avg_partition_duration_ms` - Average time per partition
- `estimated_completion_at` - Predicted completion time
- `failed_partitions` - Number of permanently failed partitions

## Triage Steps

### 1. Identify Stuck Jobs

```bash
# List jobs with stale heartbeats (>2 min old)
psql $DATABASE_URL -c "
SELECT id, asset_name, state,
       completed_partitions || '/' || total_partitions as progress,
       EXTRACT(EPOCH FROM (NOW() - heartbeat_at)) as heartbeat_age_secs,
       checkpoint_partition_key
FROM backfill_jobs
WHERE state = 'running'
  AND heartbeat_at < NOW() - INTERVAL '2 minutes'
ORDER BY heartbeat_at"
```

### 2. Check Job ETA Progress

```bash
# Jobs with increasing or stale ETA
psql $DATABASE_URL -c "
SELECT id, asset_name, completed_partitions, total_partitions,
       avg_partition_duration_ms,
       estimated_completion_at,
       (estimated_completion_at - NOW()) as eta_remaining
FROM backfill_jobs
WHERE state = 'running'
ORDER BY estimated_completion_at"
```

### 3. Check for Failed Partitions

```bash
# Get failed partition details
psql $DATABASE_URL -c "
SELECT bp.partition_key, bp.attempt_count, bp.error_message, bp.completed_at
FROM backfill_partitions bp
JOIN backfill_jobs bj ON bp.backfill_job_id = bj.id
WHERE bj.id = '<job_id>'
  AND bp.state = 'failed'
ORDER BY bp.partition_key"
```

### 4. Check Executor Logs

```bash
# Find executor processing the job
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.job_id=\"<job_id>\"" \
  --limit=50 --format="json" | jq -r '.[] | "\(.timestamp) \(.jsonPayload.level) \(.jsonPayload.message)"'
```

### 5. Verify Checkpoint Integrity

```bash
# Ensure checkpoint matches partition states
psql $DATABASE_URL -c "
SELECT
  bj.checkpoint_partition_key as job_checkpoint,
  MAX(bp.partition_key) FILTER (WHERE bp.state = 'completed') as last_completed_partition,
  COUNT(*) FILTER (WHERE bp.state = 'pending') as pending_partitions,
  COUNT(*) FILTER (WHERE bp.state = 'completed') as completed_partitions
FROM backfill_jobs bj
JOIN backfill_partitions bp ON bp.backfill_job_id = bj.id
WHERE bj.id = '<job_id>'
GROUP BY bj.checkpoint_partition_key"
```

## Resolution Procedures

### Scenario: Job Stuck with Stale Heartbeat

Executor crashed or was terminated while processing.

1. **Check if executor is still running:**
```bash
gcloud logging read "jsonPayload.job_id=\"<job_id>\"" --limit=5 | jq '.[0].timestamp'
```

2. **If no recent logs, job will be auto-reclaimed:**
   - Stale heartbeat threshold is 120 seconds
   - Another executor will claim the job automatically
   - Partitions will retry from checkpoint

3. **Force reclaim if needed (emergency):**
```sql
-- Reset heartbeat to trigger reclaim
UPDATE backfill_jobs
SET heartbeat_at = NOW() - INTERVAL '5 minutes',
    version = version + 1
WHERE id = '<job_id>' AND state = 'running';
```

### Scenario: Job Stuck in "Resuming" State

Job was resumed but no executor picked it up.

1. **Check for available executors:**
```bash
gcloud run services describe servo-backfill-executor --region=us-central1 --format="yaml(status.conditions)"
```

2. **Verify job is claimable:**
```sql
SELECT id, state, heartbeat_at
FROM backfill_jobs
WHERE state IN ('pending', 'resuming')
  AND (heartbeat_at IS NULL OR heartbeat_at < NOW() - INTERVAL '120 seconds');
```

3. **If executor is running but not claiming:**
   - Check executor logs for errors
   - Verify tenant context is correct
   - Restart executor if necessary

### Scenario: High Partition Failure Rate

Many partitions failing repeatedly.

1. **Identify failure pattern:**
```sql
SELECT partition_key, attempt_count, error_message
FROM backfill_partitions
WHERE backfill_job_id = '<job_id>' AND state = 'failed'
ORDER BY partition_key;
```

2. **Common causes:**
   - Upstream data source unavailable
   - Schema changes mid-backfill
   - Resource exhaustion (memory, CPU)
   - Rate limiting from external systems

3. **Options:**
   - Pause job, fix underlying issue, resume
   - Cancel job and create new one with corrected parameters
   - Increase partition retry limit (if transient failures)

### Scenario: Job Paused But Won't Resume

1. **Check current state:**
```sql
SELECT id, state, paused_at, checkpoint_partition_key, error_message
FROM backfill_jobs WHERE id = '<job_id>';
```

2. **If state is not 'paused', resume won't work:**
   - Must be in 'paused' state to resume
   - Check if job completed or was cancelled

3. **Verify no version conflict:**
```sql
-- Check version number is consistent
SELECT version FROM backfill_jobs WHERE id = '<job_id>';
```

### Scenario: ETA Not Updating

ETA calculation requires completed partitions.

1. **Check observation count:**
```sql
SELECT completed_partitions, avg_partition_duration_ms
FROM backfill_jobs WHERE id = '<job_id>';
```

2. **ETA requires at least 1 completed partition**
   - If `avg_partition_duration_ms` is NULL, no partitions have completed yet
   - Wait for first partition to complete

3. **Verify executor is updating progress:**
```bash
gcloud logging read "jsonPayload.job_id=\"<job_id>\" AND jsonPayload.message:~\"progress\"" --limit=10
```

### Bulk Recovery

Reset multiple stuck jobs:

```sql
-- Reset all stuck running jobs (not heartbeating)
UPDATE backfill_jobs
SET heartbeat_at = NOW() - INTERVAL '5 minutes'
WHERE state = 'running'
  AND heartbeat_at < NOW() - INTERVAL '10 minutes'
RETURNING id, asset_name;

-- Cancel all stuck resuming jobs
UPDATE backfill_jobs
SET state = 'cancelled',
    error_message = 'Bulk cancelled: stuck in resuming state',
    completed_at = NOW(),
    version = version + 1
WHERE state = 'resuming'
  AND heartbeat_at < NOW() - INTERVAL '10 minutes'
RETURNING id, asset_name;
```

## Metrics Reference

### Low-Cardinality Metrics (Recommended)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `servo_backfill_jobs_active` | Gauge | `state` | Active jobs by state |
| `servo_backfill_eta_distribution_seconds` | Histogram | - | ETA distribution |
| `servo_backfill_pause_duration_seconds` | Histogram | - | Time spent paused |
| `servo_backfill_partition_duration_seconds` | Histogram | `status` | Partition execution time |
| `servo_backfill_partition_completion_total` | Counter | `status` | Completed/failed partitions |

### PromQL Examples

```promql
# Active running jobs
servo_backfill_jobs_active{state="running"}

# Jobs stuck (running but not claiming partitions)
servo_backfill_jobs_active{state="running"}
  unless on() rate(servo_backfill_partition_completion_total[5m]) > 0

# Average partition duration (P95)
histogram_quantile(0.95, rate(servo_backfill_partition_duration_seconds_bucket[10m]))

# Partition failure rate
rate(servo_backfill_partition_completion_total{status="failed"}[5m])
  / rate(servo_backfill_partition_completion_total[5m])

# Jobs with long ETA (>1 hour)
histogram_quantile(0.5, servo_backfill_eta_distribution_seconds_bucket) > 3600
```

## Escalation

### When to Escalate
- Multiple jobs stuck for >30 minutes
- Pause/resume not working as expected
- Data integrity concerns (checkpoint mismatch)
- Unable to identify root cause in 30 minutes

### Escalation Path
1. **Immediate**: Notify on-call
2. **15 minutes**: Backend Lead
3. **30 minutes**: Platform Lead

## Prevention

- Monitor `servo_backfill_jobs_active` dashboard
- Set up alerts for stale heartbeats
- Review partition failure patterns before starting large backfills
- Test pause/resume on small jobs before production use
- Implement job concurrency limits per asset type

## Related Runbooks

- [Stuck Executions](stuck-executions.md)
- [Database Slow](database-slow.md)
- [High Error Rate](high-error-rate.md)
