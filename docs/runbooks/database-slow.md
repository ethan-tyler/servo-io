# Database Slow Runbook

## Overview

This runbook addresses alerts when database operations are experiencing elevated latency, which impacts all system operations and multiple SLOs.

### Alert: `ServoDatabaseLatencyFastBurn` / `ServoDatabaseLatencySlowBurn`

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| DB Operation P99 | <100ms | >100ms sustained |

## Impact Assessment

### Blast Radius
- **Critical**: Database is the central bottleneck
- All HTTP requests, workflow executions, and data quality checks affected
- May trigger cascading circuit breaker openings

### Symptoms
- Elevated HTTP latency across all endpoints
- Workflow executions taking longer than expected
- Timeout errors in logs

## Detection

### Alert Expression
```promql
(
  1 - sum(rate(servo_db_operation_duration_seconds_bucket{le="0.1"}[5m]))
      / sum(rate(servo_db_operation_duration_seconds_count[5m]))
) > (14.4 * 0.01)
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - Database Operations panel

### Log Query
```
resource.type="cloud_run_revision"
jsonPayload.db_duration_ms>100
```

## Triage Steps

### 1. Check Database Latency Percentiles

```bash
# P50, P90, P99 database latencies
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.50,rate(servo_db_operation_duration_seconds_bucket[5m]))*1000" | jq '.data.result[0].value[1] + " ms"'
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.90,rate(servo_db_operation_duration_seconds_bucket[5m]))*1000" | jq '.data.result[0].value[1] + " ms"'
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,rate(servo_db_operation_duration_seconds_bucket[5m]))*1000" | jq '.data.result[0].value[1] + " ms"'
```

### 2. Check Latency by Operation Type

```bash
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,sum(rate(servo_db_operation_duration_seconds_bucket[5m])) by (operation,le))*1000" | jq '.data.result[] | {operation: .metric.operation, p99_ms: .value[1]}'
```

### 3. Check Database Instance Health

```bash
# Cloud SQL metrics
gcloud sql instances describe servo-prod-db --format="yaml(state,settings.tier,settings.dataDiskSizeGb)"

# Check CPU/memory via Cloud Monitoring
gcloud monitoring metrics list --filter="resource.type=cloudsql_database" --limit=5
```

### 4. Check Active Connections

```bash
psql $DATABASE_URL -c "
SELECT COUNT(*) as total,
       COUNT(*) FILTER (WHERE state = 'active') as active,
       COUNT(*) FILTER (WHERE state = 'idle') as idle,
       COUNT(*) FILTER (WHERE state = 'idle in transaction') as idle_in_transaction
FROM pg_stat_activity
WHERE usename = 'servo_app'"
```

### 5. Check for Long-Running Queries

```bash
psql $DATABASE_URL -c "
SELECT pid,
       state,
       EXTRACT(EPOCH FROM (NOW() - query_start)) as duration_secs,
       LEFT(query, 100) as query_preview
FROM pg_stat_activity
WHERE state = 'active'
  AND query_start < NOW() - INTERVAL '5 seconds'
ORDER BY query_start"
```

### 6. Check for Lock Contention

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

## Resolution Procedures

### Scenario: Slow Query Causing Latency

1. **Identify the slow query**:
```bash
psql $DATABASE_URL -c "
SELECT query, calls, mean_exec_time, total_exec_time, rows
FROM pg_stat_statements
ORDER BY mean_exec_time DESC
LIMIT 10"
```

2. **Analyze the query plan**:
```bash
psql $DATABASE_URL -c "EXPLAIN (ANALYZE, BUFFERS) <slow_query_here>"
```

3. **If missing index, create it**:
```sql
CREATE INDEX CONCURRENTLY idx_<table>_<columns> ON <table>(<columns>);
```

4. **If query is inefficient, optimize or cache results**

### Scenario: Connection Pool Exhaustion

1. **Check current pool usage**:
```bash
psql $DATABASE_URL -c "
SELECT COUNT(*) FROM pg_stat_activity WHERE usename = 'servo_app'"
```

2. **Kill idle connections if needed**:
```bash
psql $DATABASE_URL -c "
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE usename = 'servo_app'
  AND state = 'idle'
  AND state_change < NOW() - INTERVAL '10 minutes'"
```

3. **Increase pool size in application config**:
```bash
gcloud run services update servo-worker --region=us-central1 \
  --set-env-vars="DATABASE_POOL_SIZE=50"
```

### Scenario: Database CPU/Memory Exhaustion

1. **Check resource utilization**:
```bash
# In Cloud Console or via API
gcloud sql instances describe servo-prod-db
```

2. **Scale up the database instance**:
```bash
gcloud sql instances patch servo-prod-db --tier=db-custom-4-15360
```

3. **Consider read replicas for read-heavy workloads**:
```bash
gcloud sql instances create servo-prod-db-replica --master-instance-name=servo-prod-db
```

### Scenario: Lock Contention

1. **Identify blocking transactions**:
```bash
psql $DATABASE_URL -c "
SELECT pid, state, query, wait_event_type, wait_event
FROM pg_stat_activity
WHERE wait_event_type = 'Lock'"
```

2. **Kill blocking transaction if safe**:
```bash
psql $DATABASE_URL -c "SELECT pg_terminate_backend(<blocking_pid>)"
```

3. **Review transaction patterns for optimization**

### Scenario: Vacuum/Autovacuum Running

1. **Check autovacuum status**:
```bash
psql $DATABASE_URL -c "
SELECT schemaname, relname, last_vacuum, last_autovacuum, n_dead_tup
FROM pg_stat_user_tables
WHERE n_dead_tup > 10000
ORDER BY n_dead_tup DESC"
```

2. **If autovacuum is running, it should complete naturally**

3. **If bloat is severe, schedule manual vacuum during low traffic**

### Scenario: Network Latency

1. **Check if service and database are in same region**:
```bash
gcloud run services describe servo-worker --format="value(metadata.labels.region)"
gcloud sql instances describe servo-prod-db --format="value(region)"
```

2. **If different regions, consider migration or Private Service Connect**

## Escalation

### When to Escalate
- Database latency >500ms P99 sustained for 10+ minutes
- Connection pool exhausted
- Database instance unresponsive
- Unable to identify root cause in 30 minutes

### Escalation Path
1. **Immediate**: Page on-call
2. **15 minutes**: DBA / Backend Lead
3. **30 minutes**: Platform Lead
4. **If Cloud SQL issue**: Open GCP support case

## Prevention

- Regular query performance reviews in PR process
- Implement query timeout limits (statement_timeout)
- Set up slow query logging
- Regular index analysis and optimization
- Monitor and tune autovacuum settings
- Implement connection pooling with PgBouncer
- Use read replicas for reporting queries

## Related Runbooks

- [High Latency](high-latency.md)
- [High Error Rate](high-error-rate.md)
- [Stuck Executions](stuck-executions.md)
