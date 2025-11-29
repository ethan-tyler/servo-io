# Rate Limit Rejections Runbook

## Overview

This runbook addresses alerts when rate limiting is rejecting a significant number of requests, which may indicate abuse, misconfiguration, or legitimate traffic spikes.

### Alert: `ServoRateLimitingFastBurn` / `ServoRateLimitingSlowBurn`

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| Acceptance Rate | 99.5% | <99.5% sustained |

## Impact Assessment

### Affected Users
- Tenants hitting rate limits receive 429 responses
- Legitimate requests may be rejected during traffic spikes
- Client retry storms can exacerbate the issue

### Rate Limiter Types
- **Global**: Protects overall system capacity
- **Per-Tenant**: Fair usage enforcement
- **Per-Endpoint**: Protects expensive operations

## Detection

### Alert Expression
```promql
(
  sum(rate(servo_rate_limit_rejections_total[5m]))
  / sum(rate(servo_http_requests_total[5m]))
) > 0.005
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - Rate Limiter panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.status_code=429
```

## Triage Steps

### 1. Identify Rejection Rate

```bash
# Overall rejection rate
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_rate_limit_rejections_total[5m]))/sum(rate(servo_http_requests_total[5m]))*100" | jq '.data.result[0].value[1]'
```

### 2. Identify Which Limiter is Rejecting

```bash
# Rejections by limiter type
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_rate_limit_rejections_total[5m])) by (limiter_type)" | jq '.data.result[] | {limiter: .metric.limiter_type, rate: .value[1]}'
```

### 3. Identify Affected Tenants

```bash
# Top tenants by rejections
curl -s "http://prometheus:9090/api/v1/query?query=topk(10, sum(rate(servo_rate_limit_rejections_total[5m])) by (tenant_id))" | jq '.data.result[] | {tenant: .metric.tenant_id, rate: .value[1]}'
```

### 4. Check Traffic Patterns

```bash
# Request rate over time
curl -s "http://prometheus:9090/api/v1/query_range?query=sum(rate(servo_http_requests_total[1m]))&start=$(date -u -v-1H +%Y-%m-%dT%H:%M:%SZ)&end=$(date -u +%Y-%m-%dT%H:%M:%SZ)&step=60s" | jq '.data.result[0].values | .[-10:]'
```

### 5. Check for Abuse Patterns

```bash
# Check for unusual request patterns from specific IPs/tenants
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.status_code=429" --limit=100 --format="json" | jq -r '.[].jsonPayload | "\(.tenant_id) \(.remote_addr)"' | sort | uniq -c | sort -rn | head -10
```

## Resolution Procedures

### Scenario: Legitimate Traffic Spike

1. **Verify traffic is legitimate**:
   - Check if it correlates with expected business events
   - Verify request patterns look normal

2. **Temporarily increase rate limits**:
```bash
gcloud run services update servo-worker --region=us-central1 \
  --set-env-vars="SERVO_RATE_LIMIT_GLOBAL=2000,SERVO_RATE_LIMIT_PER_TENANT=100"
```

3. **Scale up instances to handle load**:
```bash
gcloud run services update servo-worker --region=us-central1 --min-instances=20 --max-instances=200
```

### Scenario: Single Tenant Abuse

1. **Identify the tenant**:
```bash
# Top rejecting tenant
curl -s "http://prometheus:9090/api/v1/query?query=topk(1, sum(rate(servo_rate_limit_rejections_total[5m])) by (tenant_id))" | jq '.data.result[0].metric.tenant_id'
```

2. **Check tenant's request volume**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_http_requests_total{tenant_id=\"<tenant_id>\"}[5m]))" | jq '.data.result[0].value[1]'
```

3. **If abuse, apply stricter limits for tenant**:
```bash
# Add tenant-specific rate limit (requires code deployment)
# Or temporarily block tenant at load balancer level
```

4. **Contact tenant if legitimate overuse**

### Scenario: Client Retry Storm

1. **Identify retry patterns**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.status_code=429" --limit=50 --format="json" | jq -r '.[] | "\(.timestamp) \(.jsonPayload.remote_addr)"' | sort | uniq -c | sort -rn
```

2. **If clients not implementing backoff**:
   - Add `Retry-After` header to 429 responses
   - Contact client developers to implement exponential backoff

### Scenario: Rate Limit Configuration Too Aggressive

1. **Review current limits**:
```bash
gcloud run services describe servo-worker --region=us-central1 --format="json" | jq '.spec.template.spec.containers[0].env[] | select(.name | startswith("SERVO_RATE_LIMIT"))'
```

2. **Analyze typical traffic patterns**:
```bash
# P95 requests per tenant per minute
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.95, sum(rate(servo_http_requests_total[5m])) by (tenant_id, le))"
```

3. **Adjust limits based on analysis**:
```bash
gcloud run services update servo-worker --region=us-central1 \
  --set-env-vars="SERVO_RATE_LIMIT_PER_TENANT=200"
```

## Rate Limit Configuration

Current configuration:

| Limiter | Limit | Window | Description |
|---------|-------|--------|-------------|
| Global | 1000 req/s | 1 second | Overall system protection |
| Per-Tenant | 50 req/s | 1 second | Fair usage per tenant |
| Execution Trigger | 10 req/min | 1 minute | Prevent execution flooding |

## Escalation

### When to Escalate
- Rate limiting affecting >5% of requests sustained
- Legitimate high-priority tenant being limited
- Unable to distinguish abuse from legitimate traffic
- Rate limits need permanent adjustment

### Escalation Path
1. **Immediate**: Notify on-call
2. **15 minutes**: Backend Lead
3. **If abuse**: Security team

## Prevention

- Implement adaptive rate limiting based on system capacity
- Add tenant tier-based rate limits (enterprise vs free)
- Implement request coalescing for bursty clients
- Add rate limit status endpoint for clients to check remaining quota
- Document rate limits in API documentation

## Related Runbooks

- [High Error Rate](high-error-rate.md) - 429s count as client errors
- [Error Budget Burn](error-budget-burn.md)
