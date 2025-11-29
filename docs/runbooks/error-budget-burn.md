# Error Budget Burn Runbook

## Overview

Error budget burn alerts indicate that SLO violations are consuming your reliability budget faster than sustainable. These are the most critical alerts because they directly measure user-impacting reliability degradation.

### Alert Types

| Alert | Burn Rate | Time to Exhaustion | Action |
|-------|-----------|-------------------|--------|
| `Servo*FastBurn` | 14.4x | ~2 hours | Page immediately |
| `Servo*SlowBurn` | 3x | ~10 days | Create ticket, investigate |

## Impact Assessment

### Blast Radius
- **Fast Burn**: All users experiencing degraded service
- **Slow Burn**: Gradual degradation, may affect specific tenants or workflows

### Business Impact
- Error budget exhaustion leads to deployment freezes
- Sustained violations breach SLA commitments
- Customer trust erosion if not addressed

## Detection

### Alert Expressions

```promql
# Fast burn (HTTP Availability) - 14.4x rate
(
  sum(rate(servo_http_requests_total{status_code=~"5.."}[5m]))
  / sum(rate(servo_http_requests_total[5m]))
) > (14.4 * 0.001)
and
(
  sum(rate(servo_http_requests_total{status_code=~"5.."}[1h]))
  / sum(rate(servo_http_requests_total[1h]))
) > (14.4 * 0.001)
```

### Dashboard Link
[SLO Overview Dashboard](https://grafana.servo.io/d/servo-slo-overview)

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.level="ERROR"
```

## Triage Steps

### 1. Identify the Affected SLO

Check which SLO is burning:
```bash
# View current burn rates
curl -s "http://prometheus:9090/api/v1/query?query=servo:slo:http_availability:burn_rate_1h" | jq '.data.result[0].value[1]'
curl -s "http://prometheus:9090/api/v1/query?query=servo:slo:workflow_availability:burn_rate_1h" | jq '.data.result[0].value[1]'
curl -s "http://prometheus:9090/api/v1/query?query=servo:slo:http_latency:burn_rate_1h" | jq '.data.result[0].value[1]'
```

### 2. Check Error Budget Remaining

```bash
# View remaining budget
curl -s "http://prometheus:9090/api/v1/query?query=servo:slo:http_availability:error_budget_remaining" | jq '.data.result[0].value[1]'
```

Expected: Value between 0 and 1 (e.g., 0.85 = 85% remaining)

### 3. Correlate with Recent Changes

```bash
# Check recent deployments
gcloud run revisions list --service=servo-worker --region=us-central1 --limit=5

# Check recent configuration changes
gcloud run services describe servo-worker --region=us-central1 --format="yaml(spec.template.metadata.annotations)"
```

### 4. Identify Error Pattern

```bash
# Most common errors in last 10 minutes
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=servo-worker AND jsonPayload.level=ERROR" \
  --limit=50 --format="json" | jq -r '.[].jsonPayload.message' | sort | uniq -c | sort -rn | head -10
```

## Resolution Procedures

### Scenario: Recent Deployment Caused Regression

1. **Identify the problematic revision**:
```bash
gcloud run revisions list --service=servo-worker --region=us-central1 --format="table(name,active,createTime)"
```

2. **Rollback to previous revision**:
```bash
gcloud run services update-traffic servo-worker \
  --region=us-central1 \
  --to-revisions=servo-worker-00001-abc=100
```

3. **Verify recovery**:
```bash
# Watch error rate drop
watch -n 10 'curl -s "http://prometheus:9090/api/v1/query?query=servo:slo:http_availability:burn_rate_1h" | jq ".data.result[0].value[1]"'
```

### Scenario: Database Performance Degradation

1. **Check database metrics**:
```bash
psql $DATABASE_URL -c "SELECT pid, state, query_start, query FROM pg_stat_activity WHERE state = 'active' ORDER BY query_start LIMIT 10"
```

2. **Kill long-running queries if necessary**:
```bash
psql $DATABASE_URL -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE state = 'active' AND query_start < NOW() - INTERVAL '5 minutes'"
```

3. **Scale database if needed** (Cloud SQL):
```bash
gcloud sql instances patch servo-prod-db --tier=db-custom-4-15360
```

### Scenario: External Dependency Failure

1. **Check circuit breaker status**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=servo_circuit_breaker_state" | jq '.data.result[] | {service: .metric.service, state: .value[1]}'
```

2. **Identify failing dependency**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.message:~\"circuit breaker\"" --limit=20
```

3. **Enable graceful degradation** (if not already):
```bash
# Update environment to enable fallback mode
gcloud run services update servo-worker \
  --region=us-central1 \
  --set-env-vars="SERVO_FALLBACK_ENABLED=true"
```

## Escalation

### When to Escalate

- Fast burn alert sustained for >15 minutes after initial triage
- Error budget reaches <10% remaining
- Multiple SLOs affected simultaneously
- Unable to identify root cause within 30 minutes

### Escalation Path

1. **Immediate (Fast Burn)**: Page on-call engineer via PagerDuty
2. **Within 30 minutes**: Engage Backend Lead
3. **Within 1 hour**: Engage Platform Lead
4. **Major incident**: Trigger incident commander

## Prevention

### Short-term
- Add canary deployments with automatic rollback
- Implement gradual traffic shifting for deployments
- Add pre-deployment integration tests

### Long-term
- Implement chaos engineering tests
- Add dependency health checks to readiness probes
- Create deployment windows with reduced blast radius
- Implement feature flags for quick disable

## Related Runbooks

- [High Error Rate](high-error-rate.md)
- [High Latency](high-latency.md)
- [Database Slow](database-slow.md)
