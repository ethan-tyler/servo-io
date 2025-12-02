"""Execution context for runtime partition information.

This module provides access to execution context and partition information
during Servo workflow execution. The context is populated from environment
variables set by the Servo worker.

Environment Variables:
    SERVO_EXECUTION_ID: UUID of the current execution
    SERVO_WORKFLOW_ID: UUID of the workflow being executed
    SERVO_TENANT_ID: Tenant identifier
    SERVO_ASSET_ID: UUID of the asset being executed
    SERVO_ASSET_NAME: Name of the asset
    SERVO_PARTITION_KEY: Current partition key (simple string)
    SERVO_PARTITION_CONTEXT: Full partition context as JSON

Example:
    from servo import get_partition_key, get_partition_date

    @asset(name="daily_sales", partition=DailyPartition(start_date="2023-01-01"))
    def daily_sales():
        # Get the partition being processed
        partition_date = get_partition_date()

        # Load data for this specific date
        df = load_sales_for_date(partition_date)
        return df
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from datetime import date, datetime
from typing import Any


@dataclass
class RuntimePartitionContext:
    """Runtime partition context provided by the worker.

    This is the partition information available during execution, populated
    from the SERVO_PARTITION_CONTEXT environment variable.
    """

    partition_key: str
    """Current partition key being processed (e.g., "2024-01-15")"""

    partition_type: str | None = None
    """Partition type ("daily", "hourly", "monthly", etc.)"""

    timezone: str | None = None
    """Timezone for time-based partitions"""

    format: str | None = None
    """Format string for parsing partition key"""

    dimensions: dict[str, str] = field(default_factory=dict)
    """Dimension values for multi-dimensional partitions"""

    upstream_partitions: dict[str, list[str]] = field(default_factory=dict)
    """Upstream asset -> partition keys mapping"""

    def as_date(self) -> date:
        """Parse partition_key as date (for daily partitions).

        Uses the format string if provided, otherwise tries ISO format.

        Returns:
            Parsed date object

        Raises:
            ValueError: If partition_key cannot be parsed as a date
        """
        if self.format:
            return datetime.strptime(self.partition_key, self.format).date()
        return datetime.strptime(self.partition_key, "%Y-%m-%d").date()

    def as_datetime(self) -> datetime:
        """Parse partition_key as datetime (for hourly/sub-daily partitions).

        Returns:
            Parsed datetime object

        Raises:
            ValueError: If partition_key cannot be parsed as a datetime
        """
        if self.format:
            return datetime.strptime(self.partition_key, self.format)
        return datetime.fromisoformat(self.partition_key)

    def get_dimension(self, name: str) -> str | None:
        """Get a dimension value for multi-dimensional partitions.

        Args:
            name: Dimension name

        Returns:
            Dimension value or None if not found
        """
        return self.dimensions.get(name)

    def get_upstream_partitions(self, asset_name: str) -> list[str] | None:
        """Get upstream partition keys for a specific asset.

        Args:
            asset_name: Name of the upstream asset

        Returns:
            List of partition keys or None if not found
        """
        return self.upstream_partitions.get(asset_name)

    @classmethod
    def from_json(cls, data: dict[str, Any]) -> RuntimePartitionContext:
        """Create from JSON dictionary.

        Args:
            data: Dictionary from JSON parsing

        Returns:
            RuntimePartitionContext instance
        """
        return cls(
            partition_key=data.get("partition_key", ""),
            partition_type=data.get("partition_type"),
            timezone=data.get("timezone"),
            format=data.get("format"),
            dimensions=data.get("dimensions", {}),
            upstream_partitions=data.get("upstream_partitions", {}),
        )

    @classmethod
    def from_env(cls) -> RuntimePartitionContext | None:
        """Load partition context from environment variables.

        Returns:
            RuntimePartitionContext if SERVO_PARTITION_CONTEXT is set, None otherwise
        """
        ctx_json = os.environ.get("SERVO_PARTITION_CONTEXT")
        if not ctx_json:
            return None

        try:
            data = json.loads(ctx_json)
            return cls.from_json(data)
        except (json.JSONDecodeError, TypeError):
            return None


@dataclass
class ExecutionContext:
    """Full execution context provided to asset compute functions.

    This context contains all runtime information available during execution,
    including execution IDs, tenant info, and partition context.
    """

    execution_id: str | None = None
    """UUID of the current execution"""

    workflow_id: str | None = None
    """UUID of the workflow being executed"""

    tenant_id: str | None = None
    """Tenant identifier"""

    asset_id: str | None = None
    """UUID of the asset being executed"""

    asset_name: str | None = None
    """Name of the asset"""

    partition: RuntimePartitionContext | None = None
    """Partition context for partitioned execution"""

    @classmethod
    def from_env(cls) -> ExecutionContext:
        """Load execution context from environment variables.

        Returns:
            ExecutionContext populated from environment
        """
        return cls(
            execution_id=os.environ.get("SERVO_EXECUTION_ID"),
            workflow_id=os.environ.get("SERVO_WORKFLOW_ID"),
            tenant_id=os.environ.get("SERVO_TENANT_ID"),
            asset_id=os.environ.get("SERVO_ASSET_ID"),
            asset_name=os.environ.get("SERVO_ASSET_NAME"),
            partition=RuntimePartitionContext.from_env(),
        )

    @property
    def is_partitioned(self) -> bool:
        """Check if this execution has partition context."""
        return self.partition is not None


# Module-level cached context
_cached_context: ExecutionContext | None = None


def get_context() -> ExecutionContext:
    """Get the current execution context.

    This function returns the execution context for the current Servo execution.
    The context is loaded from environment variables set by the Servo worker
    and cached for subsequent calls.

    Returns:
        ExecutionContext with execution details

    Example:
        from servo import get_context

        @asset(name="my_asset")
        def my_asset():
            ctx = get_context()
            print(f"Executing {ctx.asset_name} for tenant {ctx.tenant_id}")
    """
    global _cached_context
    if _cached_context is None:
        _cached_context = ExecutionContext.from_env()
    return _cached_context


def get_partition_key() -> str | None:
    """Get the current partition key.

    Convenience function to get the partition key from the execution context.

    Returns:
        The partition key string, or None if not running in a partitioned context

    Example:
        from servo import get_partition_key

        @asset(name="daily_data")
        def daily_data():
            key = get_partition_key()  # e.g., "2024-01-15"
            return load_data_for_date(key)
    """
    ctx = get_context()
    if ctx.partition:
        return ctx.partition.partition_key
    # Fall back to simple environment variable
    return os.environ.get("SERVO_PARTITION_KEY")


def get_partition_date() -> date | None:
    """Get the current partition key as a date.

    Convenience function for daily-partitioned assets.

    Returns:
        The partition key parsed as a date, or None if not partitioned

    Raises:
        ValueError: If partition key cannot be parsed as a date

    Example:
        from servo import get_partition_date

        @asset(name="daily_sales", partition=DailyPartition(start_date="2024-01-01"))
        def daily_sales():
            dt = get_partition_date()  # datetime.date(2024, 1, 15)
            return f"SELECT * FROM sales WHERE date = '{dt}'"
    """
    ctx = get_context()
    if ctx.partition:
        return ctx.partition.as_date()

    # Fall back to simple environment variable
    key = os.environ.get("SERVO_PARTITION_KEY")
    if key:
        return datetime.strptime(key, "%Y-%m-%d").date()
    return None


def get_partition_datetime() -> datetime | None:
    """Get the current partition key as a datetime.

    Convenience function for hourly or sub-daily partitioned assets.

    Returns:
        The partition key parsed as a datetime, or None if not partitioned

    Raises:
        ValueError: If partition key cannot be parsed as a datetime

    Example:
        from servo import get_partition_datetime

        @asset(name="hourly_metrics", partition=HourlyPartition(start_date="2024-01-01"))
        def hourly_metrics():
            dt = get_partition_datetime()  # datetime(2024, 1, 15, 10, 0, 0)
            return load_metrics_for_hour(dt)
    """
    ctx = get_context()
    if ctx.partition:
        return ctx.partition.as_datetime()

    # Fall back to simple environment variable
    key = os.environ.get("SERVO_PARTITION_KEY")
    if key:
        return datetime.fromisoformat(key)
    return None


def clear_context_cache() -> None:
    """Clear the cached execution context.

    This is primarily useful for testing to ensure fresh context is loaded.
    """
    global _cached_context
    _cached_context = None
