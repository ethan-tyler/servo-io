# Data Quality Checks

Servo's data quality framework lets you define expectations about your data and automatically validate them during workflow execution. Checks can be configured to run after asset materialization and optionally block downstream processing on failure.

## Quick Start

```python
from servo import asset, check, expect

# Decorator-based checks
@asset(name="customers")
@check.not_null("customer_id", "email")
@check.unique("customer_id")
def customers():
    return {"data": [...]}

# Fluent API for complex validations
@asset(name="orders")
def orders():
    data = load_orders()

    expect(data).column("order_id").to_be_unique()
    expect(data).column("amount").to_be_between(0, 100000)
    expect(data).column("status").to_be_in(["pending", "completed", "cancelled"])

    return {"data": data}
```

## Check Types

### Column Checks

| Check | Description | Example |
|-------|-------------|---------|
| `not_null` | No null values in column | `@check.not_null("email")` |
| `unique` | All values are unique | `@check.unique("customer_id")` |
| `between` | Values within numeric range | `@check.between("age", min=0, max=150)` |
| `matches` | Values match regex pattern | `@check.matches("email", r".*@.*\\..*")` |
| `in_set` | Values from allowed set | `@check.in_set("status", ["active", "inactive"])` |

### Table Checks

| Check | Description | Example |
|-------|-------------|---------|
| `row_count` | Row count within range | `@check.row_count(min=1, max=1000000)` |
| `no_duplicate_rows` | No duplicate rows | `@check.no_duplicate_rows()` |
| `freshness` | Data not stale | `@check.freshness("updated_at", max_age_hours=24)` |

### Referential Checks

| Check | Description | Example |
|-------|-------------|---------|
| `referential_integrity` | Foreign key exists | `@check.referential_integrity("user_id", "users", "id")` |
| `schema_match` | Expected columns exist | `@check.schema_match(["id", "name", "email"])` |

### Custom Checks

```python
from servo import asset_check

@asset_check(asset="orders", name="valid_total", blocking=True)
def check_order_total(data):
    """Custom check: order total equals sum of line items."""
    for order in data["orders"]:
        expected = sum(item["price"] * item["qty"] for item in order["items"])
        if abs(order["total"] - expected) > 0.01:
            return {
                "passed": False,
                "failed_row_count": 1,
                "error_message": f"Order {order['id']} total mismatch"
            }
    return {"passed": True}
```

## Configuration Options

### Severity Levels

| Severity | Purpose |
|----------|---------|
| `error` | Critical data issues (default) |
| `warning` | Non-critical issues to review |
| `info` | Informational tracking |

```python
@check.not_null("email", severity="error")
@check.matches("phone", r"\\d{10}", severity="warning")
def customers():
    ...
```

### Blocking vs Non-Blocking

- **Blocking checks** (`blocking=True`): Halt workflow execution on failure
- **Non-blocking checks** (`blocking=False`): Record result but continue execution

```python
# Blocking: stops pipeline if email is null
@check.not_null("email", blocking=True)

# Non-blocking: logs warning but continues
@check.not_null("phone", blocking=False)
```

## CLI Commands

### List Checks for an Asset

```bash
servo quality list <asset_name> --tenant-id <tenant>
```

Example output:
```
Asset: customers

Quality Checks (3 total):
NAME                           TYPE                 SEVERITY   BLOCKING   ENABLED
----------------------------------------------------------------------------------
email_not_null                 not_null             error      yes        yes
customer_id_unique             unique               error      yes        yes
phone_format_check             matches              warning    no         yes
```

### View Check Results for an Execution

```bash
servo quality results <execution_id> --tenant-id <tenant>
```

Example output:
```
Execution: 550e8400-e29b-41d4-a716-446655440000

Summary: 2 passed, 1 failed, 0 errored, 0 skipped

CHECK                          OUTCOME    SEVERITY   BLOCKING   DURATION
--------------------------------------------------------------------------
email_not_null                 passed     error      yes        12ms
customer_id_unique             passed     error      yes        45ms
phone_format_check             failed     warning    no         8ms
    Failed rows: 15 / 1000 (1.5%)
```

### Show Check Details and History

```bash
servo quality show <check_id> --tenant-id <tenant> --limit 10
```

## Metrics

Quality check metrics are exposed via Prometheus:

```promql
# Check execution counts by outcome
servo_check_executions_total{tenant_id="...", outcome="passed|failed|error|skipped"}

# Check batch duration histogram
servo_check_duration_seconds{tenant_id="..."}
```

### Example Queries

```promql
# Pass rate (last 5 minutes)
sum(rate(servo_check_executions_total{outcome="passed"}[5m]))
/ sum(rate(servo_check_executions_total{outcome!="skipped"}[5m]))

# Failed checks by tenant
sum by (tenant_id) (rate(servo_check_executions_total{outcome="failed"}[5m]))
```

## Best Practices

### 1. Start with Non-Blocking Checks

When introducing checks on existing assets, start with `blocking=False` to observe failure patterns before enabling enforcement:

```python
@check.unique("order_id", blocking=False)  # Observe first
def orders():
    ...
```

### 2. Use Appropriate Severity

- Use `error` severity for data that must meet the condition
- Use `warning` for nice-to-have validations
- Use `info` for tracking metrics without alerting

### 3. Keep Check Names Descriptive

```python
# Good: clear what the check validates
@check.between("customer_age", 0, 150, name="valid_customer_age")

# Avoid: generic names
@check.between("customer_age", 0, 150)  # Auto-generated name
```

### 4. Combine Related Checks

Use the fluent API for related validations on the same data:

```python
@asset(name="orders")
def orders():
    data = load_orders()

    expect(data).column("order_id").to_be_unique()
    expect(data).column("order_id").to_not_be_null()
    expect(data).column("amount").to_be_between(0, 100000)

    return {"data": data}
```

## Troubleshooting

### Check Always Failing

1. Verify data format matches check expectations
2. Check column names are correct (case-sensitive)
3. Review sample failures in check results

### Blocking Check Halts All Workflows

1. Temporarily disable check: `UPDATE asset_checks SET enabled = false WHERE id = '...'`
2. Change to non-blocking: `UPDATE asset_checks SET blocking = false WHERE id = '...'`
3. Fix underlying data issue

### Check Results Not Appearing

1. Verify checks are defined with `@check.*` decorators
2. Ensure asset is registered with `@asset` decorator
3. Check that checks are enabled in the database

## Related Documentation

- [Runbook: Data Quality Failures](runbooks/data-quality-failures.md)
- [API Reference: Quality Module](api/quality.md)
