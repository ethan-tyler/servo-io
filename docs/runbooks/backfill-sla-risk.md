# Backfill SLA At Risk Runbook

## Overview

This runbook covers proactive mitigation when backfill jobs are at risk of missing their SLA deadline. A job is considered "at risk" when its estimated completion time exceeds the configured `sla_deadline_at`.

### Related Alerts

| Alert | Threshold | Severity | Description |
|-------|-----------|----------|-------------|
| `ServoBackfillSLAAtRisk` | Any at-risk jobs | Warning | Jobs projected to miss their SLA deadline |

## Impact Assessment

### Business Impact
- Early warning allows time for mitigation before actual breach
- Stakeholders can be notified proactively
- Resources can be reallocated to prevent breach
- Downstream dependencies can be adjusted if needed

### Affected Components
- Backfill jobs with configured SLA deadlines
- Resource allocation across the system
- Downstream scheduling decisions

## Diagnosis Steps

### 1. Identify At-Risk Jobs

```bash
# List all running backfill jobs
servo backfill list --status running

# Check specific job status for ETA vs deadline comparison
servo backfill status <job_id>
```

Via Grafana:
- Navigate to: Dashboards > Servo Backfill Operations > SLA Tracking
- Look at the "SLA At Risk Jobs" panel

### 2. Analyze Risk Level

For each at-risk job, calculate:
```
Risk Severity = (ETA - Deadline) / (Deadline - Now)
```

- **Low Risk** (< 20% overage): Minor adjustments may help
- **Medium Risk** (20-50% overage): Active intervention needed
- **High Risk** (> 50% overage): Significant changes required or accept breach

### 3. Identify Bottlenecks

Check for common slowdown causes:

```bash
# Check job details
servo backfill status <job_id>
```

Review:
- `avg_partition_duration`: Is it increasing over time?
- `failed_partitions` / `completed_partitions`: High failure rate?
- `waiting_upstream_count`: Blocked on dependencies?

## Mitigation Procedures

### For Low Risk Jobs

#### Option 1: Monitor Closely
- Set up focused monitoring on the specific job
- Check progress every 15-30 minutes
- Be ready to escalate if risk increases

#### Option 2: Reduce External Load
- Defer non-critical backfills
- Reduce concurrent jobs competing for resources

### For Medium Risk Jobs

#### Option 1: Optimize Partition Execution
If partitions are taking longer than expected:
1. Check external service health (APIs, databases)
2. Review recent changes to partition logic
3. Look for data anomalies in slow partitions

#### Option 2: Pause Non-Critical Jobs
```bash
# Identify lower-priority jobs
servo backfill list --status running

# Pause jobs without SLA or with later deadlines
servo backfill pause <lower_priority_job_id>
```

#### Option 3: Extend Deadline (If Acceptable)
If stakeholders agree to a later deadline:
```bash
# Currently requires direct database update
# Future: servo backfill update-sla <job_id> --deadline "2024-01-17T00:00:00Z"
```

### For High Risk Jobs

#### Option 1: Recreate with Higher Priority
```bash
# Cancel and recreate with maximum priority
servo backfill cancel <job_id> --reason "SLA at risk - recreating with higher priority"

servo backfill start <asset> --partition <key> --priority 10 \
  --sla-deadline "2024-01-17T00:00:00Z"
```

#### Option 2: Emergency Resource Scaling
1. Contact platform team to scale executor instances
2. Temporarily reduce resource limits for other workloads
3. Enable burst capacity if available

#### Option 3: Stakeholder Communication
If breach is likely unavoidable:
1. Calculate expected completion time
2. Notify stakeholders immediately
3. Document the situation and contributing factors
4. Prepare downstream impact mitigation

## Preventive Measures

### 1. Improve ETA Accuracy

The ETA is calculated based on:
- Remaining partitions × average partition duration
- Historical accuracy of estimates

To improve:
- Ensure partition sizes are roughly uniform
- Account for known slowdowns (e.g., end-of-month data)
- Build in buffer when setting deadlines

### 2. Configure Early Warning Thresholds

Adjust alert sensitivity in `servo-alerts.yaml`:
```yaml
# Current threshold alerts when ETA > deadline
# Consider adding a buffer threshold for earlier warning
expr: servo_backfill_sla_at_risk_jobs > 0
```

### 3. Establish SLA Tiers

Define different SLA tiers with appropriate deadlines:

| Tier | Priority | Deadline Buffer | Use Case |
|------|----------|-----------------|----------|
| Critical | 8-10 | Tight (< 10%) | Revenue-impacting, customer-facing |
| Standard | 4-7 | Moderate (20-30%) | Regular operations |
| Best Effort | 0-3 | Flexible | Internal, non-urgent |

### 4. Regular SLA Review

Weekly review of:
- Jobs that came close to SLA breach
- Accuracy of ETA estimates
- Patterns in at-risk jobs (specific assets, times, etc.)

## Escalation

### When to Escalate

- Multiple high-priority jobs at risk simultaneously
- Risk level increasing rapidly
- External dependencies causing widespread slowdowns
- System-wide performance degradation

### Escalation Path

1. **On-call Engineer**: Initial assessment and standard mitigation
2. **Platform Team Lead**: Resource scaling decisions
3. **Product/Business Stakeholder**: Deadline negotiation, impact communication

## Related Runbooks

- [Backfill SLA Breach](./backfill-sla-breach.md) - When SLA is actually breached
- [Backfill Throughput](./backfill-throughput.md) - Throughput degradation issues
- [Backfill Operations](./backfill-operations.md) - General backfill troubleshooting
