# High Error Rate Runbook

## Overview

This runbook addresses alerts indicating elevated HTTP 5xx error rates. High error rates directly impact user experience and consume error budget.

### Alert: `ServoHTTPAvailabilityFastBurn` / `ServoHTTPAvailabilitySlowBurn`

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| HTTP Success Rate | 99.9% | <99.9% sustained |

## Impact Assessment

### Affected Components
- All HTTP API endpoints (`/execute`, `/health`, `/metrics`)
- Workflow trigger requests from Cloud Scheduler
- Health check failures may cause instance restarts

### User Experience
- Failed workflow triggers
- Timeout errors in client applications
- Potential data staleness if scheduled workflows fail

## Detection

### Alert Expression
```promql
sum(rate(servo_http_requests_total{status_code=~"5.."}[5m]))
/ sum(rate(servo_http_requests_total[5m]))
> 0.001
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - HTTP Status Distribution panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.status_code>=500
```

## Triage Steps

### 1. Quantify the Error Rate

```bash
# Current error rate percentage
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_http_requests_total{status_code=~\"5..\"}[5m]))/sum(rate(servo_http_requests_total[5m]))*100" | jq '.data.result[0].value[1]'
```

Expected output: Number (e.g., "0.15" = 0.15% error rate)

### 2. Identify Error Distribution by Status Code

```bash
# Breakdown by status code
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_http_requests_total{status_code=~\"5..\"}[5m])) by (status_code)" | jq '.data.result[] | {code: .metric.status_code, rate: .value[1]}'
```

Common codes:
- **500**: Internal Server Error (application bug or unhandled exception)
- **502**: Bad Gateway (upstream dependency failure)
- **503**: Service Unavailable (overload or circuit breaker open)
- **504**: Gateway Timeout (slow response from backend)

### 3. Identify Affected Endpoints

```bash
# Error rate by endpoint
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_http_requests_total{status_code=~\"5..\"}[5m])) by (endpoint)" | jq '.data.result[] | {endpoint: .metric.endpoint, rate: .value[1]}'
```

### 4. Check Recent Error Logs

```bash
# Most recent errors with stack traces
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=servo-worker AND jsonPayload.level=ERROR" \
  --limit=20 --format="json" | jq -r '.[] | "\(.timestamp) \(.jsonPayload.message)"'
```

### 5. Check Service Health

```bash
# Direct health check
curl -v https://servo-worker-xxx.run.app/health

# Check instance status
gcloud run services describe servo-worker --region=us-central1 --format="yaml(status.conditions)"
```

## Resolution Procedures

### Scenario: 500 Internal Server Error (Application Bug)

1. **Identify the error pattern**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.level=ERROR" \
  --limit=50 --format="json" | jq -r '.[].jsonPayload | "\(.message) - \(.error // "no stack")"' | head -20
```

2. **Check for panic/crash loops**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND textPayload:~\"panic\"" --limit=10
```

3. **If caused by recent deployment, rollback**:
```bash
gcloud run services update-traffic servo-worker --region=us-central1 --to-revisions=PREVIOUS_REVISION=100
```

### Scenario: 502/504 Upstream Dependency Failure

1. **Check database connectivity**:
```bash
psql $DATABASE_URL -c "SELECT 1"
```

2. **Check Cloud Tasks queue**:
```bash
gcloud tasks queues describe servo-execution-queue --location=us-central1
```

3. **Check if circuit breaker is open**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=servo_circuit_breaker_state" | jq '.data.result'
```

4. **If database issue, see [Database Slow](database-slow.md)**

### Scenario: 503 Service Overload

1. **Check current instance count**:
```bash
gcloud run services describe servo-worker --region=us-central1 --format="yaml(status.traffic)"
```

2. **Check rate limiting**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_rate_limit_rejections_total[5m]))" | jq '.data.result[0].value[1]'
```

3. **Scale up if needed**:
```bash
gcloud run services update servo-worker --region=us-central1 --min-instances=10 --max-instances=100
```

### Scenario: Memory/CPU Exhaustion

1. **Check resource usage**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.message:~\"OOM\\|out of memory\"" --limit=10
```

2. **Increase resources**:
```bash
gcloud run services update servo-worker --region=us-central1 --memory=2Gi --cpu=2
```

## Escalation

### When to Escalate
- Error rate >1% sustained for 10+ minutes
- Unable to identify root cause within 30 minutes
- Rollback doesn't resolve the issue
- Multiple tenants reporting issues

### Escalation Path
1. **Immediate**: Page on-call via PagerDuty
2. **15 minutes**: Backend Lead
3. **30 minutes**: Platform Lead

## Prevention

- Implement structured error handling with proper error types
- Add circuit breakers for all external dependencies
- Implement request validation at API boundary
- Add integration tests for error scenarios
- Monitor deployment health with canary analysis

## Related Runbooks

- [Error Budget Burn](error-budget-burn.md)
- [Database Slow](database-slow.md)
- [Circuit Breaker Open](circuit-breaker-open.md)
