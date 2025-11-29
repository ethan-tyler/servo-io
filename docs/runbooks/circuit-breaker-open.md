# Circuit Breaker Open Runbook

## Overview

This runbook addresses alerts when a circuit breaker has opened, indicating repeated failures to a downstream dependency. The circuit breaker pattern protects the system from cascading failures.

### Alert: `ServoCircuitBreakerOpen`

| Metric | Normal State | Alert Threshold |
|--------|--------------|-----------------|
| Circuit Breaker State | Closed (0) | Open (2) for >1 minute |

## Impact Assessment

### Affected Components
- Requests to the protected service are failing fast
- Dependent features may be degraded
- Error rates will be elevated until circuit closes

### Circuit Breaker States
- **Closed (0)**: Normal operation, requests pass through
- **Half-Open (1)**: Testing recovery with limited requests
- **Open (2)**: Failing fast, no requests to dependency

## Detection

### Alert Expression
```promql
servo_circuit_breaker_state == 2
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - Circuit Breaker panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.message:~"circuit breaker"
```

## Triage Steps

### 1. Identify Which Circuit Breaker is Open

```bash
# Check all circuit breakers
curl -s "http://prometheus:9090/api/v1/query?query=servo_circuit_breaker_state" | jq '.data.result[] | {service: .metric.service, state: .value[1], state_name: (if .value[1] == "0" then "closed" elif .value[1] == "1" then "half-open" else "open" end)}'
```

### 2. Check Failure History

```bash
# Recent circuit breaker transitions
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.message:~\"circuit breaker\"" --limit=20 --format="json" | jq -r '.[] | "\(.timestamp) \(.jsonPayload.message)"'
```

### 3. Check Underlying Dependency Health

Based on which circuit breaker is open:

**Database Circuit Breaker**:
```bash
psql $DATABASE_URL -c "SELECT 1"
psql $DATABASE_URL -c "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active'"
```

**Cloud Tasks Circuit Breaker**:
```bash
gcloud tasks queues describe servo-execution-queue --location=us-central1
```

**External API Circuit Breaker**:
```bash
# Check the specific external endpoint
curl -v https://external-api.example.com/health
```

### 4. Check Error Patterns Before Circuit Opened

```bash
# Errors in the 5 minutes before circuit opened
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.level=ERROR AND timestamp>\"$(date -u -v-10M +%Y-%m-%dT%H:%M:%SZ)\"" --limit=50
```

## Resolution Procedures

### Scenario: Database Connectivity Issue

1. **Check database status**:
```bash
gcloud sql instances describe servo-prod-db --format="yaml(state,settings.ipConfiguration)"
```

2. **If database is healthy, check connection pool**:
```bash
# Connection pool exhaustion
psql $DATABASE_URL -c "SELECT COUNT(*) FROM pg_stat_activity WHERE usename = 'servo_app'"
```

3. **Restart the service to reset connections**:
```bash
gcloud run services update servo-worker --region=us-central1 --set-env-vars="RESTART_TIMESTAMP=$(date +%s)"
```

### Scenario: Cloud Tasks Queue Issue

1. **Check queue health**:
```bash
gcloud tasks queues describe servo-execution-queue --location=us-central1
```

2. **If queue is paused, resume it**:
```bash
gcloud tasks queues resume servo-execution-queue --location=us-central1
```

3. **If queue rate limited, check for backlog**:
```bash
gcloud tasks list --queue=servo-execution-queue --location=us-central1 --limit=10
```

### Scenario: External API Unavailable

1. **Verify external API status**:
   - Check status page of the external service
   - Try direct curl to the API

2. **If extended outage, enable fallback mode**:
```bash
gcloud run services update servo-worker --region=us-central1 --set-env-vars="SERVO_FALLBACK_ENABLED=true"
```

3. **If API is recovering, manually test**:
```bash
# Force a half-open test
curl -X POST https://servo-worker.run.app/internal/circuit-breaker/test?service=external-api
```

### Forcing Circuit Breaker Reset

If the dependency has recovered but the circuit hasn't closed:

1. **Wait for half-open period** (usually 30 seconds)

2. **Or restart the service** to reset circuit state:
```bash
gcloud run services update servo-worker --region=us-central1 --set-env-vars="RESTART_TIMESTAMP=$(date +%s)"
```

## Escalation

### When to Escalate
- Circuit open for >10 minutes after dependency recovery
- Multiple circuit breakers open simultaneously
- Root cause unclear after 15 minutes
- User-facing impact confirmed

### Escalation Path
1. **Immediate**: Notify on-call
2. **10 minutes**: Backend Lead
3. **20 minutes**: If external dependency, contact vendor

## Prevention

- Monitor dependency health proactively
- Implement graceful degradation for non-critical features
- Set up alerting on dependency SLOs
- Configure circuit breaker thresholds based on dependency SLA
- Implement retry with exponential backoff before circuit opens

## Circuit Breaker Configuration

Current configuration in `servo-storage/src/circuit_breaker.rs`:

| Parameter | Value | Description |
|-----------|-------|-------------|
| Failure Threshold | 5 | Failures before opening |
| Success Threshold | 3 | Successes to close from half-open |
| Half-Open Timeout | 30s | Time before testing recovery |
| Open Timeout | 60s | Time in open state before half-open |

## Related Runbooks

- [High Error Rate](high-error-rate.md)
- [Database Slow](database-slow.md)
- [Error Budget Burn](error-budget-burn.md)
