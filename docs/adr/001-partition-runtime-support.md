# ADR-001: Partition Runtime Support

## Status

Accepted

## Context

Servo supports multiple partition types for organizing data by time or custom dimensions:

1. **Daily partitions** (`YYYY-MM-DD`) - Most common, fully supported
2. **Hourly partitions** (`YYYY-MM-DD-HH`) - Fully supported
3. **Custom partitions** (arbitrary keys) - SDK-only support
4. **Multi-dimensional partitions** - Future consideration
5. **Mapped partitions** (dynamic discovery) - Future consideration

The Python SDK allows defining assets with any partition configuration. However, the runtime/backend currently enforces and schedules only date-based partitions (daily and hourly).

## Decision

### Current Implementation (Phase 1)

The runtime enforces date-range partitions with the following constraints:

- Partition keys must match `YYYY-MM-DD` (daily) or `YYYY-MM-DD-HH` (hourly) format
- Backfill jobs accept `--start` and `--end` date parameters
- Partition validation in CLI rejects malformed dates
- Partition generation creates contiguous date ranges

### Deferred Features (Future Phases)

1. **Custom partition scheduling**: Allow arbitrary partition keys without date validation
2. **Multi-dimensional partitions**: Partition by multiple attributes (e.g., date + region)
3. **Mapped partitions**: Dynamic partition discovery from external sources
4. **Partition dependencies**: Cross-partition dependency tracking

## Consequences

### Positive

- Simple, predictable partition model for initial release
- Date-based partitions cover 90%+ of use cases
- Clear mental model for operators
- Efficient date-range queries and scheduling

### Negative

- Users with non-date partitions must manage partitions manually
- SDK partition definitions may not match runtime behavior
- Documentation must clearly state limitations

### Neutral

- Future partition features can be added incrementally
- No breaking changes required for expansion

## Implementation Notes

### Current Validation (CLI)

```rust
// servo-cli/src/commands/backfill.rs
fn validate_partition_key(key: &str) -> Result<()> {
    // Accepts: YYYY-MM-DD, YYYY-MM-DD-HH, or custom keys
    // Custom keys bypass date validation but may not work with range backfills
}
```

### Date Range Generation

```rust
// servo-cli/src/commands/backfill.rs
fn generate_date_range(start: &str, end: &str) -> Result<Vec<String>> {
    // Generates contiguous YYYY-MM-DD keys from start to end
}
```

### Future Extension Points

1. Add `partition_type` field to `AssetModel` (daily, hourly, custom, mapped)
2. Implement partition discovery for mapped partitions
3. Add multi-dimensional partition key format (`date=2024-01-01/region=us-east`)
4. Support partition-level dependencies in lineage graph

## References

- [Dagster Partitions](https://docs.dagster.io/concepts/partitions-schedules-sensors/partitions)
- [dbt Incremental Models](https://docs.getdbt.com/docs/build/incremental-models)
- [Apache Airflow Data-aware Scheduling](https://airflow.apache.org/docs/apache-airflow/stable/authoring-and-scheduling/datasets.html)
