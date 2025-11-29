"""Tests for the @workflow decorator."""

import pytest

from servo.asset import asset, clear_asset_registry
from servo.exceptions import ServoValidationError
from servo.workflow import (
    Workflow,
    clear_workflow_registry,
    get_workflow,
    get_workflow_registry,
    workflow,
)


@pytest.fixture(autouse=True)
def clean_registries():
    """Clear registries before each test."""
    clear_asset_registry()
    clear_workflow_registry()
    yield
    clear_asset_registry()
    clear_workflow_registry()


class TestWorkflowDecorator:
    """Tests for @workflow decorator."""

    def test_basic_workflow_no_args(self):
        """Test @workflow without parentheses."""

        @asset
        def asset_a():
            return []

        @workflow
        def my_workflow():
            return [asset_a]

        assert isinstance(my_workflow, Workflow)
        assert my_workflow.name == "my_workflow"
        assert my_workflow.schedule is None

    def test_basic_workflow_with_args(self):
        """Test @workflow with arguments."""

        @asset
        def asset_a():
            return []

        @workflow(name="custom_workflow", schedule="0 * * * *")
        def my_workflow():
            return [asset_a]

        assert my_workflow.name == "custom_workflow"
        assert my_workflow.schedule == "0 * * * *"

    def test_workflow_with_multiple_assets(self):
        """Test workflow with multiple assets."""

        @asset(name="raw_data")
        def raw_data():
            return []

        @asset(name="processed_data", dependencies=["raw_data"])
        def processed_data():
            return []

        @asset(name="final_data", dependencies=["processed_data"])
        def final_data():
            return []

        @workflow(name="etl_pipeline")
        def etl():
            return [raw_data, processed_data, final_data]

        assets = etl()
        assert assets == ["raw_data", "processed_data", "final_data"]

    def test_workflow_registry(self):
        """Test workflows are registered in global registry."""

        @asset
        def asset_a():
            return []

        @workflow(name="registered_workflow")
        def my_workflow():
            return [asset_a]

        # Call to trigger registration
        my_workflow()

        registry = get_workflow_registry()
        assert "registered_workflow" in registry

    def test_get_workflow(self):
        """Test getting workflow by name."""

        @asset
        def asset_a():
            return []

        @workflow(name="findable")
        def findable():
            return [asset_a]

        findable()  # Register

        found = get_workflow("findable")
        assert found is not None
        assert found.name == "findable"

        not_found = get_workflow("nonexistent")
        assert not_found is None

    def test_workflow_with_executor(self):
        """Test workflow with custom executor."""

        @asset
        def asset_a():
            return []

        @workflow(name="cloud_workflow", executor="cloudrun")
        def cloud():
            return [asset_a]

        cloud()
        definition = get_workflow("cloud_workflow")
        assert definition is not None
        assert definition.executor == "cloudrun"

    def test_workflow_with_timeout(self):
        """Test workflow with custom timeout."""

        @asset
        def asset_a():
            return []

        @workflow(name="long_workflow", timeout_seconds=7200)
        def long_running():
            return [asset_a]

        long_running()
        definition = get_workflow("long_workflow")
        assert definition is not None
        assert definition.timeout_seconds == 7200

    def test_workflow_with_retries(self):
        """Test workflow with retries."""

        @asset
        def asset_a():
            return []

        @workflow(name="retry_workflow", retries=3)
        def retry():
            return [asset_a]

        retry()
        definition = get_workflow("retry_workflow")
        assert definition is not None
        assert definition.retries == 3

    def test_workflow_with_asset_names(self):
        """Test workflow can use asset names as strings."""

        @asset(name="named_asset")
        def named():
            return []

        @workflow(name="string_workflow")
        def using_names():
            return ["named_asset"]

        assets = using_names()
        assert assets == ["named_asset"]


