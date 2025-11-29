# High Latency Runbook

## Overview

This runbook addresses alerts indicating elevated HTTP response latency. High latency impacts user experience and may indicate resource contention or degraded dependencies.

### Alert: `ServoHTTPLatencyFastBurn` / `ServoHTTPLatencySlowBurn`

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| HTTP P99 Latency | <500ms | >500ms sustained |

## Impact Assessment

### Affected Users
- All API consumers experiencing slow responses
- Cloud Scheduler triggers may timeout
- Client applications may retry, causing load amplification

### Symptoms
- Slow workflow trigger responses
- Increased timeout errors
- Queued requests building up

## Detection

### Alert Expression
```promql
(
  1 - sum(rate(servo_http_request_duration_seconds_bucket{le="0.5"}[5m]))
      / sum(rate(servo_http_request_duration_seconds_count[5m]))
) > (14.4 * 0.01)
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - Latency Percentiles panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.duration_ms>500
```

## Triage Steps

### 1. Check Current Latency Percentiles

```bash
# P50, P90, P99 latencies
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.50,rate(servo_http_request_duration_seconds_bucket[5m]))" | jq '.data.result[0].value[1]'
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.90,rate(servo_http_request_duration_seconds_bucket[5m]))" | jq '.data.result[0].value[1]'
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,rate(servo_http_request_duration_seconds_bucket[5m]))" | jq '.data.result[0].value[1]'
```

### 2. Identify Slow Endpoints

```bash
# P99 by endpoint
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,sum(rate(servo_http_request_duration_seconds_bucket[5m])) by (endpoint,le))" | jq '.data.result[] | {endpoint: .metric.endpoint, p99_ms: (.value[1] | tonumber * 1000)}'
```

### 3. Check Database Latency

```bash
# Database operation P99
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,rate(servo_db_operation_duration_seconds_bucket[5m]))" | jq '.data.result[0].value[1]'
```

Expected: <100ms. If higher, database is likely the bottleneck.

### 4. Check CPU/Memory Pressure

```bash
# Container resource metrics
gcloud monitoring metrics list --filter="resource.type=cloud_run_revision AND metric.type:~\"container\"" --limit=10
```

### 5. Check for Lock Contention

```bash
# Active database connections and locks
psql $DATABASE_URL -c "SELECT pid, state, wait_event_type, wait_event, query FROM pg_stat_activity WHERE state = 'active' ORDER BY query_start LIMIT 20"
```

## Resolution Procedures

### Scenario: Database Query Latency

1. **Identify slow queries**:
```bash
# In PostgreSQL, enable pg_stat_statements if not already
psql $DATABASE_URL -c "SELECT query, calls, mean_exec_time, total_exec_time FROM pg_stat_statements ORDER BY mean_exec_time DESC LIMIT 10"
```

2. **Check for missing indexes**:
```bash
psql $DATABASE_URL -c "SELECT schemaname, relname, seq_scan, idx_scan FROM pg_stat_user_tables WHERE seq_scan > idx_scan ORDER BY seq_scan DESC LIMIT 10"
```

3. **Add missing indexes** (if identified):
```sql
CREATE INDEX CONCURRENTLY idx_executions_tenant_state ON executions(tenant_id, state);
```

4. **Scale database** (Cloud SQL):
```bash
gcloud sql instances patch servo-prod-db --tier=db-custom-4-15360
```

### Scenario: CPU Exhaustion

1. **Check CPU utilization**:
```bash
gcloud run services describe servo-worker --format="yaml(status)"
```

2. **Scale horizontally**:
```bash
gcloud run services update servo-worker --region=us-central1 --min-instances=10
```

3. **Scale vertically**:
```bash
gcloud run services update servo-worker --region=us-central1 --cpu=4 --memory=4Gi
```

### Scenario: Memory Pressure (GC Pauses)

1. **Check memory usage patterns**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND textPayload:~\"memory\\|heap\\|gc\"" --limit=20
```

2. **Increase memory**:
```bash
gcloud run services update servo-worker --region=us-central1 --memory=4Gi
```

### Scenario: Network Latency to Dependencies

1. **Check if running in same region as database**:
```bash
gcloud run services describe servo-worker --format="value(metadata.labels.region)"
gcloud sql instances describe servo-prod-db --format="value(region)"
```

2. **If different regions, consider migration** or VPC setup

### Scenario: Cold Start Latency

1. **Check cold start frequency**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.message:~\"cold start\"" --limit=20
```

2. **Increase minimum instances to reduce cold starts**:
```bash
gcloud run services update servo-worker --region=us-central1 --min-instances=5
```

## Escalation

### When to Escalate
- P99 latency >2s sustained for 10+ minutes
- Database queries consistently >500ms
- CPU at 100% despite scaling attempts
- Unable to identify root cause within 30 minutes

### Escalation Path
1. **Immediate**: Page on-call
2. **15 minutes**: Backend Lead
3. **30 minutes**: DBA/Platform Lead

## Prevention

- Implement query optimization reviews in PR process
- Add database query timing to integration tests
- Set up auto-scaling based on latency metrics
- Implement connection pooling with appropriate limits
- Add caching for frequently accessed data
- Profile application regularly for hot spots

## Related Runbooks

- [Error Budget Burn](error-budget-burn.md)
- [Database Slow](database-slow.md)
- [Stuck Executions](stuck-executions.md)
