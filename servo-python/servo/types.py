"""Type definitions for Servo SDK."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Any, TypeVar

# Generic type for dataframes (pandas, polars, etc.)
DataFrame = TypeVar("DataFrame")

# Generic type for database tables
Table = TypeVar("Table")


# ========== Type Mappings for Schema Validation ==========

# Map pandas dtypes to canonical type names
PANDAS_TYPE_MAP: dict[str, str] = {
    "object": "string",
    "string": "string",
    "int64": "int64",
    "int32": "int32",
    "Int64": "int64",  # nullable int
    "Int32": "int32",  # nullable int
    "float64": "float64",
    "float32": "float32",
    "Float64": "float64",  # nullable float
    "datetime64[ns]": "datetime",
    "datetime64[ns, UTC]": "datetime",
    "bool": "bool",
    "boolean": "bool",  # nullable bool
    "category": "category",
}

# Map polars dtypes to canonical type names
POLARS_TYPE_MAP: dict[str, str] = {
    "Utf8": "string",
    "String": "string",
    "Int64": "int64",
    "Int32": "int32",
    "Int16": "int16",
    "Int8": "int8",
    "UInt64": "uint64",
    "UInt32": "uint32",
    "Float64": "float64",
    "Float32": "float32",
    "Datetime": "datetime",
    "Date": "date",
    "Boolean": "bool",
    "Categorical": "category",
}


@dataclass
class SchemaColumn:
    """Schema column definition for type-aware schema matching.

    Used to define expected column structure including name, data type,
    and nullability for schema validation checks.
    """

    name: str
    data_type: str = "any"  # "string", "int64", "float64", "datetime", "bool", etc.
    nullable: bool = True

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary representation."""
        return {
            "name": self.name,
            "data_type": self.data_type,
            "nullable": self.nullable,
        }


class AssetStatus(str, Enum):
    """Status of an asset materialization."""

    PENDING = "pending"
    RUNNING = "running"
    SUCCESS = "success"
    FAILED = "failed"
    SKIPPED = "skipped"
    UPSTREAM_FAILED = "upstream_failed"


class MaterializationTrigger(str, Enum):
    """What triggered the materialization."""

    SCHEDULED = "scheduled"
    MANUAL = "manual"
    UPSTREAM = "upstream"
    BACKFILL = "backfill"


@dataclass
class AssetMetadata:
    """Metadata associated with an asset."""

    owner: str | None = None
    team: str | None = None
    tags: list[str] = field(default_factory=list)
    description: str | None = None
    custom: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary representation."""
        result: dict[str, Any] = {}
        if self.owner is not None:
            result["owner"] = self.owner
        if self.team is not None:
            result["team"] = self.team
        if self.tags:
            result["tags"] = self.tags
        if self.description is not None:
            result["description"] = self.description
        if self.custom:
            result.update(self.custom)
        return result


@dataclass
class AssetOutput:
    """Output from an asset execution."""

    value: Any
    metadata: dict[str, Any] = field(default_factory=dict)
    row_count: int | None = None
    byte_size: int | None = None

    @classmethod
    def from_dataframe(cls, df: Any) -> AssetOutput:
        """Create output from a pandas/polars DataFrame."""
        row_count = None
        byte_size = None

        # Try pandas
        if hasattr(df, "shape") and hasattr(df, "memory_usage"):
            row_count = df.shape[0]
            try:
                byte_size = int(df.memory_usage(deep=True).sum())
            except Exception:
                pass
        # Try polars
        elif hasattr(df, "shape") and hasattr(df, "estimated_size"):
            row_count = df.shape[0]
            try:
                byte_size = df.estimated_size()
            except Exception:
                pass

        return cls(value=df, row_count=row_count, byte_size=byte_size)


@dataclass
class Materialization:
    """Record of an asset materialization."""

    asset_name: str
    run_id: str
    status: AssetStatus
    started_at: datetime
    completed_at: datetime | None = None
    trigger: MaterializationTrigger = MaterializationTrigger.MANUAL
    partition_key: str | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    error: str | None = None

    @property
    def duration_seconds(self) -> float | None:
        """Get duration in seconds if completed."""
        if self.completed_at is None:
            return None
        return (self.completed_at - self.started_at).total_seconds()


@dataclass
class AssetDefinition:
    """Definition of a registered asset."""

    name: str
    function_name: str
    module: str
    dependencies: list[str]
    description: str | None
    metadata: AssetMetadata
    group: str | None = None
    is_source: bool = False
    partition: Any | None = None  # PartitionDefinition (avoid circular import)
    partition_mappings: dict[str, Any] | None = None  # {dep_name: PartitionMapping}

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for API serialization."""
        result = {
            "name": self.name,
            "function_name": self.function_name,
            "module": self.module,
            "dependencies": self.dependencies,
            "description": self.description,
            "metadata": self.metadata.to_dict(),
            "group": self.group,
            "is_source": self.is_source,
        }
        if self.partition is not None:
            result["partition"] = self.partition.to_dict()
        if self.partition_mappings is not None:
            result["partition_mappings"] = {
                name: mapping.to_dict() for name, mapping in self.partition_mappings.items()
            }
        return result


