# Servo Incident Response Runbooks

This directory contains actionable runbooks for responding to Servo platform alerts.
Each runbook follows a consistent structure to minimize Mean Time to Resolution (MTTR).

## Runbook Index

| Alert Category | Runbook | Severity | SLO Impact |
|---------------|---------|----------|------------|
| **SLO Burn Rate** | [Error Budget Burn](error-budget-burn.md) | Critical/Warning | Direct |
| **Availability** | [High Error Rate](high-error-rate.md) | Critical | HTTP Availability |
| **Latency** | [High Latency](high-latency.md) | Critical | HTTP Latency |
| **Resilience** | [Circuit Breaker Open](circuit-breaker-open.md) | Warning | Availability |
| **Execution** | [Stuck Executions](stuck-executions.md) | Warning | Workflow Duration |
| **Rate Limiting** | [Rate Limit Rejections](rate-limit-rejections.md) | Warning | Rate Limit SLO |
| **Database** | [Database Slow](database-slow.md) | Critical | All SLOs |
| **Data Quality** | [Data Quality Failures](data-quality-failures.md) | Warning | Data Quality |
| **Traffic** | [No Traffic](no-traffic.md) | Critical | All SLOs |
| **Backfill** | [Backfill Operations](backfill-operations.md) | Warning | Workflow Duration |

## Runbook Structure

Each runbook follows this template:

1. **Overview**: What the alert means and why it matters
2. **Impact Assessment**: Affected users, business impact, blast radius
3. **Detection**: Alert expression, dashboard link, log query
4. **Triage Steps**: Numbered diagnostic commands with expected output
5. **Resolution Procedures**: Scenario-based fixes with commands
6. **Escalation**: When and who to escalate to
7. **Prevention**: Long-term fixes and improvements

## Quick Reference

### Common Diagnostic Commands

```bash
# Check service health
gcloud run services describe servo-worker --region=us-central1

# View recent logs (last 10 minutes)
gcloud logging read "resource.type=cloud_run_revision AND resource.labels.service_name=servo-worker" \
  --limit=100 --format="table(timestamp,jsonPayload.level,jsonPayload.message)"

# Check execution status
psql $DATABASE_URL -c "SELECT state, COUNT(*) FROM executions WHERE created_at > NOW() - INTERVAL '1 hour' GROUP BY state"

# View active connections
psql $DATABASE_URL -c "SELECT COUNT(*) FROM pg_stat_activity WHERE state = 'active'"
```

### SLO Thresholds

| SLO | Target | Error Budget (30d) | Fast Burn Alert | Slow Burn Alert |
|-----|--------|-------------------|-----------------|-----------------|
| HTTP Availability | 99.9% | 43.2 min | >14.4x (2min) | >3x (5min) |
| Workflow Availability | 99.9% | 43.2 min | >14.4x (2min) | >3x (5min) |
| HTTP Latency (P99 < 500ms) | 99% | 7.2 hours | >14.4x (2min) | >6x (5min) |
| Workflow Duration (P95 < 5min) | 95% | 36 hours | >14.4x (2min) | >3x (10min) |
| Data Quality Pass Rate | 95% | 8.4 hours | >10x (2min) | >2x (10min) |

### Escalation Contacts

| Role | Slack Channel | PagerDuty | Hours |
|------|--------------|-----------|-------|
| On-Call Engineer | #servo-oncall | @servo-primary | 24/7 |
| Backend Lead | #servo-backend | @backend-lead | Business |
| Platform Lead | #servo-platform | @platform-lead | Business |
| Incident Commander | #incidents | @incident-commander | Major incidents |

## Dashboard Links

- [SLO Overview](https://grafana.servo.io/d/servo-slo-overview)
- [Workflow Execution](https://grafana.servo.io/d/servo-workflow-execution)
- [Data Quality](https://grafana.servo.io/d/servo-data-quality)
- [Infrastructure](https://grafana.servo.io/d/servo-infrastructure)
- [GCP Cloud Console](https://console.cloud.google.com/run?project=servo-prod)
