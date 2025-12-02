"""Tests for servo.context module."""

import json
import os
from datetime import date, datetime
from unittest import mock

from servo.context import (
    ExecutionContext,
    RuntimePartitionContext,
    clear_context_cache,
    get_context,
    get_partition_date,
    get_partition_datetime,
    get_partition_key,
)


class TestRuntimePartitionContext:
    """Tests for RuntimePartitionContext."""

    def test_basic_creation(self):
        """Test basic context creation."""
        ctx = RuntimePartitionContext(partition_key="2024-01-15")
        assert ctx.partition_key == "2024-01-15"
        assert ctx.partition_type is None
        assert ctx.timezone is None
        assert ctx.dimensions == {}
        assert ctx.upstream_partitions == {}

    def test_full_creation(self):
        """Test context with all fields."""
        ctx = RuntimePartitionContext(
            partition_key="2024-01-15",
            partition_type="daily",
            timezone="UTC",
            format="%Y-%m-%d",
            dimensions={"date": "2024-01-15", "region": "us-west"},
            upstream_partitions={"source": ["2024-01-14", "2024-01-15"]},
        )
        assert ctx.partition_key == "2024-01-15"
        assert ctx.partition_type == "daily"
        assert ctx.timezone == "UTC"
        assert ctx.format == "%Y-%m-%d"
        assert ctx.dimensions == {"date": "2024-01-15", "region": "us-west"}
        assert ctx.upstream_partitions == {"source": ["2024-01-14", "2024-01-15"]}

    def test_as_date(self):
        """Test parsing partition key as date."""
        ctx = RuntimePartitionContext(partition_key="2024-01-15")
        assert ctx.as_date() == date(2024, 1, 15)

    def test_as_date_with_format(self):
        """Test parsing with custom format."""
        ctx = RuntimePartitionContext(partition_key="15/01/2024", format="%d/%m/%Y")
        assert ctx.as_date() == date(2024, 1, 15)

    def test_as_datetime(self):
        """Test parsing partition key as datetime."""
        ctx = RuntimePartitionContext(partition_key="2024-01-15T10:30:00")
        dt = ctx.as_datetime()
        assert dt == datetime(2024, 1, 15, 10, 30, 0)

    def test_get_dimension(self):
        """Test getting dimension values."""
        ctx = RuntimePartitionContext(
            partition_key="2024-01-15|us-west",
            dimensions={"date": "2024-01-15", "region": "us-west"},
        )
        assert ctx.get_dimension("date") == "2024-01-15"
        assert ctx.get_dimension("region") == "us-west"
        assert ctx.get_dimension("nonexistent") is None

    def test_get_upstream_partitions(self):
        """Test getting upstream partition keys."""
        ctx = RuntimePartitionContext(
            partition_key="2024-01-15",
            upstream_partitions={"source": ["2024-01-14", "2024-01-15"]},
        )
        assert ctx.get_upstream_partitions("source") == ["2024-01-14", "2024-01-15"]
        assert ctx.get_upstream_partitions("nonexistent") is None

    def test_from_json(self):
        """Test creating from JSON dictionary."""
        data = {
            "partition_key": "2024-01-15",
            "partition_type": "daily",
            "timezone": "America/New_York",
            "dimensions": {"date": "2024-01-15"},
            "upstream_partitions": {"source": ["2024-01-14"]},
        }
        ctx = RuntimePartitionContext.from_json(data)
        assert ctx.partition_key == "2024-01-15"
        assert ctx.partition_type == "daily"
        assert ctx.timezone == "America/New_York"
        assert ctx.get_dimension("date") == "2024-01-15"

    def test_from_env_with_context(self):
        """Test loading from environment variable."""
        context_json = json.dumps(
            {
                "partition_key": "2024-01-15",
                "partition_type": "daily",
                "timezone": "UTC",
            }
        )
        with mock.patch.dict(os.environ, {"SERVO_PARTITION_CONTEXT": context_json}):
            ctx = RuntimePartitionContext.from_env()
            assert ctx is not None
            assert ctx.partition_key == "2024-01-15"
            assert ctx.partition_type == "daily"

    def test_from_env_without_context(self):
        """Test loading when environment variable is not set."""
        with mock.patch.dict(os.environ, {}, clear=True):
            # Make sure SERVO_PARTITION_CONTEXT is not set
            os.environ.pop("SERVO_PARTITION_CONTEXT", None)
            ctx = RuntimePartitionContext.from_env()
            assert ctx is None

    def test_from_env_with_invalid_json(self):
        """Test loading with invalid JSON."""
        with mock.patch.dict(os.environ, {"SERVO_PARTITION_CONTEXT": "not valid json"}):
            ctx = RuntimePartitionContext.from_env()
            assert ctx is None


