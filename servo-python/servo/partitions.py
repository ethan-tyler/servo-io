"""Advanced partitioning support for Servo assets.

This module provides Dagster-compatible partition definitions for time-based,
static, dynamic, and multi-dimensional partitioning.

.. note::
    **SDK-Only Feature**: Partitioning is currently available for asset definition
    and metadata purposes only. Full runtime/scheduler support for partition-aware
    execution is planned for a future release. Currently, partition definitions are
    serialized and stored but not yet used for automated partition scheduling.

Example usage:

    from servo.partitions import DailyPartition, StaticPartition, MultiPartition

    @asset(
        name="regional_daily_sales",
        partition=MultiPartition(
            date=DailyPartition(start_date="2023-01-01"),
            region=StaticPartition(["us", "eu", "apac"]),
        ),
    )
    def regional_daily_sales(context):
        date = context.partition_keys["date"]
        region = context.partition_keys["region"]
        return load_sales(date, region)
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Iterator
from dataclasses import dataclass, field
from datetime import date, datetime, timedelta
from enum import Enum
from typing import Any


class PartitionGranularity(str, Enum):
    """Granularity for time-based partitions."""

    HOURLY = "hourly"
    DAILY = "daily"
    WEEKLY = "weekly"
    MONTHLY = "monthly"


class PartitionDefinition(ABC):
    """Base class for all partition definitions."""

    @abstractmethod
    def get_partition_keys(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> list[str]:
        """Get all partition keys, optionally filtered by date range."""

    @abstractmethod
    def validate_partition_key(self, key: str) -> bool:
        """Check if a partition key is valid for this definition."""

    @abstractmethod
    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary for API/storage."""

    @property
    @abstractmethod
    def partition_type(self) -> str:
        """Return the partition type identifier."""