class TestWorkflowValidation:
    """Tests for workflow validation."""

    def test_invalid_name_empty(self):
        """Test empty name raises error."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(name="")
            def bad():
                return []

        assert "cannot be empty" in str(exc.value)

    def test_invalid_cron_expression(self):
        """Test invalid cron expression."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="invalid")
            def bad():
                return []

        assert "cron" in str(exc.value).lower()

    def test_invalid_timeout(self):
        """Test invalid timeout value."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(timeout_seconds=0)
            def bad():
                return []

        assert "positive" in str(exc.value).lower()

    def test_negative_retries(self):
        """Test negative retries value."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(retries=-1)
            def bad():
                return []

        assert "negative" in str(exc.value).lower()

    def test_unknown_asset_reference(self):
        """Test referencing unknown asset by name."""

        @workflow(name="bad_workflow")
        def bad():
            return ["nonexistent_asset"]

        with pytest.raises(ServoValidationError) as exc:
            bad()

        assert "Unknown asset" in str(exc.value)

    def test_invalid_return_type(self):
        """Test workflow must return list."""

        @workflow
        def bad():
            return "not a list"

        with pytest.raises(ServoValidationError) as exc:
            bad()

        assert "list" in str(exc.value).lower()


class TestWorkflowAssets:
    """Tests for workflow asset management."""

    def test_assets_property(self):
        """Test assets property returns asset names."""

        @asset(name="a")
        def asset_a():
            return []

        @asset(name="b")
        def asset_b():
            return []

        @workflow(name="multi_asset")
        def multi():
            return [asset_a, asset_b]

        assets = multi.assets
        assert assets == ["a", "b"]

    def test_definition_to_dict(self):
        """Test workflow definition serialization."""

        @asset
        def asset_a():
            return []

        @workflow(
            name="serializable",
            schedule="0 0 * * *",
            description="Test workflow",
            timeout_seconds=1800,
            retries=2,
        )
        def serializable():
            """Docstring."""
            return [asset_a]

        serializable()
        definition = get_workflow("serializable")
        assert definition is not None

        as_dict = definition.to_dict()
        assert as_dict["name"] == "serializable"
        assert as_dict["schedule"] == "0 0 * * *"
        assert as_dict["timeout_seconds"] == 1800
        assert as_dict["retries"] == 2


class TestWorkflowDuplicateDetection:
    """Tests for workflow duplicate detection."""

    def test_duplicate_workflow_raises_error(self):
        """Test that registering duplicate workflow raises error."""

        @asset(name="dup_wf_asset")
        def asset_a():
            return []

        @workflow(name="unique_workflow")
        def first():
            return [asset_a]

        first()  # Register the workflow

        with pytest.raises(ServoValidationError) as exc:

            @workflow(name="unique_workflow")
            def second():
                return [asset_a]

            second()  # Try to register again

        assert "already registered" in str(exc.value)
        assert "unique_workflow" in str(exc.value)


class TestCronValidation:
    """Tests for enhanced cron validation."""

    def test_valid_cron_basic(self):
        """Test basic valid cron expressions."""

        @asset(name="cron_asset")
        def asset_a():
            return []

        @workflow(name="cron_test_1", schedule="0 0 * * *")
        def daily():
            return [asset_a]

        @workflow(name="cron_test_2", schedule="*/15 * * * *")
        def every_15_min():
            return [asset_a]

        @workflow(name="cron_test_3", schedule="0 9-17 * * 1-5")
        def business_hours():
            return [asset_a]

        # These should not raise
        daily()
        every_15_min()
        business_hours()

    def test_valid_cron_complex(self):
        """Test complex valid cron expressions."""

        @asset(name="cron_complex_asset")
        def asset_a():
            return []

        @workflow(name="cron_complex_1", schedule="0 0 1,15 * *")
        def biweekly():
            return [asset_a]

        @workflow(name="cron_complex_2", schedule="0 */6 * * *")
        def every_6_hours():
            return [asset_a]

        biweekly()
        every_6_hours()

    def test_invalid_minute_out_of_range(self):
        """Test minute out of valid range."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="60 * * * *")
            def bad():
                return []

        assert "minute" in str(exc.value).lower()

    def test_invalid_hour_out_of_range(self):
        """Test hour out of valid range."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="0 24 * * *")
            def bad():
                return []

        assert "hour" in str(exc.value).lower()

    def test_invalid_day_of_month_out_of_range(self):
        """Test day of month out of valid range."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="0 0 32 * *")
            def bad():
                return []

        assert "day-of-month" in str(exc.value).lower()

    def test_invalid_month_out_of_range(self):
        """Test month out of valid range."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="0 0 * 13 *")
            def bad():
                return []

        assert "month" in str(exc.value).lower()

    def test_invalid_day_of_week_out_of_range(self):
        """Test day of week out of valid range."""
        with pytest.raises(ServoValidationError) as exc:

            @workflow(schedule="0 0 * * 7")
            def bad():
                return []

        assert "day-of-week" in str(exc.value).lower()
