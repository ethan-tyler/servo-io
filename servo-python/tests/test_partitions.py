"""Tests for partition definitions and mappings."""

from __future__ import annotations

from datetime import datetime

import pytest

from servo.partitions import (
    AllUpstreamMapping,
    DailyPartition,
    DimensionMapping,
    DynamicPartition,
    HourlyPartition,
    IdentityMapping,
    MonthlyPartition,
    MultiPartition,
    PartitionContext,
    StaticPartition,
    TimeWindowMapping,
    WeeklyPartition,
    parse_partition_definition,
    parse_partition_mapping,
)


class TestDailyPartition:
    """Tests for DailyPartition."""

    def test_get_partition_keys_bounded(self) -> None:
        """Test getting keys for a bounded date range."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-05",
        )
        keys = partition.get_partition_keys()
        assert keys == [
            "2023-01-01",
            "2023-01-02",
            "2023-01-03",
            "2023-01-04",
            "2023-01-05",
        ]

    def test_get_partition_keys_with_filter(self) -> None:
        """Test filtering keys by date range."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-10",
        )
        keys = partition.get_partition_keys(
            start=datetime(2023, 1, 3),
            end=datetime(2023, 1, 5),
        )
        assert keys == ["2023-01-03", "2023-01-04", "2023-01-05"]

    def test_validate_partition_key_valid(self) -> None:
        """Test validating a valid partition key."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-31",
        )
        assert partition.validate_partition_key("2023-01-15") is True

    def test_validate_partition_key_invalid_format(self) -> None:
        """Test validating an invalid format."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-31",
        )
        assert partition.validate_partition_key("invalid") is False

    def test_validate_partition_key_out_of_range(self) -> None:
        """Test validating a key outside the range."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-31",
        )
        assert partition.validate_partition_key("2022-12-31") is False
        assert partition.validate_partition_key("2023-02-01") is False

    def test_to_dict(self) -> None:
        """Test serialization to dictionary."""
        partition = DailyPartition(
            start_date="2023-01-01",
            end_date="2023-01-31",
        )
        result = partition.to_dict()
        assert result["type"] == "time_daily"
        assert result["start_date"] == "2023-01-01"
        assert result["end_date"] == "2023-01-31"

    def test_partition_type(self) -> None:
        """Test partition type property."""
        partition = DailyPartition(start_date="2023-01-01")
        assert partition.partition_type == "time_daily"


class TestWeeklyPartition:
    """Tests for WeeklyPartition."""

    def test_get_partition_keys(self) -> None:
        """Test getting weekly partition keys."""
        partition = WeeklyPartition(
            start_date="2023-01-02",  # Monday
            end_date="2023-01-16",
        )
        keys = partition.get_partition_keys()
        assert keys == ["2023-01-02", "2023-01-09", "2023-01-16"]

    def test_partition_type(self) -> None:
        """Test partition type property."""
        partition = WeeklyPartition(start_date="2023-01-02")
        assert partition.partition_type == "time_weekly"


class TestMonthlyPartition:
    """Tests for MonthlyPartition."""

    def test_get_partition_keys(self) -> None:
        """Test getting monthly partition keys."""
        partition = MonthlyPartition(
            start_date="2023-01-01",
            end_date="2023-03-31",
        )
        keys = partition.get_partition_keys()
        assert keys == ["2023-01-01", "2023-02-01", "2023-03-01"]

    def test_partition_type(self) -> None:
        """Test partition type property."""
        partition = MonthlyPartition(start_date="2023-01-01")
        assert partition.partition_type == "time_monthly"


class TestHourlyPartition:
    """Tests for HourlyPartition."""

    def test_get_partition_keys_limited(self) -> None:
        """Test getting hourly keys with filter to limit results."""
        partition = HourlyPartition(
            start_date="2023-01-01",
            end_date="2023-01-01",
        )
        keys = partition.get_partition_keys(
            start=datetime(2023, 1, 1, 0),
            end=datetime(2023, 1, 1, 3),
        )
        assert keys == [
            "2023-01-01T00",
            "2023-01-01T01",
            "2023-01-01T02",
            "2023-01-01T03",
        ]

    def test_partition_type(self) -> None:
        """Test partition type property."""
        partition = HourlyPartition(start_date="2023-01-01")
        assert partition.partition_type == "time_hourly"


class TestStaticPartition:
    """Tests for StaticPartition."""

    def test_get_partition_keys(self) -> None:
        """Test getting static partition keys."""
        partition = StaticPartition(keys=["us", "eu", "apac"])
        keys = partition.get_partition_keys()
        assert keys == ["us", "eu", "apac"]

    def test_get_partition_keys_ignores_date_filter(self) -> None:
        """Test that date filters are ignored for static partitions."""
        partition = StaticPartition(keys=["us", "eu", "apac"])
        keys = partition.get_partition_keys(
            start=datetime(2023, 1, 1),
            end=datetime(2023, 1, 31),
        )
        assert keys == ["us", "eu", "apac"]

    def test_validate_partition_key_valid(self) -> None:
        """Test validating a valid key."""
        partition = StaticPartition(keys=["us", "eu", "apac"])
        assert partition.validate_partition_key("us") is True
        assert partition.validate_partition_key("eu") is True

    def test_validate_partition_key_invalid(self) -> None:
        """Test validating an invalid key."""
        partition = StaticPartition(keys=["us", "eu", "apac"])
        assert partition.validate_partition_key("unknown") is False

    def test_to_dict(self) -> None:
        """Test serialization."""
        partition = StaticPartition(keys=["us", "eu"])
        result = partition.to_dict()
        assert result["type"] == "static"
        assert result["keys"] == ["us", "eu"]

    def test_partition_type(self) -> None:
        """Test partition type property."""
        partition = StaticPartition(keys=["a"])
        assert partition.partition_type == "static"


class TestDynamicPartition:
    """Tests for DynamicPartition."""

    def test_add_partition_key(self) -> None:
        """Test adding partition keys."""
        partition = DynamicPartition(name="customers")
        partition.add_partition_key("cust_001")
        partition.add_partition_key("cust_002")
        assert partition.get_partition_keys() == ["cust_001", "cust_002"]

    def test_add_partition_key_no_duplicates(self) -> None:
        """Test that duplicate keys are not added."""
        partition = DynamicPartition(name="customers")
        partition.add_partition_key("cust_001")
        partition.add_partition_key("cust_001")
        assert partition.get_partition_keys() == ["cust_001"]

    def test_remove_partition_key(self) -> None:
        """Test removing partition keys."""
        partition = DynamicPartition(name="customers")
        partition.add_partition_key("cust_001")
        partition.add_partition_key("cust_002")
        partition.remove_partition_key("cust_001")
        assert partition.get_partition_keys() == ["cust_002"]

    def test_validate_partition_key(self) -> None:
        """Test key validation."""
        partition = DynamicPartition(name="customers")
        partition.add_partition_key("cust_001")
        assert partition.validate_partition_key("cust_001") is True
        assert partition.validate_partition_key("cust_002") is False

    def test_to_dict(self) -> None:
        """Test serialization."""
        partition = DynamicPartition(name="customers")
        partition.add_partition_key("cust_001")
        result = partition.to_dict()
        assert result["type"] == "dynamic"
        assert result["name"] == "customers"
        assert result["keys"] == ["cust_001"]


class TestMultiPartition:
    """Tests for MultiPartition."""

    def test_get_partition_keys_cartesian_product(self) -> None:
        """Test getting multi-dimensional keys."""
        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-02"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        keys = partition.get_multi_partition_keys()
        assert len(keys) == 4
        assert {"date": "2023-01-01", "region": "us"} in keys
        assert {"date": "2023-01-01", "region": "eu"} in keys
        assert {"date": "2023-01-02", "region": "us"} in keys
        assert {"date": "2023-01-02", "region": "eu"} in keys

    def test_validate_partition_key_valid(self) -> None:
        """Test validating a valid multi-dimensional key."""
        import json

        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        key = json.dumps({"date": "2023-01-15", "region": "us"}, sort_keys=True)
        assert partition.validate_partition_key(key) is True

    def test_validate_partition_key_invalid_dimension(self) -> None:
        """Test validating with invalid dimension value."""
        import json

        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        key = json.dumps({"date": "2023-01-15", "region": "apac"}, sort_keys=True)
        assert partition.validate_partition_key(key) is False

    def test_validate_partition_key_missing_dimension(self) -> None:
        """Test validating with missing dimension."""
        import json

        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        key = json.dumps({"date": "2023-01-15"}, sort_keys=True)
        assert partition.validate_partition_key(key) is False

    def test_to_dict(self) -> None:
        """Test serialization."""
        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        result = partition.to_dict()
        assert result["type"] == "multi"
        assert "dimensions" in result
        assert "date" in result["dimensions"]
        assert "region" in result["dimensions"]


class TestIdentityMapping:
    """Tests for IdentityMapping."""

    def test_get_upstream_partitions(self) -> None:
        """Test identity mapping returns same key."""
        mapping = IdentityMapping()
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        keys = mapping.get_upstream_partitions("2023-01-15", upstream_def)
        assert keys == ["2023-01-15"]

    def test_to_dict(self) -> None:
        """Test serialization."""
        mapping = IdentityMapping()
        assert mapping.to_dict() == {"type": "identity"}


class TestTimeWindowMapping:
    """Tests for TimeWindowMapping."""

    def test_get_upstream_partitions_week(self) -> None:
        """Test getting a week of upstream partitions."""
        mapping = TimeWindowMapping(start_offset=-6, end_offset=0)
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        keys = mapping.get_upstream_partitions("2023-01-07", upstream_def)
        assert keys == [
            "2023-01-01",
            "2023-01-02",
            "2023-01-03",
            "2023-01-04",
            "2023-01-05",
            "2023-01-06",
            "2023-01-07",
        ]

    def test_get_upstream_partitions_filters_invalid(self) -> None:
        """Test that invalid partition keys are filtered."""
        mapping = TimeWindowMapping(start_offset=-3, end_offset=0)
        upstream_def = DailyPartition(start_date="2023-01-02", end_date="2023-01-31")
        # 2023-01-03 with -3 offset tries to get 2022-12-31, 2023-01-01 which are invalid
        keys = mapping.get_upstream_partitions("2023-01-03", upstream_def)
        assert keys == ["2023-01-02", "2023-01-03"]

    def test_get_upstream_partitions_invalid_format(self) -> None:
        """Test with invalid date format."""
        mapping = TimeWindowMapping(start_offset=-1, end_offset=0)
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        keys = mapping.get_upstream_partitions("invalid-date", upstream_def)
        assert keys == []

    def test_to_dict(self) -> None:
        """Test serialization."""
        mapping = TimeWindowMapping(start_offset=-6, end_offset=0)
        result = mapping.to_dict()
        assert result["type"] == "time_window"
        assert result["start_offset"] == -6
        assert result["end_offset"] == 0


class TestAllUpstreamMapping:
    """Tests for AllUpstreamMapping."""

    def test_get_upstream_partitions(self) -> None:
        """Test getting all upstream partitions."""
        mapping = AllUpstreamMapping()
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-03")
        keys = mapping.get_upstream_partitions("any-key", upstream_def)
        assert keys == ["2023-01-01", "2023-01-02", "2023-01-03"]

    def test_to_dict(self) -> None:
        """Test serialization."""
        mapping = AllUpstreamMapping()
        assert mapping.to_dict() == {"type": "all_upstream"}


class TestDimensionMapping:
    """Tests for DimensionMapping."""

    def test_get_upstream_partitions_single_dimension(self) -> None:
        """Test mapping to single dimension."""
        import json

        mapping = DimensionMapping(dimension_map={"date": "date"})
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        downstream_key = json.dumps({"date": "2023-01-15", "region": "us"})
        keys = mapping.get_upstream_partitions(downstream_key, upstream_def)
        assert keys == ["2023-01-15"]

    def test_get_upstream_partitions_invalid_json(self) -> None:
        """Test with invalid JSON."""
        mapping = DimensionMapping(dimension_map={"date": "date"})
        upstream_def = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        keys = mapping.get_upstream_partitions("not-json", upstream_def)
        assert keys == ["not-json"]

    def test_to_dict(self) -> None:
        """Test serialization."""
        mapping = DimensionMapping(dimension_map={"date": "date"})
        result = mapping.to_dict()
        assert result["type"] == "dimension"
        assert result["dimension_map"] == {"date": "date"}


class TestPartitionContext:
    """Tests for PartitionContext."""

    def test_single_partition_key(self) -> None:
        """Test context with single partition key."""
        partition = DailyPartition(start_date="2023-01-01", end_date="2023-01-31")
        context = PartitionContext(
            partition_key="2023-01-15",
            partition_definition=partition,
        )
        assert context.partition_key == "2023-01-15"
        assert context.partition_keys == {"default": "2023-01-15"}
        assert context.is_multi_partition is False

    def test_multi_partition_key(self) -> None:
        """Test context with multi-dimensional partition key."""
        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        multi_keys = {"date": "2023-01-15", "region": "us"}
        context = PartitionContext.from_multi_keys(multi_keys, partition)
        assert context.partition_keys == multi_keys
        assert context.is_multi_partition is True
        assert context.get_dimension_key("date") == "2023-01-15"
        assert context.get_dimension_key("region") == "us"
        assert context.get_dimension_key("unknown") is None

    def test_partition_keys_parses_json(self) -> None:
        """Test that partition_keys parses JSON string."""
        import json

        partition = MultiPartition(
            date=DailyPartition(start_date="2023-01-01", end_date="2023-01-31"),
            region=StaticPartition(keys=["us", "eu"]),
        )
        key_dict = {"date": "2023-01-15", "region": "us"}
        context = PartitionContext(
            partition_key=json.dumps(key_dict, sort_keys=True),
            partition_definition=partition,
        )
        assert context.partition_keys == key_dict


class TestParsePartitionDefinition:
    """Tests for parse_partition_definition."""

    def test_parse_daily(self) -> None:
        """Test parsing daily partition."""
        data = {
            "type": "daily",
            "start_date": "2023-01-01",
            "end_date": "2023-01-31",
        }
        partition = parse_partition_definition(data)
        assert isinstance(partition, DailyPartition)
        assert partition.start_date == "2023-01-01"

    def test_parse_static(self) -> None:
        """Test parsing static partition."""
        data = {
            "type": "static",
            "keys": ["a", "b", "c"],
        }
        partition = parse_partition_definition(data)
        assert isinstance(partition, StaticPartition)
        assert partition.keys == ["a", "b", "c"]

    def test_parse_dynamic(self) -> None:
        """Test parsing dynamic partition."""
        data = {
            "type": "dynamic",
            "name": "customers",
            "keys": ["cust_001", "cust_002"],
        }
        partition = parse_partition_definition(data)
        assert isinstance(partition, DynamicPartition)
        assert partition.name == "customers"
        assert partition.get_partition_keys() == ["cust_001", "cust_002"]

    def test_parse_multi(self) -> None:
        """Test parsing multi partition."""
        data = {
            "type": "multi",
            "dimensions": {
                "date": {"type": "daily", "start_date": "2023-01-01"},
                "region": {"type": "static", "keys": ["us", "eu"]},
            },
        }
        partition = parse_partition_definition(data)
        assert isinstance(partition, MultiPartition)
        assert "date" in partition.dimensions
        assert "region" in partition.dimensions

    def test_parse_unknown_type(self) -> None:
        """Test parsing unknown partition type."""
        data = {"type": "unknown"}
        with pytest.raises(ValueError, match="Unknown partition type"):
            parse_partition_definition(data)


class TestParsePartitionMapping:
    """Tests for parse_partition_mapping."""

    def test_parse_identity(self) -> None:
        """Test parsing identity mapping."""
        data = {"type": "identity"}
        mapping = parse_partition_mapping(data)
        assert isinstance(mapping, IdentityMapping)

    def test_parse_time_window(self) -> None:
        """Test parsing time window mapping."""
        data = {
            "type": "time_window",
            "start_offset": -6,
            "end_offset": 0,
        }
        mapping = parse_partition_mapping(data)
        assert isinstance(mapping, TimeWindowMapping)
        assert mapping.start_offset == -6
        assert mapping.end_offset == 0

    def test_parse_all_upstream(self) -> None:
        """Test parsing all upstream mapping."""
        data = {"type": "all_upstream"}
        mapping = parse_partition_mapping(data)
        assert isinstance(mapping, AllUpstreamMapping)

    def test_parse_dimension(self) -> None:
        """Test parsing dimension mapping."""
        data = {
            "type": "dimension",
            "dimension_map": {"date": "date"},
        }
        mapping = parse_partition_mapping(data)
        assert isinstance(mapping, DimensionMapping)
        assert mapping.dimension_map == {"date": "date"}

    def test_parse_unknown_type(self) -> None:
        """Test parsing unknown mapping type."""
        data = {"type": "unknown"}
        with pytest.raises(ValueError, match="Unknown partition mapping type"):
            parse_partition_mapping(data)