@dataclass
class WorkflowDefinition:
    """Definition of a registered workflow."""

    name: str
    function_name: str
    module: str
    schedule: str | None
    assets: list[str]
    description: str | None
    executor: str | None = None
    timeout_seconds: int = 3600
    retries: int = 0

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for API serialization."""
        return {
            "name": self.name,
            "function_name": self.function_name,
            "module": self.module,
            "schedule": self.schedule,
            "assets": self.assets,
            "description": self.description,
            "executor": self.executor,
            "timeout_seconds": self.timeout_seconds,
            "retries": self.retries,
        }


# ========== Data Quality Types ==========


class CheckSeverity(str, Enum):
    """Severity level for data quality checks."""

    ERROR = "error"  # Blocks downstream execution if blocking=True
    WARNING = "warning"  # Logged + alerts, but workflow continues
    INFO = "info"  # Informational only, logged but no alerts


class CheckStatus(str, Enum):
    """Result status of a check execution."""

    PASSED = "passed"
    FAILED = "failed"
    SKIPPED = "skipped"
    ERROR = "error"  # Check itself errored (not a data quality failure)


@dataclass
class CheckResult:
    """Result of a data quality check execution."""

    check_name: str
    status: CheckStatus
    severity: CheckSeverity
    asset_name: str
    blocking: bool = True
    message: str | None = None
    rows_checked: int | None = None
    rows_failed: int | None = None
    details: dict[str, Any] = field(default_factory=dict)
    executed_at: datetime = field(default_factory=datetime.utcnow)
    duration_ms: float | None = None

    @property
    def passed(self) -> bool:
        """Check if the result passed."""
        return self.status == CheckStatus.PASSED

    @property
    def should_block(self) -> bool:
        """Check if this result should block downstream execution."""
        return (
            self.blocking
            and self.status in (CheckStatus.FAILED, CheckStatus.ERROR)
            and self.severity == CheckSeverity.ERROR
        )

    @property
    def failure_rate(self) -> float | None:
        """Calculate failure rate as a percentage if row counts available."""
        if self.rows_checked and self.rows_checked > 0:
            return ((self.rows_failed or 0) / self.rows_checked) * 100.0
        return None

    @classmethod
    def success(
        cls,
        check_name: str,
        asset_name: str,
        *,
        rows_checked: int | None = None,
        message: str | None = None,
        details: dict[str, Any] | None = None,
        duration_ms: float | None = None,
    ) -> CheckResult:
        """Factory for a successful check result."""
        return cls(
            check_name=check_name,
            status=CheckStatus.PASSED,
            severity=CheckSeverity.ERROR,
            asset_name=asset_name,
            message=message,
            rows_checked=rows_checked,
            rows_failed=0,
            details=details or {},
            duration_ms=duration_ms,
        )

    @classmethod
    def failure(
        cls,
        check_name: str,
        asset_name: str,
        message: str,
        *,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        rows_checked: int | None = None,
        rows_failed: int | None = None,
        details: dict[str, Any] | None = None,
        duration_ms: float | None = None,
    ) -> CheckResult:
        """Factory for a failed check result."""
        return cls(
            check_name=check_name,
            status=CheckStatus.FAILED,
            severity=severity,
            asset_name=asset_name,
            blocking=blocking,
            message=message,
            rows_checked=rows_checked,
            rows_failed=rows_failed,
            details=details or {},
            duration_ms=duration_ms,
        )

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for API serialization."""
        result: dict[str, Any] = {
            "check_name": self.check_name,
            "status": self.status.value,
            "severity": self.severity.value,
            "asset_name": self.asset_name,
            "blocking": self.blocking,
            "executed_at": self.executed_at.isoformat(),
        }
        if self.message is not None:
            result["message"] = self.message
        if self.rows_checked is not None:
            result["rows_checked"] = self.rows_checked
        if self.rows_failed is not None:
            result["rows_failed"] = self.rows_failed
        if self.details:
            result["details"] = self.details
        if self.duration_ms is not None:
            result["duration_ms"] = self.duration_ms
        return result


