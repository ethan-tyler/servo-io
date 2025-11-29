"""Tests for the @asset decorator."""

import pytest

from servo.asset import (
    Asset,
    asset,
    clear_asset_registry,
    get_asset,
    get_asset_registry,
    validate_dependencies,
    validate_dependencies_strict,
)
from servo.exceptions import AssetExecutionError, ServoValidationError
from servo.types import AssetOutput


@pytest.fixture(autouse=True)
def clean_registry():
    """Clear registry before each test."""
    clear_asset_registry()
    yield
    clear_asset_registry()


class TestAssetDecorator:
    """Tests for @asset decorator."""

    def test_basic_asset_no_args(self):
        """Test @asset without parentheses."""

        @asset
        def my_asset():
            return {"data": [1, 2, 3]}

        assert isinstance(my_asset, Asset)
        assert my_asset.name == "my_asset"
        assert my_asset.dependencies == []
        assert my_asset() == {"data": [1, 2, 3]}

    def test_basic_asset_with_args(self):
        """Test @asset with arguments."""

        @asset(name="custom_name", dependencies=["upstream"])
        def my_asset():
            return {"data": [1, 2, 3]}

        assert my_asset.name == "custom_name"
        assert my_asset.dependencies == ["upstream"]

    def test_asset_with_metadata(self):
        """Test asset with metadata."""

        @asset(
            name="orders",
            metadata={
                "owner": "data-team",
                "team": "analytics",
                "tags": ["production", "critical"],
                "custom_field": "value",
            },
        )
        def orders():
            return []

        assert orders.metadata.owner == "data-team"
        assert orders.metadata.team == "analytics"
        assert orders.metadata.tags == ["production", "critical"]
        assert orders.metadata.custom == {"custom_field": "value"}

    def test_asset_with_description(self):
        """Test asset description from docstring."""

        @asset
        def documented_asset():
            """This is the asset description."""
            return []

        assert documented_asset.description == "This is the asset description."

    def test_asset_description_override(self):
        """Test explicit description overrides docstring."""

        @asset(description="Explicit description")
        def documented_asset():
            """Docstring description."""
            return []

        assert documented_asset.description == "Explicit description"

    def test_asset_registry(self):
        """Test assets are registered in global registry."""

        @asset(name="asset_a")
        def asset_a():
            return []

        @asset(name="asset_b", dependencies=["asset_a"])
        def asset_b():
            return []

        registry = get_asset_registry()
        assert "asset_a" in registry
        assert "asset_b" in registry
        assert registry["asset_b"].dependencies == ["asset_a"]

    def test_get_asset(self):
        """Test getting asset by name."""

        @asset(name="findable")
        def findable():
            return []

        found = get_asset("findable")
        assert found is not None
        assert found.name == "findable"

        not_found = get_asset("nonexistent")
        assert not_found is None

    def test_asset_with_group(self):
        """Test asset grouping."""

        @asset(name="grouped_asset", group="etl")
        def grouped():
            return []

        definition = get_asset("grouped_asset")
        assert definition is not None
        assert definition.group == "etl"

    def test_source_asset(self):
        """Test source asset flag."""

        @asset(name="source", is_source=True)
        def source():
            return []

        definition = get_asset("source")
        assert definition is not None
        assert definition.is_source is True

    def test_asset_materialize(self):
        """Test materialize method."""

        @asset
        def simple():
            return {"value": 42}

        output = simple.materialize()
        assert isinstance(output, AssetOutput)
        assert output.value == {"value": 42}

    def test_asset_materialize_with_kwargs(self):
        """Test materialize passes kwargs."""

        @asset
        def parameterized(multiplier: int = 1):
            return {"value": 10 * multiplier}

        output = parameterized.materialize(multiplier=5)
        assert output.value == {"value": 50}


class TestAssetValidation:
    """Tests for asset validation."""

    def test_invalid_name_empty(self):
        """Test empty name raises error."""
        with pytest.raises(ServoValidationError) as exc:

            @asset(name="")
            def bad():
                return []

        assert "cannot be empty" in str(exc.value)

    def test_invalid_name_special_chars(self):
        """Test special characters in name."""
        with pytest.raises(ServoValidationError) as exc:

            @asset(name="invalid@name")
            def bad():
                return []

        assert "alphanumeric" in str(exc.value)

    def test_self_dependency(self):
        """Test asset cannot depend on itself."""
        with pytest.raises(ServoValidationError) as exc:

            @asset(name="self_dep", dependencies=["self_dep"])
            def bad():
                return []

        assert "cannot depend on itself" in str(exc.value)


class TestAssetExecution:
    """Tests for asset execution."""

    def test_execution_error_wrapped(self):
        """Test execution errors are wrapped."""

        @asset
        def failing():
            raise ValueError("Something went wrong")

        with pytest.raises(AssetExecutionError) as exc:
            failing()

        assert exc.value.asset_name == "failing"
        assert exc.value.original_error is not None
        assert isinstance(exc.value.original_error, ValueError)

    def test_asset_preserves_function_name(self):
        """Test wrapper preserves function metadata."""

        @asset
        def my_function():
            """My docstring."""
            return []

        assert my_function.__name__ == "my_function"
        assert my_function.__doc__ == "My docstring."


class TestAssetOutput:
    """Tests for AssetOutput."""

    def test_from_dict(self):
        """Test creating output from dict."""
        output = AssetOutput(value={"key": "value"})
        assert output.value == {"key": "value"}
        assert output.row_count is None

    def test_with_row_count(self):
        """Test output with row count."""
        output = AssetOutput(value=[], row_count=100, byte_size=1024)
        assert output.row_count == 100
        assert output.byte_size == 1024

    def test_with_metadata(self):
        """Test output with metadata."""
        output = AssetOutput(value=[], metadata={"source": "api"})
        assert output.metadata == {"source": "api"}


class TestDuplicateDetection:
    """Tests for duplicate asset detection."""

    def test_duplicate_asset_raises_error(self):
        """Test that registering duplicate asset raises error."""

        @asset(name="unique_asset")
        def first():
            return []

        with pytest.raises(ServoValidationError) as exc:

            @asset(name="unique_asset")
            def second():
                return []

        assert "already registered" in str(exc.value)
        assert "unique_asset" in str(exc.value)


class TestDependencyValidation:
    """Tests for dependency validation."""

    def test_validate_dependencies_success(self):
        """Test validating dependencies when all exist."""

        @asset(name="upstream_asset")
        def upstream():
            return []

        @asset(name="downstream_asset", dependencies=["upstream_asset"])
        def downstream():
            return []

        errors = validate_dependencies()
        assert errors == []

    def test_validate_dependencies_missing(self):
        """Test validating dependencies when some are missing."""

        @asset(name="orphan_asset", dependencies=["nonexistent_asset"])
        def orphan():
            return []

        errors = validate_dependencies()
        assert len(errors) == 1
        assert "nonexistent_asset" in errors[0]

    def test_validate_dependencies_strict_raises(self):
        """Test strict validation raises on missing dependencies."""

        @asset(name="orphan_asset_strict", dependencies=["also_nonexistent"])
        def orphan():
            return []

        with pytest.raises(ServoValidationError) as exc:
            validate_dependencies_strict()

        assert "also_nonexistent" in str(exc.value)
