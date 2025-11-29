# No Traffic Runbook

## Overview

This runbook addresses alerts when the service is receiving no traffic, which may indicate a routing issue, service outage, or deployment failure.

### Alert: `ServoNoTraffic`

| Metric | Expected | Alert Threshold |
|--------|----------|-----------------|
| Request Rate | >0 req/min | 0 requests for 5 minutes |

## Impact Assessment

### Severity: Critical
- Complete service outage
- All users affected
- Scheduled workflows not executing
- Health checks may be failing

### Potential Causes
- Service crashed and not restarting
- DNS/routing misconfiguration
- Load balancer issue
- Region-wide GCP outage
- Deployment failure

## Detection

### Alert Expression
```promql
sum(rate(servo_http_requests_total[5m])) == 0
```

### Dashboard
[Infrastructure Dashboard](https://grafana.servo.io/d/servo-infrastructure) - Request Rate panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
```

## Triage Steps

### 1. Verify Alert is Not False Positive

```bash
# Check if we're in a known maintenance window
# Check if this is expected low-traffic period

# Verify Prometheus is collecting metrics
curl -s "http://prometheus:9090/api/v1/query?query=up{job=\"servo-worker\"}" | jq '.data.result'
```

### 2. Check Service Status

```bash
# Check Cloud Run service status
gcloud run services describe servo-worker --region=us-central1 --format="yaml(status)"

# Check if any revisions are serving
gcloud run services describe servo-worker --region=us-central1 --format="yaml(spec.traffic)"
```

### 3. Check Instance Health

```bash
# Check if instances are running
gcloud run revisions list --service=servo-worker --region=us-central1 --limit=5

# Check for recent container crashes
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=servo-worker AND severity>=ERROR" --limit=20
```

### 4. Check DNS and Routing

```bash
# Resolve service URL
host servo-worker-xxx.run.app

# Check if service is reachable
curl -v https://servo-worker-xxx.run.app/health

# Check from different location
curl -v --resolve servo-worker-xxx.run.app:443:$(dig +short servo-worker-xxx.run.app) https://servo-worker-xxx.run.app/health
```

### 5. Check Cloud Scheduler Triggers

```bash
# List scheduled jobs
gcloud scheduler jobs list --location=us-central1

# Check recent job executions
gcloud scheduler jobs describe <job_name> --location=us-central1
```

### 6. Check GCP Status

```bash
# Check GCP status page
open https://status.cloud.google.com/
```

## Resolution Procedures

### Scenario: Service Crashed (Container Startup Failure)

1. **Check container logs**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=servo-worker AND textPayload:~\"starting\\|failed\\|error\"" --limit=30
```

2. **Check for configuration issues**:
```bash
gcloud run services describe servo-worker --format="yaml(spec.template.spec.containers[0])"
```

3. **If recent deployment caused the issue, rollback**:
```bash
# List revisions
gcloud run revisions list --service=servo-worker --region=us-central1

# Rollback to previous working revision
gcloud run services update-traffic servo-worker --region=us-central1 --to-revisions=<previous_revision>=100
```

### Scenario: Missing Environment Variables

1. **Check required environment variables**:
```bash
gcloud run services describe servo-worker --format="json" | jq '.spec.template.spec.containers[0].env'
```

2. **Compare with expected configuration**:
```bash
# Required variables: DATABASE_URL, GCP_PROJECT_ID, etc.
```

3. **Fix missing variables**:
```bash
gcloud run services update servo-worker --region=us-central1 --set-env-vars="MISSING_VAR=value"
```

### Scenario: Database Connection Failure

1. **Check database connectivity**:
```bash
psql $DATABASE_URL -c "SELECT 1"
```

2. **If VPC connector issue**:
```bash
gcloud run services describe servo-worker --format="yaml(spec.template.metadata.annotations)"
```

3. **Check VPC connector status**:
```bash
gcloud compute networks vpc-access connectors describe <connector_name> --region=us-central1
```

### Scenario: DNS/Routing Issue

1. **Check domain mapping**:
```bash
gcloud run domain-mappings list
```

2. **If using custom domain, verify DNS records**:
```bash
dig servo.example.com
```

3. **If Cloud Load Balancer, check backend health**:
```bash
gcloud compute backend-services get-health <backend_name> --global
```

### Scenario: Resource Exhaustion (OOM/CPU Limit)

1. **Check for OOM kills**:
```bash
gcloud logging read "resource.type=cloud_run_revision AND textPayload:~\"OOM\\|killed\\|memory\"" --limit=20
```

2. **Increase resources**:
```bash
gcloud run services update servo-worker --region=us-central1 --memory=2Gi --cpu=2
```

### Scenario: Cloud Scheduler Not Triggering

1. **Check job status**:
```bash
gcloud scheduler jobs describe <job_name> --location=us-central1
```

2. **Check for permission issues**:
```bash
# Verify service account has invoker permissions
gcloud run services get-iam-policy servo-worker --region=us-central1
```

3. **Manually trigger to test**:
```bash
gcloud scheduler jobs run <job_name> --location=us-central1
```

### Scenario: GCP Region Outage

1. **Verify GCP status**:
   - Check https://status.cloud.google.com/
   - Check Twitter @GCPcloud

2. **If region outage, consider failover**:
   - Deploy to alternate region
   - Update DNS/load balancer to route to healthy region

## Escalation

### When to Escalate
- Service down for >5 minutes
- Unable to identify root cause in 10 minutes
- Deployment rollback doesn't resolve issue
- Suspected GCP platform issue

### Escalation Path
1. **Immediate**: Page on-call (P1 severity)
2. **5 minutes**: Platform Lead
3. **10 minutes**: If GCP issue, open support case
4. **15 minutes**: Incident commander for major outage

## Prevention

- Implement multi-region deployment for disaster recovery
- Set up synthetic monitoring / uptime checks
- Implement canary deployments with automatic rollback
- Add startup health checks that fail fast
- Monitor Cloud Scheduler job success rates
- Set up redundant health checks from external locations

## Recovery Verification

After resolving the issue:

1. **Verify service is responding**:
```bash
curl https://servo-worker-xxx.run.app/health
```

2. **Verify metrics are being collected**:
```bash
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_http_requests_total[1m]))" | jq '.data.result[0].value[1]'
```

3. **Verify scheduled workflows are executing**:
```bash
psql $DATABASE_URL -c "SELECT COUNT(*) FROM executions WHERE created_at > NOW() - INTERVAL '10 minutes'"
```

## Related Runbooks

- [High Error Rate](high-error-rate.md)
- [Error Budget Burn](error-budget-burn.md)