@dataclass
class TimePartitionDefinition(PartitionDefinition):
    """Base class for time-based partition definitions."""

    start_date: str  # ISO format: YYYY-MM-DD
    end_date: str | None = None  # None means unbounded (up to today)
    timezone: str = "UTC"
    fmt: str = "%Y-%m-%d"  # Key format
    _granularity: PartitionGranularity = field(
        default=PartitionGranularity.DAILY, repr=False
    )

    def __post_init__(self) -> None:
        """Validate inputs."""
        # Validate start_date
        self._parse_date(self.start_date)
        if self.end_date:
            self._parse_date(self.end_date)

    def _parse_date(self, date_str: str) -> date:
        """Parse a date string."""
        return datetime.strptime(date_str, "%Y-%m-%d").date()

    def _get_end_date(self) -> date:
        """Get the effective end date (today if unbounded)."""
        if self.end_date:
            return self._parse_date(self.end_date)
        return datetime.now().date()

    def _iter_dates(self) -> Iterator[date]:
        """Iterate over all partition dates."""
        current = self._parse_date(self.start_date)
        end = self._get_end_date()
        delta = self._get_delta()

        while current <= end:
            yield current
            current = self._next_date(current, delta)

    def _get_delta(self) -> timedelta:
        """Get the time delta for this granularity."""
        if self._granularity == PartitionGranularity.HOURLY:
            return timedelta(hours=1)
        elif self._granularity == PartitionGranularity.DAILY:
            return timedelta(days=1)
        elif self._granularity == PartitionGranularity.WEEKLY:
            return timedelta(weeks=1)
        else:  # MONTHLY handled specially
            return timedelta(days=1)

    def _next_date(self, current: date, delta: timedelta) -> date:
        """Get the next partition date."""
        if self._granularity == PartitionGranularity.MONTHLY:
            # Move to first of next month
            if current.month == 12:
                return date(current.year + 1, 1, 1)
            return date(current.year, current.month + 1, 1)
        return current + delta

    def get_partition_keys(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> list[str]:
        """Get all partition keys in the date range."""
        keys: list[str] = []
        start_filter = start.date() if start else None
        end_filter = end.date() if end else None

        for d in self._iter_dates():
            if start_filter and d < start_filter:
                continue
            if end_filter and d > end_filter:
                continue
            keys.append(d.strftime(self.fmt))

        return keys

    def validate_partition_key(self, key: str) -> bool:
        """Check if a partition key is valid."""
        try:
            parsed = datetime.strptime(key, self.fmt).date()
            start = self._parse_date(self.start_date)
            end = self._get_end_date()
            return start <= parsed <= end
        except ValueError:
            return False

    @property
    def partition_type(self) -> str:
        """Return the partition type."""
        return f"time_{self._granularity.value}"

    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary."""
        return {
            "type": self.partition_type,
            "start_date": self.start_date,
            "end_date": self.end_date,
            "timezone": self.timezone,
            "format": self.fmt,
            "granularity": self._granularity.value,
        }


@dataclass
class DailyPartition(TimePartitionDefinition):
    """Daily time-based partition.

    Example:
        partition = DailyPartition(start_date="2023-01-01")
        keys = partition.get_partition_keys()
        # ["2023-01-01", "2023-01-02", ...]
    """

    def __post_init__(self) -> None:
        self._granularity = PartitionGranularity.DAILY
        super().__post_init__()


@dataclass
class HourlyPartition(TimePartitionDefinition):
    """Hourly time-based partition.

    Example:
        partition = HourlyPartition(start_date="2023-01-01")
        keys = partition.get_partition_keys()
        # ["2023-01-01T00", "2023-01-01T01", ...]
    """

    fmt: str = "%Y-%m-%dT%H"

    def __post_init__(self) -> None:
        self._granularity = PartitionGranularity.HOURLY
        super().__post_init__()

    def _iter_dates(self) -> Iterator[date]:
        """Iterate over all partition hours (returns datetime for hourly)."""
        current_dt = datetime.strptime(self.start_date, "%Y-%m-%d")
        end_dt = datetime.combine(self._get_end_date(), datetime.max.time())

        while current_dt <= end_dt:
            yield current_dt.date()
            current_dt += timedelta(hours=1)

    def get_partition_keys(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> list[str]:
        """Get all partition keys (hourly)."""
        keys: list[str] = []
        current_dt = datetime.strptime(self.start_date, "%Y-%m-%d")
        end_dt = datetime.combine(self._get_end_date(), datetime.max.time())

        start_filter = start if start else None
        end_filter = end if end else None

        while current_dt <= end_dt:
            if start_filter and current_dt < start_filter:
                current_dt += timedelta(hours=1)
                continue
            if end_filter and current_dt > end_filter:
                break
            keys.append(current_dt.strftime(self.fmt))
            current_dt += timedelta(hours=1)

        return keys


@dataclass
class WeeklyPartition(TimePartitionDefinition):
    """Weekly time-based partition (starts Monday).

    Example:
        partition = WeeklyPartition(start_date="2023-01-02")  # Monday
        keys = partition.get_partition_keys()
        # ["2023-01-02", "2023-01-09", ...]
    """

    def __post_init__(self) -> None:
        self._granularity = PartitionGranularity.WEEKLY
        super().__post_init__()


@dataclass
class MonthlyPartition(TimePartitionDefinition):
    """Monthly time-based partition (first of month).

    Example:
        partition = MonthlyPartition(start_date="2023-01-01")
        keys = partition.get_partition_keys()
        # ["2023-01-01", "2023-02-01", ...]
    """

    fmt: str = "%Y-%m-01"

    def __post_init__(self) -> None:
        self._granularity = PartitionGranularity.MONTHLY
        super().__post_init__()


@dataclass
class StaticPartition(PartitionDefinition):
    """Static partition with a fixed list of keys.

    Example:
        partition = StaticPartition(["us", "eu", "apac"])
        keys = partition.get_partition_keys()
        # ["us", "eu", "apac"]
    """

    keys: list[str]

    def get_partition_keys(
        self,
        start: datetime | None = None,  # noqa: ARG002
        end: datetime | None = None,  # noqa: ARG002
    ) -> list[str]:
        """Get all static partition keys (start/end ignored)."""
        return self.keys.copy()

    def validate_partition_key(self, key: str) -> bool:
        """Check if the key is in the static list."""
        return key in self.keys

    @property
    def partition_type(self) -> str:
        return "static"

    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary."""
        return {
            "type": "static",
            "keys": self.keys,
        }


@dataclass
class DynamicPartition(PartitionDefinition):
    """Dynamic partition with runtime-discovered keys.

    Keys are registered at runtime and stored in the backend.

    Example:
        partition = DynamicPartition(name="customer_ids")
        # Keys are discovered/added at runtime via API
    """

    name: str
    _keys: list[str] = field(default_factory=list, repr=False)

    def add_partition_key(self, key: str) -> None:
        """Add a new partition key (typically synced with backend)."""
        if key not in self._keys:
            self._keys.append(key)

    def remove_partition_key(self, key: str) -> None:
        """Remove a partition key."""
        if key in self._keys:
            self._keys.remove(key)

    def get_partition_keys(
        self,
        start: datetime | None = None,  # noqa: ARG002
        end: datetime | None = None,  # noqa: ARG002
    ) -> list[str]:
        """Get current dynamic partition keys (start/end ignored)."""
        return self._keys.copy()

    def validate_partition_key(self, key: str) -> bool:
        """Check if the key exists in dynamic partition."""
        return key in self._keys

    @property
    def partition_type(self) -> str:
        return "dynamic"

    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary."""
        return {
            "type": "dynamic",
            "name": self.name,
            "keys": self._keys,
        }


@dataclass
class MultiPartition(PartitionDefinition):
    """Multi-dimensional partition combining multiple partition dimensions.

    Creates a Cartesian product of all dimension keys.

    Example:
        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-03"),
            region=StaticPartition(["us", "eu"]),
        )
        keys = partition.get_partition_keys()
        # [
        #     {"date": "2023-01-01", "region": "us"},
        #     {"date": "2023-01-01", "region": "eu"},
        #     {"date": "2023-01-02", "region": "us"},
        #     ...
        # ]
    """

    dimensions: dict[str, PartitionDefinition] = field(default_factory=dict)

    def __init__(self, **dimensions: PartitionDefinition) -> None:
        """Initialize with named dimensions."""
        self.dimensions = dimensions

    def get_partition_keys(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> list[str]:
        """Get all multi-dimensional partition keys as JSON strings."""
        import json

        multi_keys = self._get_multi_keys(start, end)
        return [json.dumps(mk, sort_keys=True) for mk in multi_keys]

    def get_multi_partition_keys(
        self,
        start: datetime | None = None,
        end: datetime | None = None,
    ) -> list[dict[str, str]]:
        """Get partition keys as list of dictionaries."""
        return self._get_multi_keys(start, end)

    def _get_multi_keys(
        self,
        start: datetime | None,
        end: datetime | None,
    ) -> list[dict[str, str]]:
        """Generate Cartesian product of all dimensions."""
        if not self.dimensions:
            return []

        # Get keys for each dimension
        dim_keys: dict[str, list[str]] = {}
        for dim_name, dim_def in self.dimensions.items():
            dim_keys[dim_name] = dim_def.get_partition_keys(start, end)

        # Generate Cartesian product
        dim_names = list(dim_keys.keys())
        result: list[dict[str, str]] = []
        self._cartesian_product(dim_names, dim_keys, 0, {}, result)
        return result

    def _cartesian_product(
        self,
        dim_names: list[str],
        dim_keys: dict[str, list[str]],
        index: int,
        current: dict[str, str],
        result: list[dict[str, str]],
    ) -> None:
        """Recursively generate Cartesian product."""
        if index == len(dim_names):
            result.append(current.copy())
            return

        dim_name = dim_names[index]
        for key in dim_keys[dim_name]:
            current[dim_name] = key
            self._cartesian_product(dim_names, dim_keys, index + 1, current, result)

    def validate_partition_key(self, key: str) -> bool:
        """Validate a multi-dimensional key (JSON string)."""
        import json

        try:
            parsed = json.loads(key)
            if not isinstance(parsed, dict):
                return False
            if set(parsed.keys()) != set(self.dimensions.keys()):
                return False
            for dim_name, dim_key in parsed.items():
                if not self.dimensions[dim_name].validate_partition_key(dim_key):
                    return False
            return True
        except (json.JSONDecodeError, KeyError):
            return False

    @property
    def partition_type(self) -> str:
        return "multi"

    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary."""
        return {
            "type": "multi",
            "dimensions": {
                name: dim.to_dict() for name, dim in self.dimensions.items()
            },
        }