class TestExecutionContext:
    """Tests for ExecutionContext."""

    def test_from_env(self):
        """Test loading full context from environment."""
        env = {
            "SERVO_EXECUTION_ID": "exec-123",
            "SERVO_WORKFLOW_ID": "wf-456",
            "SERVO_TENANT_ID": "tenant-abc",
            "SERVO_ASSET_ID": "asset-789",
            "SERVO_ASSET_NAME": "daily_sales",
            "SERVO_PARTITION_CONTEXT": json.dumps(
                {
                    "partition_key": "2024-01-15",
                    "partition_type": "daily",
                }
            ),
        }
        with mock.patch.dict(os.environ, env, clear=False):
            ctx = ExecutionContext.from_env()
            assert ctx.execution_id == "exec-123"
            assert ctx.workflow_id == "wf-456"
            assert ctx.tenant_id == "tenant-abc"
            assert ctx.asset_id == "asset-789"
            assert ctx.asset_name == "daily_sales"
            assert ctx.partition is not None
            assert ctx.partition.partition_key == "2024-01-15"

    def test_is_partitioned(self):
        """Test is_partitioned property."""
        ctx_partitioned = ExecutionContext(
            partition=RuntimePartitionContext(partition_key="2024-01-15")
        )
        assert ctx_partitioned.is_partitioned is True

        ctx_not_partitioned = ExecutionContext()
        assert ctx_not_partitioned.is_partitioned is False


