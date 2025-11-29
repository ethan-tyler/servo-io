# Data Quality Failures Runbook

## Overview

This runbook addresses alerts when data quality check pass rates drop below acceptable thresholds, indicating potential data issues in pipelines.

### Alert: `ServoDataQualityFastBurn` / `ServoDataQualitySlowBurn`

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| Check Pass Rate | >95% | <95% sustained |

## Impact Assessment

### Affected Components
- Data quality checks failing on assets
- Blocking checks may halt downstream workflows
- Data consumers may receive invalid data

### Severity Levels
- **Error + Blocking**: Workflow stops, downstream assets not produced
- **Error + Non-blocking**: Data flows but may be incorrect
- **Warning**: Logged for review, no immediate action
- **Info**: Informational only

## Detection

### Alert Expression
```promql
(
  1 - sum(rate(servo_check_executions_total{outcome="passed"}[5m]))
      / sum(rate(servo_check_executions_total{outcome!="skipped"}[5m]))
) > (10.0 * 0.05)
```

### Dashboard
[Data Quality Dashboard](https://grafana.servo.io/d/servo-data-quality) - Pass Rate panel

### Log Query
```
resource.type="cloud_run_revision"
resource.labels.service_name="servo-worker"
jsonPayload.check_outcome="failed"
```

## Triage Steps

### 1. Check Current Pass Rate

```bash
# Overall pass rate
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_check_executions_total{outcome=\"passed\"}[5m]))/sum(rate(servo_check_executions_total{outcome!=\"skipped\"}[5m]))*100" | jq '.data.result[0].value[1]'
```

### 2. Identify Failing Checks by Type

```bash
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(servo_check_executions_total{outcome=\"failed\"}[5m])) by (check_type)" | jq '.data.result[] | {type: .metric.check_type, rate: .value[1]}'
```

### 3. Identify Affected Tenants

```bash
curl -s "http://prometheus:9090/api/v1/query?query=topk(10, sum(rate(servo_check_executions_total{outcome=\"failed\"}[5m])) by (tenant_id))" | jq '.data.result[] | {tenant: .metric.tenant_id, rate: .value[1]}'
```

### 4. List Recent Check Failures

```bash
psql $DATABASE_URL -c "
SELECT cr.check_id, ac.name, ac.asset_id, cr.outcome, cr.error_message,
       cr.failed_row_count, cr.total_row_count, cr.executed_at
FROM check_results cr
JOIN asset_checks ac ON cr.check_id = ac.id
WHERE cr.outcome = 'failed'
  AND cr.executed_at > NOW() - INTERVAL '1 hour'
ORDER BY cr.executed_at DESC
LIMIT 20"
```

### 5. Check for Data Source Issues

```bash
# Look for upstream data issues in logs
gcloud logging read "resource.type=cloud_run_revision AND jsonPayload.message:~\"data source\\|upstream\\|ingestion\"" --limit=20
```

## Resolution Procedures

### Scenario: Known Data Quality Issue (Expected)

1. **If temporary data issue, acknowledge and monitor**:
   - Document the issue in incident tracker
   - Set expectation for resolution timeline

2. **If check is too strict, adjust threshold**:
```bash
# Update check definition (via API or database)
psql $DATABASE_URL -c "
UPDATE asset_checks
SET check_type = jsonb_set(check_type, '{threshold}', '0.95')
WHERE id = '<check_id>'"
```

### Scenario: Upstream Data Source Issue

1. **Identify the data source**:
```bash
psql $DATABASE_URL -c "
SELECT a.name, a.source_config
FROM assets a
JOIN asset_checks ac ON ac.asset_id = a.id
WHERE ac.id = '<failing_check_id>'"
```

2. **Check source system health**:
   - Verify source database/API is accessible
   - Check for recent schema changes
   - Verify data freshness

3. **If source is down, mark check as skipped temporarily**:
```bash
psql $DATABASE_URL -c "
UPDATE asset_checks
SET enabled = false
WHERE id = '<check_id>'"
```

### Scenario: Schema Drift

1. **Compare expected vs actual schema**:
```bash
# Check for column changes
gcloud logging read "jsonPayload.message:~\"column\\|schema\\|missing\"" --limit=20
```

2. **Update check definition to match new schema**:
```bash
# Example: Update column reference
psql $DATABASE_URL -c "
UPDATE asset_checks
SET check_type = jsonb_set(check_type, '{column}', '\"new_column_name\"')
WHERE id = '<check_id>'"
```

### Scenario: Blocking Check Causing Workflow Failures

1. **Identify blocked workflows**:
```bash
psql $DATABASE_URL -c "
SELECT e.id, e.workflow_id, e.state, e.error_message
FROM executions e
WHERE e.state = 'failed'
  AND e.error_message LIKE '%blocking check%'
  AND e.completed_at > NOW() - INTERVAL '1 hour'"
```

2. **Decision: Fix check or make non-blocking**

   **Option A - Fix the check** (if data issue):
   - Fix upstream data source
   - Retry the workflow

   **Option B - Make check non-blocking** (if check is too strict):
   ```bash
   psql $DATABASE_URL -c "
   UPDATE asset_checks
   SET blocking = false
   WHERE id = '<check_id>'"
   ```

3. **Retry failed workflows**:
```bash
# Re-trigger executions
psql $DATABASE_URL -c "
UPDATE executions
SET state = 'pending',
    error_message = NULL,
    started_at = NULL,
    completed_at = NULL
WHERE id IN ('<execution_id_1>', '<execution_id_2>')"
```

### Scenario: Check Logic Bug

1. **Review check definition**:
```bash
psql $DATABASE_URL -c "
SELECT name, check_type, severity, blocking
FROM asset_checks
WHERE id = '<check_id>'"
```

2. **Test check against sample data**:
```bash
# If check is SQL-based, run manually
psql $DATABASE_URL -c "<check_query>"
```

3. **Fix check definition**:
```bash
psql $DATABASE_URL -c "
UPDATE asset_checks
SET check_type = '<corrected_check_type>'::jsonb
WHERE id = '<check_id>'"
```

## Escalation

### When to Escalate
- Pass rate <90% sustained for 30+ minutes
- Blocking checks causing multiple workflow failures
- Unable to identify root cause in 30 minutes
- Data issue affecting critical business metrics

### Escalation Path
1. **Immediate**: Data quality owner
2. **15 minutes**: Data engineering lead
3. **30 minutes**: Platform lead

## Prevention

- Implement check staging/testing before production
- Add check versioning for rollback
- Monitor upstream data source SLAs
- Implement schema change detection
- Add data profiling for drift detection
- Set up alerting on upstream data freshness

## Check Types Reference

| Check Type | Purpose | Common Failures |
|------------|---------|-----------------|
| `not_null` | Ensure no nulls | Upstream ETL bugs |
| `unique` | Ensure uniqueness | Duplicate ingestion |
| `in_range` | Value bounds | Data corruption |
| `regex` | Pattern matching | Format changes |
| `row_count` | Volume checks | Missing data |
| `freshness` | Data timeliness | Pipeline delays |
| `accepted_values` | Enum validation | New values added |

## Related Runbooks

- [Error Budget Burn](error-budget-burn.md)
- [Stuck Executions](stuck-executions.md)
