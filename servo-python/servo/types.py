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

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for API serialization."""
        return {
            "name": self.name,
            "function_name": self.function_name,
            "module": self.module,
            "dependencies": self.dependencies,
            "description": self.description,
            "metadata": self.metadata.to_dict(),
            "group": self.group,
            "is_source": self.is_source,
        }


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
