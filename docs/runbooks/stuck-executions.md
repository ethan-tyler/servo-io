# Stuck Executions Runbook

## Overview

This runbook addresses alerts when workflow executions remain in "running" state longer than expected, indicating they may be stuck or hung.

### Alert: `ServoStuckExecutions`

| Metric | Expected | Alert Threshold |
|--------|----------|-----------------|
| Execution Duration | <5 minutes (P95) | Running >30 minutes |

## Impact Assessment

### Affected Components
- Workflows not completing as scheduled
- Downstream dependencies waiting for data
- Resource consumption by stuck processes

### Symptoms
- Executions showing "running" state for extended periods
- Increasing execution queue depth
- Stale data in downstream systems

## Detection

### Alert Expression
```promql
count(servo_execution_state{state="running"} and (time() - servo_execution_start_time) > 1800) > 0
```

### Dashboard
[Workflow Execution Dashboard](https://grafana.servo.io/d/servo-workflow-execution) - Execution States panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.execution_id="<execution_id>"
```

## Triage Steps

### 1. Identify Stuck Executions

```bash
# List executions running >30 minutes
psql $DATABASE_URL -c "
SELECT id, workflow_id, tenant_id, started_at,
       EXTRACT(EPOCH FROM (NOW() - started_at))/60 as minutes_running
FROM executions
WHERE state = 'running'
  AND started_at < NOW() - INTERVAL '30 minutes'
ORDER BY started_at
LIMIT 20"
```

### 2. Check Execution Logs

```bash
# Get logs for specific execution
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.execution_id=\"<execution_id>\"" --limit=50 --format="json" | jq -r '.[] | "\(.timestamp) \(.jsonPayload.level) \(.jsonPayload.message)"'
```

### 3. Check for Worker Issues

```bash
# Check if worker instances are healthy
gcloud run services describe servo-worker --region=us-central1 --format="yaml(status.conditions)"

# Check for OOM or crash loops
gcloud logging read "resource.type=cloud_run_revision AND (textPayload:~\"OOM\" OR textPayload:~\"terminated\" OR textPayload:~\"crash\")" --limit=20
```

### 4. Check for Database Locks

```bash
# Check for blocked queries
psql $DATABASE_URL -c "
SELECT pid, state, wait_event_type, wait_event, query
FROM pg_stat_activity
WHERE state = 'active' AND wait_event IS NOT NULL
ORDER BY query_start"
```

### 5. Check Cloud Tasks Status

```bash
# Check if task is still in queue
gcloud tasks list --queue=servo-execution-queue --location=us-central1 --filter="name:~\"<execution_id>\""
```

## Resolution Procedures

### Scenario: Execution Actually Stuck (No Progress)

1. **Verify no progress in logs**:
```bash
# Check last activity for execution
gcloud logging read "jsonPayload.execution_id=\"<execution_id>\"" --limit=5 --format="json" | jq '.[0].timestamp'
```

2. **Mark execution as failed**:
```bash
psql $DATABASE_URL -c "
UPDATE executions
SET state = 'failed',
    error_message = 'Manually terminated: execution stuck',
    completed_at = NOW(),
    updated_at = NOW()
WHERE id = '<execution_id>'"
```

3. **Trigger retry if appropriate**:
```bash
# Re-trigger the workflow
curl -X POST "https://servo-worker.run.app/execute" \
  -H "Content-Type: application/json" \
  -d '{"workflow_id": "<workflow_id>", "tenant_id": "<tenant_id>"}'
```

### Scenario: Worker Instance Died

1. **Check if the request was lost**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND textPayload:~\"terminated\\|killed\"" --limit=10
```

2. **Reset stuck executions and allow retry**:
```bash
psql $DATABASE_URL -c "
UPDATE executions
SET state = 'pending',
    started_at = NULL,
    updated_at = NOW()
WHERE state = 'running'
  AND started_at < NOW() - INTERVAL '30 minutes'"
```

### Scenario: Database Lock Contention

1. **Identify blocking queries**:
```bash
psql $DATABASE_URL -c "
SELECT blocked.pid AS blocked_pid,
       blocked.query AS blocked_query,
       blocking.pid AS blocking_pid,
       blocking.query AS blocking_query
FROM pg_catalog.pg_locks blocked_locks
JOIN pg_catalog.pg_stat_activity blocked ON blocked.pid = blocked_locks.pid
JOIN pg_catalog.pg_locks blocking_locks ON blocking_locks.locktype = blocked_locks.locktype
JOIN pg_catalog.pg_stat_activity blocking ON blocking.pid = blocking_locks.pid
WHERE NOT blocked_locks.granted"
```

2. **Kill blocking query if safe**:
```bash
psql $DATABASE_URL -c "SELECT pg_terminate_backend(<blocking_pid>)"
```

### Scenario: External Dependency Timeout

1. **Check if waiting on external call**:
```bash
gcloud logging read "jsonPayload.execution_id=\"<execution_id>\" AND jsonPayload.message:~\"waiting\\|timeout\\|external\"" --limit=10
```

2. **If circuit breaker should be open, check state**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=servo_circuit_breaker_state" | jq '.data.result'
```

### Bulk Recovery

For multiple stuck executions:

```bash
# Reset all stuck executions
psql $DATABASE_URL -c "
UPDATE executions
SET state = 'failed',
    error_message = 'Bulk reset: execution stuck >30 min',
    completed_at = NOW(),
    updated_at = NOW()
WHERE state = 'running'
  AND started_at < NOW() - INTERVAL '30 minutes'
RETURNING id, workflow_id"
```

## Escalation

### When to Escalate
- >10 executions stuck simultaneously
- Same workflow repeatedly getting stuck
- Worker instances crashing repeatedly
- Unable to identify root cause in 30 minutes

### Escalation Path
1. **Immediate**: Notify on-call
2. **15 minutes**: Backend Lead
3. **30 minutes**: Platform Lead

## Prevention

- Implement execution timeout enforcement (currently 600s)
- Add heartbeat updates during long-running operations
- Implement dead-letter queue for failed tasks
- Add watchdog process for stuck execution detection
- Set Cloud Run request timeout appropriately

## Related Runbooks

- [High Error Rate](high-error-rate.md)
- [Error Budget Burn](error-budget-burn.md)
- [Database Slow](database-slow.md)