class TestConvenienceFunctions:
    """Tests for convenience functions."""

    def setup_method(self):
        """Clear cache before each test."""
        clear_context_cache()

    def teardown_method(self):
        """Clear cache after each test."""
        clear_context_cache()

    def test_get_context_cached(self):
        """Test that context is cached."""
        env = {
            "SERVO_EXECUTION_ID": "exec-123",
            "SERVO_PARTITION_CONTEXT": json.dumps({"partition_key": "2024-01-15"}),
        }
        with mock.patch.dict(os.environ, env, clear=False):
            ctx1 = get_context()
            ctx2 = get_context()
            assert ctx1 is ctx2  # Same object (cached)

    def test_get_partition_key_from_context(self):
        """Test getting partition key from full context."""
        env = {
            "SERVO_PARTITION_CONTEXT": json.dumps(
                {
                    "partition_key": "2024-01-15",
                    "partition_type": "daily",
                }
            ),
        }
        with mock.patch.dict(os.environ, env, clear=False):
            key = get_partition_key()
            assert key == "2024-01-15"

    def test_get_partition_key_fallback(self):
        """Test fallback to simple environment variable."""
        clear_context_cache()
        env = {"SERVO_PARTITION_KEY": "2024-01-20"}
        with mock.patch.dict(os.environ, env, clear=True):
            key = get_partition_key()
            assert key == "2024-01-20"

    def test_get_partition_key_none(self):
        """Test when no partition key is available."""
        clear_context_cache()
        with mock.patch.dict(os.environ, {}, clear=True):
            key = get_partition_key()
            assert key is None

    def test_get_partition_date_from_context(self):
        """Test getting partition date from context."""
        env = {
            "SERVO_PARTITION_CONTEXT": json.dumps(
                {
                    "partition_key": "2024-01-15",
                    "partition_type": "daily",
                }
            ),
        }
        with mock.patch.dict(os.environ, env, clear=False):
            dt = get_partition_date()
            assert dt == date(2024, 1, 15)

    def test_get_partition_date_fallback(self):
        """Test fallback to simple environment variable."""
        clear_context_cache()
        env = {"SERVO_PARTITION_KEY": "2024-01-20"}
        with mock.patch.dict(os.environ, env, clear=True):
            dt = get_partition_date()
            assert dt == date(2024, 1, 20)

    def test_get_partition_datetime(self):
        """Test getting partition datetime."""
        env = {
            "SERVO_PARTITION_CONTEXT": json.dumps(
                {
                    "partition_key": "2024-01-15T10:30:00",
                    "partition_type": "hourly",
                }
            ),
        }
        with mock.patch.dict(os.environ, env, clear=False):
            dt = get_partition_datetime()
            assert dt == datetime(2024, 1, 15, 10, 30, 0)

    def test_clear_context_cache(self):
        """Test that cache clearing works."""
        env1 = {
            "SERVO_PARTITION_CONTEXT": json.dumps({"partition_key": "2024-01-15"}),
        }
        with mock.patch.dict(os.environ, env1, clear=False):
            ctx1 = get_context()
            assert ctx1.partition.partition_key == "2024-01-15"

        clear_context_cache()

        env2 = {
            "SERVO_PARTITION_CONTEXT": json.dumps({"partition_key": "2024-02-20"}),
        }
        with mock.patch.dict(os.environ, env2, clear=False):
            ctx2 = get_context()
            assert ctx2.partition.partition_key == "2024-02-20"
            assert ctx1 is not ctx2  # Different objects


class TestIntegration:
    """Integration tests for the full context flow."""

    def setup_method(self):
        clear_context_cache()

    def teardown_method(self):
        clear_context_cache()

    def test_full_partition_context_flow(self):
        """Test the complete flow from JSON to partition key."""
        # Simulate what the Rust worker would set
        partition_context = {
            "partition_key": "2024-01-15",
            "partition_type": "daily",
            "timezone": "UTC",
            "dimensions": {"date": "2024-01-15"},
            "upstream_partitions": {"raw_data": ["2024-01-14", "2024-01-15"]},
        }
        env = {
            "SERVO_EXECUTION_ID": "550e8400-e29b-41d4-a716-446655440000",
            "SERVO_WORKFLOW_ID": "550e8400-e29b-41d4-a716-446655440001",
            "SERVO_TENANT_ID": "acme-corp",
            "SERVO_ASSET_ID": "550e8400-e29b-41d4-a716-446655440002",
            "SERVO_ASSET_NAME": "daily_sales",
            "SERVO_PARTITION_KEY": "2024-01-15",  # Simple fallback
            "SERVO_PARTITION_CONTEXT": json.dumps(partition_context),
        }

        with mock.patch.dict(os.environ, env, clear=False):
            # Get full context
            ctx = get_context()
            assert ctx.execution_id == "550e8400-e29b-41d4-a716-446655440000"
            assert ctx.tenant_id == "acme-corp"
            assert ctx.asset_name == "daily_sales"
            assert ctx.is_partitioned

            # Get partition key
            key = get_partition_key()
            assert key == "2024-01-15"

            # Get as date
            dt = get_partition_date()
            assert dt == date(2024, 1, 15)

            # Access partition context details
            assert ctx.partition.partition_type == "daily"
            assert ctx.partition.timezone == "UTC"
            assert ctx.partition.get_dimension("date") == "2024-01-15"
            assert ctx.partition.get_upstream_partitions("raw_data") == [
                "2024-01-14",
                "2024-01-15",
            ]
