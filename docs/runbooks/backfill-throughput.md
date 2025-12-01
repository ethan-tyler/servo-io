# Backfill Throughput Degradation Runbook

## Overview

This runbook covers diagnosis and resolution of backfill throughput degradation. Throughput is measured by the rate of partition completions and overall job progress.

### Related Alerts

| Alert | Threshold | Severity | Description |
|-------|-----------|----------|-------------|
| `ServoBackfillThroughputDegraded` | 50% drop over 1 hour | Warning | Partition completion rate significantly decreased |
| `ServoBackfillHighClaimLatency` | P95 > 60s for 5 min | Warning | Jobs waiting too long to be claimed |
| `ServoBackfillTenantHighFailureRate` | > 30% for 10 min | Warning | High failure rate for specific tenant |

## Impact Assessment

### Business Impact
- Backfill jobs take longer to complete
- SLA deadlines may be at risk
- Resource utilization inefficient
- Downstream processes delayed

### Affected Components
- Backfill executors
- External data sources (APIs, databases)
- Job scheduling and claiming
- Network connectivity

## Diagnosis Steps

### 1. Verify Throughput Degradation

```bash
# Check recent job completions via Grafana
# Dashboard: Servo Backfill Operations > Throughput Analysis
```

Key metrics to check:
- `servo_backfill_partitions_processed_total` rate
- `servo_backfill_job_duration_seconds` percentiles
- `servo_backfill_job_claim_latency_seconds`

### 2. Identify Scope

Determine if degradation is:
- **Global**: Affecting all jobs across tenants
- **Tenant-specific**: Only certain tenants affected
- **Asset-specific**: Only certain asset types affected

```bash
# Check tenant breakdown
# Grafana: Backfill Operations > Tenant View
```

### 3. Check Executor Health

```bash
# View executor status
kubectl get pods -l app=servo-executor -n servo

# Check executor logs
kubectl logs -l app=servo-executor -n servo --tail=100

# Check resource utilization
kubectl top pods -l app=servo-executor -n servo
```

### 4. Check External Dependencies

Common external bottlenecks:
- **Database**: Connection pool exhaustion, slow queries
- **APIs**: Rate limiting, increased latency
- **Network**: Connectivity issues, DNS problems

```bash
# Check database connections
kubectl exec -it <db-pod> -- psql -c "SELECT count(*) FROM pg_stat_activity;"

# Check API health (if applicable)
curl -w "@curl-format.txt" -o /dev/null -s <api-health-endpoint>
```

### 5. Analyze Failure Patterns

```bash
# Check failure rate metric
# Grafana query: rate(servo_backfill_partitions_processed_total{status="failed"}[5m])

# Check specific job failures
servo backfill status <job_id>
```

## Resolution Procedures

### Global Throughput Degradation

#### Check 1: Executor Resources

If executors are resource-constrained:
```bash
# Scale up executors
kubectl scale deployment servo-executor -n servo --replicas=<higher_count>

# Or increase resource limits
kubectl edit deployment servo-executor -n servo
```

#### Check 2: Database Performance

If database is the bottleneck:
1. Check for long-running queries
2. Review connection pool settings
3. Check index usage
4. Consider read replica for backfill reads

#### Check 3: Concurrent Job Limits

If too many jobs running simultaneously:
```bash
# Pause lower-priority jobs
servo backfill list --status running
servo backfill pause <job_id>
```

### Tenant-Specific Degradation

#### Check 1: Tenant Data Volume

Large tenants may experience slower processing:
- Review partition sizes for the tenant
- Consider splitting large partitions
- Adjust tenant resource allocation

#### Check 2: Tenant-Specific API Issues

If tenant uses external data sources:
- Check API rate limits for that tenant
- Verify API credentials are valid
- Check for tenant-specific service degradation

### Asset-Specific Degradation

#### Check 1: Asset Configuration

Review the asset definition:
- Partition key granularity
- Data volume per partition
- External dependencies

#### Check 2: Recent Asset Changes

Check for recent changes to:
- Asset definition
- Transformation logic
- Source system behavior

### High Claim Latency

If jobs are waiting too long to be claimed:

#### Check 1: Executor Availability
```bash
# Ensure executors are running and healthy
kubectl get pods -l app=servo-executor -n servo
```

#### Check 2: Queue Depth
```bash
# Check pending jobs
servo backfill list --status pending
```

If queue is large:
- Scale executors
- Pause non-critical jobs
- Review priority settings

#### Check 3: Executor Stuck
```bash
# Check for stuck executors
kubectl logs -l app=servo-executor -n servo | grep -i "stuck\|timeout"
```

Restart stuck executors if necessary:
```bash
kubectl delete pod <stuck-executor-pod> -n servo
```

### High Failure Rate

#### Analyze Failure Reasons

```bash
# Check job details for failure information
servo backfill status <job_id>
```

Common failure causes:
1. **Transient errors**: Usually resolve with retry
2. **Data issues**: Bad data in specific partitions
3. **Resource exhaustion**: Memory/timeout issues
4. **External service errors**: API/DB unavailable

#### Resolution by Failure Type

**Transient errors**:
- Increase retry limits if appropriate
- Check external service health

**Data issues**:
- Identify problematic partitions
- Fix source data or add data validation

**Resource exhaustion**:
- Increase partition timeout
- Add memory to executors
- Split large partitions

**External service errors**:
- Wait for service recovery
- Fail over to backup if available
- Pause affected jobs temporarily

## Prevention

### 1. Capacity Planning

- Monitor throughput trends over time
- Plan executor scaling for peak loads
- Set resource requests/limits appropriately

### 2. Early Warning Alerts

Current alerts provide early warning:
- `ServoBackfillThroughputDegraded`: 50% drop threshold
- `ServoBackfillHighClaimLatency`: Executor contention

Consider adjusting thresholds based on your SLA requirements.

### 3. Regular Health Checks

Weekly review of:
- Average throughput by tenant/asset
- Failure rate trends
- Claim latency percentiles
- Resource utilization

### 4. Load Testing

Periodically test maximum throughput:
- Run synthetic backfill jobs
- Identify bottlenecks before production impact
- Validate scaling procedures

## Escalation

### When to Escalate

- Throughput degradation > 50% for > 30 minutes
- Multiple SLA-bound jobs affected
- Root cause not identified within 15 minutes
- External dependency issues requiring vendor contact

### Escalation Path

1. **On-call Engineer**: Initial diagnosis and standard remediation
2. **Platform Team**: Infrastructure scaling, database issues
3. **Data Engineering Lead**: Asset-specific issues, data problems
4. **External Vendor**: Third-party API or service issues

## Related Runbooks

- [Backfill SLA Breach](./backfill-sla-breach.md) - SLA has been breached
- [Backfill SLA Risk](./backfill-sla-risk.md) - SLA at risk
- [Backfill Operations](./backfill-operations.md) - General backfill troubleshooting