# ========== Partition Mappings for Dependencies ==========


class PartitionMapping(ABC):
    """Base class for partition mappings between assets."""

    @abstractmethod
    def get_upstream_partitions(
        self,
        downstream_key: str,
        upstream_def: PartitionDefinition,
    ) -> list[str]:
        """Get upstream partition keys for a downstream partition."""

    @abstractmethod
    def to_dict(self) -> dict[str, Any]:
        """Serialize to dictionary."""


@dataclass
class IdentityMapping(PartitionMapping):
    """1:1 mapping between upstream and downstream partitions.

    The upstream partition key is the same as the downstream key.

    Example:
        # daily_metrics depends on daily_raw with same partition key
        dependencies={"daily_raw": IdentityMapping()}
    """

    def get_upstream_partitions(
        self,
        downstream_key: str,
        upstream_def: PartitionDefinition,  # noqa: ARG002
    ) -> list[str]:
        """Return the same partition key."""
        return [downstream_key]

    def to_dict(self) -> dict[str, Any]:
        return {"type": "identity"}


@dataclass
class TimeWindowMapping(PartitionMapping):
    """Time-window mapping for aggregations across time partitions.

    Maps a downstream partition to multiple upstream partitions using
    day offsets.

    Example:
        # weekly_summary depends on 7 days of daily_data
        dependencies={"daily_data": TimeWindowMapping(start_offset=-6, end_offset=0)}

        # If downstream is "2023-01-07" (weekly), upstream partitions are:
        # ["2023-01-01", "2023-01-02", ..., "2023-01-07"]
    """

    start_offset: int = 0  # Days before
    end_offset: int = 0  # Days after

    def get_upstream_partitions(
        self,
        downstream_key: str,
        upstream_def: PartitionDefinition,
    ) -> list[str]:
        """Get all upstream partitions in the time window."""
        # Parse downstream key as date
        try:
            base_date = datetime.strptime(downstream_key, "%Y-%m-%d").date()
        except ValueError:
            # Try other formats or return empty
            return []

        # Generate all dates in the window
        keys: list[str] = []
        for offset in range(self.start_offset, self.end_offset + 1):
            target_date = base_date + timedelta(days=offset)
            key = target_date.strftime("%Y-%m-%d")
            if upstream_def.validate_partition_key(key):
                keys.append(key)

        return keys

    def to_dict(self) -> dict[str, Any]:
        return {
            "type": "time_window",
            "start_offset": self.start_offset,
            "end_offset": self.end_offset,
        }


