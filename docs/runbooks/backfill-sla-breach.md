# Backfill SLA Breach Runbook

## Overview

This runbook covers procedures for handling backfill jobs that have breached their SLA deadline. An SLA breach occurs when a job with a configured `sla_deadline_at` is still active past that deadline.

### Related Alerts

| Alert | Threshold | Severity | Description |
|-------|-----------|----------|-------------|
| `ServoBackfillSLABreach` | Any breached jobs | Critical | Jobs have exceeded their SLA deadline |

## Impact Assessment

### Business Impact
- Downstream processes depending on backfilled data may be delayed
- SLA commitments to customers may be violated
- Data freshness expectations not met
- Potential revenue or reporting impact if data is time-sensitive

### Affected Components
- Backfill jobs with configured SLA deadlines
- Downstream data pipelines waiting for backfilled data
- Business processes dependent on complete historical data

## Diagnosis Steps

### 1. Identify Breached Jobs

```bash
# List all backfill jobs to find breached ones
servo backfill list --status running

# Or query directly via Grafana dashboard:
# Navigate to: Dashboards > Servo Backfill Operations > SLA Tracking
```

### 2. Check Job Details

```bash
# Get detailed status for the breached job
servo backfill status <job_id>
```

Review:
- `sla_deadline_at`: When was the deadline?
- `estimated_completion_at`: What is the current ETA?
- `completed_partitions` / `total_partitions`: How much progress has been made?
- `failed_partitions`: Are failures slowing progress?

### 3. Identify Root Cause

Common causes of SLA breach:
1. **Unrealistic deadline**: Deadline set too aggressively for data volume
2. **Slow partition execution**: External dependencies (APIs, DBs) slower than expected
3. **High failure rate**: Partitions failing and retrying, consuming time
4. **Resource contention**: Multiple backfills competing for resources
5. **Upstream dependencies**: Waiting on upstream jobs to complete
6. **Job paused**: Job was paused and not resumed in time

## Resolution Procedures

### For Jobs Still Running

#### Option 1: Expedite the Job (Increase Priority)

If the job should complete ASAP:
```bash
# Note: Priority cannot be changed after job creation
# Consider cancelling and recreating with higher priority

servo backfill cancel <job_id> --reason "SLA breach - recreating with higher priority"

servo backfill start <asset> --partition <key> --priority 10 \
  --sla-deadline "2024-01-17T00:00:00Z"
```

#### Option 2: Add Resources

If the executor is resource-constrained:
1. Scale up executor instances (if available)
2. Reduce concurrent workloads on the system
3. Optimize partition execution (investigate slow partitions)

#### Option 3: Accept the Breach

If the job cannot be expedited:
1. Notify stakeholders of the delay
2. Update downstream systems to expect delayed data
3. Document the breach for post-mortem

### For Jobs Waiting on Upstream

```bash
# Check upstream job status
servo backfill status <parent_job_id>
```

If upstream jobs are the bottleneck, apply the same resolution options to them first.

### For Paused Jobs

```bash
# Resume immediately
servo backfill resume <job_id>
```

## Prevention

### Setting Realistic SLA Deadlines

Calculate deadline based on:
- Partition count × average partition duration
- Add buffer for retries (typically 20-30%)
- Account for queue wait time
- Consider peak vs off-peak performance

Example:
```bash
# For 30 partitions at ~2 min each with 30% buffer:
# 30 × 2 × 1.3 = 78 minutes
# Set deadline ~2 hours from now

servo backfill start my_asset --start 2024-01-01 --end 2024-01-30 \
  --sla-deadline "$(date -u -v+2H '+%Y-%m-%dT%H:%M:%SZ')"
```

### Monitoring SLA Risk

1. Set up dashboards to track jobs approaching SLA
2. Configure `ServoBackfillSLAAtRisk` alert for early warning
3. Review SLA compliance metrics regularly

## Post-Incident

1. Document the breach: job ID, deadline, actual completion time
2. Analyze root cause: why was deadline missed?
3. Adjust future SLA targets based on actual performance
4. Update monitoring thresholds if needed
5. Communicate lessons learned to stakeholders

## Related Runbooks

- [Backfill Operations](./backfill-operations.md) - General backfill troubleshooting
- [Backfill SLA Risk](./backfill-sla-risk.md) - Proactive SLA risk mitigation
- [Backfill Throughput](./backfill-throughput.md) - Throughput degradation issues