@dataclass
class CheckDefinition:
    """Definition of a registered check."""

    name: str
    asset_name: str
    check_type: str  # "not_null", "unique", "in_range", "regex", etc.
    severity: CheckSeverity
    blocking: bool
    function_name: str
    module: str
    description: str | None = None
    parameters: dict[str, Any] = field(default_factory=dict)
    column: str | None = None  # For column-specific checks

    def to_rust_check_type(self) -> dict[str, Any]:
        """Convert to Rust CheckType JSON schema.

        This generates JSON that matches the Rust backend's CheckType enum
        which uses #[serde(tag = "type")] for tagged serialization.
        """
        if self.check_type == "not_null":
            columns = self.parameters.get("columns", [self.column] if self.column else [])
            return {"type": "not_null", "columns": columns}
        elif self.check_type == "unique":
            columns = self.parameters.get("columns", [self.column] if self.column else [])
            return {"type": "unique", "columns": columns}
        elif self.check_type == "in_range":
            return {
                "type": "in_range",
                "column": self.column,
                "min": self.parameters.get("min"),
                "max": self.parameters.get("max"),
            }
        elif self.check_type == "regex":
            return {
                "type": "regex",
                "column": self.column,
                "pattern": self.parameters.get("pattern"),
            }
        elif self.check_type == "row_count":
            return {
                "type": "row_count",
                "min": self.parameters.get("min"),
                "max": self.parameters.get("max"),
            }
        elif self.check_type == "accepted_values":
            return {
                "type": "accepted_values",
                "column": self.column,
                "values": self.parameters.get("values", []),
            }
        elif self.check_type == "no_duplicate_rows":
            return {
                "type": "no_duplicate_rows",
                "columns": self.parameters.get("columns"),
            }
        elif self.check_type == "freshness":
            return {
                "type": "freshness",
                "timestamp_column": self.column,
                "max_age_seconds": self.parameters.get("max_age_seconds"),
            }
        elif self.check_type == "referential_integrity":
            return {
                "type": "referential_integrity",
                "column": self.column,
                "reference_table": self.parameters.get("reference_table"),
                "reference_column": self.parameters.get("reference_column"),
            }
        elif self.check_type == "schema_match":
            return {
                "type": "schema_match",
                "expected_columns": self.parameters.get("expected_columns", []),
                "allow_extra_columns": self.parameters.get("allow_extra_columns", True),
            }
        else:
            # Custom check - pass through
            return {"type": "custom", "name": self.name, "description": self.description or ""}

    def to_api_payload(self) -> dict[str, Any]:
        """Convert to API payload for backend sync."""
        return {
            "name": self.name,
            "description": self.description,
            "check_type": self.to_rust_check_type(),
            "severity": self.severity.value,
            "blocking": self.blocking,
        }

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for API serialization."""
        result: dict[str, Any] = {
            "name": self.name,
            "asset_name": self.asset_name,
            "check_type": self.check_type,
            "severity": self.severity.value,
            "blocking": self.blocking,
            "function_name": self.function_name,
            "module": self.module,
        }
        if self.description is not None:
            result["description"] = self.description
        if self.parameters:
            result["parameters"] = self.parameters
        if self.column is not None:
            result["column"] = self.column
        return result