@dataclass
class AllUpstreamMapping(PartitionMapping):
    """Maps to all partitions of the upstream asset.

    Useful for aggregations that need all historical data.

    Example:
        # full_summary depends on all daily_data partitions
        dependencies={"daily_data": AllUpstreamMapping()}
    """

    def get_upstream_partitions(
        self,
        downstream_key: str,  # noqa: ARG002
        upstream_def: PartitionDefinition,
    ) -> list[str]:
        """Return all upstream partition keys."""
        return upstream_def.get_partition_keys()

    def to_dict(self) -> dict[str, Any]:
        return {"type": "all_upstream"}


@dataclass
class DimensionMapping(PartitionMapping):
    """Maps dimensions between multi-dimensional partitions.

    Allows remapping dimension names or filtering dimensions.

    Example:
        # downstream has (date, region), upstream has (date) only
        dependencies={"upstream": DimensionMapping(dimension_map={"date": "date"})}
    """

    dimension_map: dict[str, str] = field(default_factory=dict)

    def get_upstream_partitions(
        self,
        downstream_key: str,
        upstream_def: PartitionDefinition,  # noqa: ARG002
    ) -> list[str]:
        """Map dimensions from downstream to upstream."""
        import json

        try:
            parsed = json.loads(downstream_key)
            if not isinstance(parsed, dict):
                return [downstream_key]

            # Remap dimensions
            upstream_key: dict[str, str] = {}
            for up_dim, down_dim in self.dimension_map.items():
                if down_dim in parsed:
                    upstream_key[up_dim] = parsed[down_dim]

            # If single dimension, return as string
            if len(upstream_key) == 1:
                return list(upstream_key.values())

            return [json.dumps(upstream_key, sort_keys=True)]
        except json.JSONDecodeError:
            return [downstream_key]

    def to_dict(self) -> dict[str, Any]:
        return {
            "type": "dimension",
            "dimension_map": self.dimension_map,
        }


