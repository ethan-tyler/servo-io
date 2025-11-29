# Servo Deployment Artifacts

This directory contains deployment configurations for monitoring and observability.

## SLO Specifications

**Location:** `slo/servo-slos.yaml`

Formal SLO definitions following Google SRE best practices. Includes:
- SLI definitions (what we measure)
- SLO targets (what we commit to)
- Error budget policy (actions when budget is consumed)

### SLO Summary

| SLO | Target | Error Budget (30d) | Window |
|-----|--------|-------------------|--------|
| HTTP Availability | 99.9% | 43.2 min | 30d |
| Workflow Availability | 99.9% | 43.2 min | 30d |
| HTTP Latency (P99 < 500ms) | 99% | 7.2 hours | 30d |
| Workflow Duration (P95 < 5min) | 95% | 36 hours | 30d |
| Data Quality Pass Rate | 95% | 8.4 hours | 7d |
| Database Latency (P99 < 100ms) | 99% | 7.2 hours | 30d |

## Alerting Rules

### Standard Alerts

**Location:** `alerts/servo-alerts.yaml`

Traditional threshold-based alerts for Servo infrastructure.

```yaml
# prometheus.yml
rule_files:
  - /path/to/servo-alerts.yaml
  - /path/to/servo-slo-alerts.yaml
```

### SLO Burn Rate Alerts

**Location:** `alerts/servo-slo-alerts.yaml`

Multi-window burn rate alerts based on Google SRE best practices:

| Alert Type | Burn Rate | Time to Exhaustion | Action |
|------------|-----------|-------------------|--------|
| Fast Burn | 14.4x | ~2 hours | Page immediately |
| Slow Burn | 3x | ~10 days | Create ticket |

### Alert Categories

| Category | Alerts | Description |
|----------|--------|-------------|
| SLO Burn Rate | `Servo*FastBurn`, `Servo*SlowBurn` | Error budget consumption |
| Data Quality | `ServoDataQualityFailures`, `ServoNoDataQualityChecks` | Check pass/fail rates |
| Rate Limiting | `ServoRateLimitRejections`, `ServoTenantRateLimited` | Rate limit exhaustion |
| Infrastructure | `ServoSlowDatabaseOperations`, `ServoHTTP5xxErrors`, `ServoNoTraffic` | System health |
| Capacity | `ServoHighTenantCardinality`, `ServoLargeWorkflows` | Resource usage |
| Workflow State | `ServoStateTransitionFailures`, `ServoStuckExecutions` | Execution lifecycle |

## Grafana Dashboards

**Location:** `dashboards/`

Import into Grafana via UI or provisioning:

```yaml
# /etc/grafana/provisioning/dashboards/servo.yaml
apiVersion: 1
providers:
  - name: 'servo'
    folder: 'Servo'
    type: file
    options:
      path: /path/to/deploy/dashboards
```

### Available Dashboards

| Dashboard | UID | Description |
|-----------|-----|-------------|
| [slo-overview.json](dashboards/slo-overview.json) | `servo-slo-overview` | Error budgets, burn rates, SLO compliance |
| [workflow-execution.json](dashboards/workflow-execution.json) | `servo-workflow-execution` | Workflow execution counts, durations, success rates by tenant |
| [data-quality.json](dashboards/data-quality.json) | `servo-data-quality` | Data quality check outcomes, pass rates, performance |
| [infrastructure.json](dashboards/infrastructure.json) | `servo-infrastructure` | HTTP requests, DB operations, rate limiting |

### Dashboard Features

**SLO Overview Dashboard:**
- Error budget remaining (gauge per SLO)
- Burn rate trend (1-hour window)
- Time to budget exhaustion
- SLI actual values vs targets

**Workflow Execution Dashboard:**
- Execution counts (24h total, by status)
- Success rate gauge with SLO threshold
- Duration percentiles (P50, P90, P99)
- Executions by tenant breakdown
- State transition monitoring
- Asset count distribution

**Data Quality Dashboard:**
- Check counts and pass rates
- Outcome breakdown (passed/failed/skipped)
- Pass rate trend with 95% threshold line
- Duration percentiles
- Distribution histogram

**Infrastructure Dashboard:**
- HTTP request rate and success rate
- Latency percentiles by endpoint
- Status code distribution (2xx/4xx/5xx)
- Database operation latency
- Rate limiter status and rejections

### Template Variables

All dashboards support:
- `datasource`: Prometheus data source selector
- `tenant_id`: Filter by tenant (where applicable)

## GCP Cloud Monitoring

For GCP deployments, these Prometheus rules can be converted to Cloud Monitoring alerting policies. The metrics are exposed at `/metrics` endpoint in Prometheus text format.

### Example Cloud Run Configuration

```yaml
# Cloud Run service with metrics
spec:
  template:
    metadata:
      annotations:
        run.googleapis.com/cpu-throttling: "false"
    spec:
      containers:
        - name: servo-worker
          ports:
            - containerPort: 8080
          env:
            - name: METRICS_ENABLED
              value: "true"
```

## Metrics Reference

See [servo-worker/src/metrics.rs](../servo-worker/src/metrics.rs) for complete metric definitions.

### Key Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `servo_workflow_executions_total` | Counter | status, tenant_id | Total executions |
| `servo_workflow_duration_seconds` | Histogram | status, tenant_id | Execution duration |
| `servo_check_executions_total` | Counter | tenant_id, outcome | Check executions |
| `servo_check_duration_seconds` | Histogram | tenant_id | Check batch duration |
| `servo_http_requests_total` | Counter | endpoint, status_code | HTTP requests |
| `servo_http_request_duration_seconds` | Histogram | endpoint | HTTP latency |
| `servo_rate_limit_rejections_total` | Counter | limiter_type, tenant_id | Rate limit rejections |
| `servo_db_operation_duration_seconds` | Histogram | operation | DB operation latency |

### MTTR-Focused Metrics

These metrics help reduce Mean Time to Recovery (MTTR):

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `servo_circuit_breaker_state` | Gauge | service | Current state (0=closed, 1=half-open, 2=open) |
| `servo_circuit_breaker_transitions_total` | Counter | service, from_state, to_state | State transitions |
| `servo_circuit_breaker_recovery_seconds` | Histogram | service | Time to recover from open state |
| `servo_execution_queue_depth` | Gauge | state | Pending/running execution counts |
| `servo_execution_queue_wait_seconds` | Histogram | tenant_id | Queue wait time |
| `servo_error_detection_seconds` | Histogram | error_type | Time to detect errors |
| `servo_dependency_latency_seconds` | Histogram | dependency | External dependency latency |
| `servo_dependency_health` | Gauge | dependency | Dependency health (0/1) |

## Incident Response

See [docs/runbooks/](../docs/runbooks/) for actionable runbooks for each alert type.

## Local Development

Start local observability stack with Docker Compose:

```bash
# Start Jaeger and PostgreSQL
docker-compose -f docker-compose.dev.yml up -d

# Start with Prometheus and Grafana
docker-compose -f docker-compose.dev.yml --profile metrics up -d

# Configure servo-worker for local tracing
export SERVO_TRACE_EXPORTER=jaeger
export SERVO_JAEGER_ENDPOINT=http://localhost:4317
export SERVO_TRACE_ENABLED=true
```

Access:

- Jaeger UI: http://localhost:16686
- Prometheus: http://localhost:9090 (with --profile metrics)
- Grafana: http://localhost:3000 (with --profile metrics)