# ========== Partition Context ==========


@dataclass
class PartitionContext:
    """Context for partition-aware asset execution.

    Provides access to current partition keys and helper methods.

    Example:
        @asset(name="daily_sales", partition=DailyPartition(start_date="2023-01-01"))
        def daily_sales(context: PartitionContext):
            date = context.partition_key
            return load_sales_for_date(date)
    """

    partition_key: str
    partition_definition: PartitionDefinition
    _multi_keys: dict[str, str] | None = None

    @property
    def partition_keys(self) -> dict[str, str]:
        """Get multi-dimensional partition keys.

        For multi-partitions, returns a dict like {"date": "2023-01-01", "region": "us"}.
        For single-dimension partitions, returns {"default": partition_key}.
        """
        if self._multi_keys is not None:
            return self._multi_keys

        # Try to parse as JSON (multi-partition)
        import json

        try:
            parsed = json.loads(self.partition_key)
            if isinstance(parsed, dict):
                return parsed
        except json.JSONDecodeError:
            pass

        return {"default": self.partition_key}

    @property
    def is_multi_partition(self) -> bool:
        """Check if this is a multi-dimensional partition."""
        return isinstance(self.partition_definition, MultiPartition)

    def get_dimension_key(self, dimension: str) -> str | None:
        """Get the key for a specific dimension."""
        keys = self.partition_keys
        return keys.get(dimension)

    @classmethod
    def from_multi_keys(
        cls,
        multi_keys: dict[str, str],
        partition_definition: PartitionDefinition,
    ) -> PartitionContext:
        """Create context from multi-dimensional keys."""
        import json

        return cls(
            partition_key=json.dumps(multi_keys, sort_keys=True),
            partition_definition=partition_definition,
            _multi_keys=multi_keys,
        )


# ========== Utility Functions ==========


def parse_partition_definition(data: dict[str, Any]) -> PartitionDefinition:
    """Parse a partition definition from a dictionary.

    Useful for deserializing from API responses or storage.
    """
    partition_type = data.get("type", "")

    if partition_type == "static":
        return StaticPartition(keys=data.get("keys", []))

    if partition_type == "dynamic":
        partition = DynamicPartition(name=data.get("name", ""))
        for key in data.get("keys", []):
            partition.add_partition_key(key)
        return partition

    if partition_type in ("time_daily", "daily"):
        return DailyPartition(
            start_date=data.get("start_date", ""),
            end_date=data.get("end_date"),
            timezone=data.get("timezone", "UTC"),
        )

    if partition_type in ("time_hourly", "hourly"):
        return HourlyPartition(
            start_date=data.get("start_date", ""),
            end_date=data.get("end_date"),
            timezone=data.get("timezone", "UTC"),
        )

    if partition_type in ("time_weekly", "weekly"):
        return WeeklyPartition(
            start_date=data.get("start_date", ""),
            end_date=data.get("end_date"),
            timezone=data.get("timezone", "UTC"),
        )

    if partition_type in ("time_monthly", "monthly"):
        return MonthlyPartition(
            start_date=data.get("start_date", ""),
            end_date=data.get("end_date"),
            timezone=data.get("timezone", "UTC"),
        )

    if partition_type == "multi":
        dimensions = {}
        for dim_name, dim_data in data.get("dimensions", {}).items():
            dimensions[dim_name] = parse_partition_definition(dim_data)
        return MultiPartition(**dimensions)

    msg = f"Unknown partition type: {partition_type}"
    raise ValueError(msg)


def parse_partition_mapping(data: dict[str, Any]) -> PartitionMapping:
    """Parse a partition mapping from a dictionary."""
    mapping_type = data.get("type", "")

    if mapping_type == "identity":
        return IdentityMapping()

    if mapping_type == "time_window":
        return TimeWindowMapping(
            start_offset=data.get("start_offset", 0),
            end_offset=data.get("end_offset", 0),
        )

    if mapping_type == "all_upstream":
        return AllUpstreamMapping()

    if mapping_type == "dimension":
        return DimensionMapping(
            dimension_map=data.get("dimension_map", {}),
        )

    msg = f"Unknown partition mapping type: {mapping_type}"
    raise ValueError(msg)
