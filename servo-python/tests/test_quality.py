"""Tests for the data quality framework."""

import pytest

from servo.exceptions import BlockingCheckError, CheckExecutionError, ServoValidationError
from servo.quality import (
    AssetCheck,
    Expectation,
    asset_check,
    check,
    clear_check_registry,
    expect,
    get_check,
    get_check_registry,
    get_checks_for_asset,
    run_checks_for_asset,
)
from servo.types import CheckResult, CheckSeverity, CheckStatus


@pytest.fixture(autouse=True)
def clean_registry():
    """Clear registry before each test."""
    clear_check_registry()
    yield
    clear_check_registry()


# ========== Mock Data for Testing ==========


class MockDataFrame:
    """Mock DataFrame for testing without pandas dependency."""

    def __init__(self, data: dict):
        self._data = data
        self._len = len(next(iter(data.values()))) if data else 0

    def __len__(self):
        return self._len

    def __getitem__(self, key):
        return MockColumn(self._data.get(key, []))

    def duplicated(self, subset=None):  # noqa: ARG002
        """Mock duplicated check."""
        return MockBooleanSeries([False] * self._len)


class MockColumn:
    """Mock column for testing."""

    def __init__(self, values: list, dtype: str = "object"):
        self._values = values
        self.dtype = dtype

    def __iter__(self):
        return iter(self._values)

    def __len__(self):
        return len(self._values)

    def isna(self):
        """Return null mask."""
        return MockBooleanSeries([v is None for v in self._values])

    def duplicated(self):
        """Return duplicate mask."""
        seen = set()
        result = []
        for v in self._values:
            if v in seen:
                result.append(True)
            else:
                seen.add(v)
                result.append(False)
        return MockBooleanSeries(result)

    def between(self, min_val, max_val):
        """Return range mask."""
        return MockBooleanSeries(
            [min_val <= v <= max_val if v is not None else False for v in self._values]
        )

    @property
    def str(self):
        """Return string accessor."""
        return MockStringAccessor(self._values)

    def isin(self, values):
        """Return membership mask."""
        return MockBooleanSeries([v in values for v in self._values])

    def max(self):
        """Return max value (for timestamp freshness checks)."""
        non_null = [v for v in self._values if v is not None]
        return max(non_null) if non_null else None


class MockBooleanSeries:
    """Mock boolean series for testing."""

    def __init__(self, values: list[bool]):
        self._values = values

    def __invert__(self):
        return MockBooleanSeries([not v for v in self._values])

    def sum(self):
        return sum(1 for v in self._values if v)

    def any(self):
        """Return True if any value is True."""
        return any(self._values)


class MockStringAccessor:
    """Mock string accessor for regex operations."""

    def __init__(self, values: list):
        self._values = values

    def match(self, pattern, na=False):
        import re

        compiled = re.compile(pattern)
        return MockBooleanSeries(
            [bool(compiled.match(str(v))) if v is not None else na for v in self._values]
        )


# ========== Test @asset_check Decorator ==========


class TestAssetCheckDecorator:
    """Tests for @asset_check decorator."""

    def test_basic_decorator(self):
        """Test basic @asset_check usage."""

        @asset_check(asset="customers")
        def check_has_data(data):
            return len(data) > 0

        assert isinstance(check_has_data, AssetCheck)
        assert check_has_data.name == "check_has_data"
        assert check_has_data.asset == "customers"

    def test_custom_name(self):
        """Test @asset_check with custom name."""

        @asset_check(asset="orders", name="validate_orders")
        def my_check(_data):
            return True

        assert my_check.name == "validate_orders"

    def test_check_registered(self):
        """Test check is registered in global registry."""

        @asset_check(asset="products")
        def check_products(_data):
            return True

        registry = get_check_registry()
        assert "check_products" in registry
        assert registry["check_products"].asset_name == "products"

    def test_get_check(self):
        """Test getting check by name."""

        @asset_check(asset="users")
        def check_users(_data):
            return True

        found = get_check("check_users")
        assert found is not None
        assert found.name == "check_users"
        assert found.asset_name == "users"

        not_found = get_check("nonexistent")
        assert not_found is None

    def test_get_checks_for_asset(self):
        """Test getting all checks for an asset."""

        @asset_check(asset="orders", name="check_1")
        def c1(_data):
            return True

        @asset_check(asset="orders", name="check_2")
        def c2(_data):
            return True

        @asset_check(asset="customers", name="check_3")
        def c3(_data):
            return True

        order_checks = get_checks_for_asset("orders")
        assert len(order_checks) == 2
        assert {c.name for c in order_checks} == {"check_1", "check_2"}

        customer_checks = get_checks_for_asset("customers")
        assert len(customer_checks) == 1
        assert customer_checks[0].name == "check_3"

    def test_check_returns_bool_true(self):
        """Test check returning True."""

        @asset_check(asset="test_asset")
        def check_pass(_data):
            return True

        result = check_pass({"data": [1, 2, 3]})
        assert result.status == CheckStatus.PASSED
        assert result.check_name == "check_pass"
        assert result.asset_name == "test_asset"

    def test_check_returns_bool_false(self):
        """Test check returning False."""

        @asset_check(asset="test_asset", severity=CheckSeverity.WARNING)
        def check_fail(_data):
            return False

        result = check_fail({"data": []})
        assert result.status == CheckStatus.FAILED
        assert result.severity == CheckSeverity.WARNING
        assert "failed" in result.message.lower()

    def test_check_returns_check_result(self):
        """Test check returning CheckResult directly."""

        @asset_check(asset="test_asset")
        def check_custom(_data):
            return CheckResult.success(
                check_name="check_custom",
                asset_name="test_asset",
                rows_checked=100,
            )

        result = check_custom({})
        assert result.status == CheckStatus.PASSED
        assert result.rows_checked == 100

    def test_check_severity_options(self):
        """Test different severity levels."""

        @asset_check(asset="test", severity=CheckSeverity.INFO)
        def info_check(_data):
            return False

        @asset_check(asset="test", severity=CheckSeverity.WARNING, name="warn_check")
        def warn_check(_data):
            return False

        @asset_check(asset="test", severity=CheckSeverity.ERROR, name="err_check")
        def err_check(_data):
            return False

        assert info_check({}).severity == CheckSeverity.INFO
        assert warn_check({}).severity == CheckSeverity.WARNING
        assert err_check({}).severity == CheckSeverity.ERROR

    def test_blocking_flag(self):
        """Test blocking vs non-blocking checks."""

        @asset_check(asset="test", blocking=True)
        def blocking_check(_data):
            return False

        @asset_check(asset="test", blocking=False, name="non_blocking")
        def non_blocking_check(_data):
            return False

        assert blocking_check({}).blocking is True
        assert non_blocking_check({}).blocking is False

    def test_check_exception_wrapped(self):
        """Test that exceptions in checks are wrapped."""

        @asset_check(asset="test")
        def failing_check(_data):
            raise ValueError("Something went wrong")

        with pytest.raises(CheckExecutionError) as exc:
            failing_check({})

        assert exc.value.check_name == "failing_check"
        assert exc.value.asset_name == "test"
        assert isinstance(exc.value.original_error, ValueError)

    def test_duplicate_check_name_raises(self):
        """Test that duplicate check names raise error."""

        @asset_check(asset="test", name="unique_check")
        def first(_data):
            return True

        with pytest.raises(ServoValidationError) as exc:

            @asset_check(asset="test", name="unique_check")
            def second(_data):
                return True

        assert "already registered" in str(exc.value)

    def test_check_preserves_function_metadata(self):
        """Test that wrapper preserves function metadata."""

        @asset_check(asset="test")
        def documented_check(_data):
            """This is the check docstring."""
            return True

        assert documented_check.__name__ == "documented_check"
        assert documented_check.__doc__ == "This is the check docstring."

    def test_description_from_docstring(self):
        """Test description is extracted from docstring."""

        @asset_check(asset="test")
        def check_with_doc(data):
            """Validates that data is not empty."""
            return len(data) > 0

        definition = get_check("check_with_doc")
        assert definition.description == "Validates that data is not empty."

    def test_explicit_description_override(self):
        """Test explicit description overrides docstring."""

        @asset_check(asset="test", description="Explicit description")
        def check_with_override(_data):
            """Docstring description."""
            return True

        definition = get_check("check_with_override")
        assert definition.description == "Explicit description"


# ========== Test Fluent expect() API ==========


class TestExpectationAPI:
    """Tests for fluent expect() API."""

    def test_expect_creates_expectation(self):
        """Test expect() returns Expectation."""
        data = MockDataFrame({"id": [1, 2, 3]})
        exp = expect(data, asset_name="test")
        assert isinstance(exp, Expectation)

    def test_column_not_null_pass(self):
        """Test column not null check passes."""
        data = MockDataFrame({"id": [1, 2, 3]})
        results = expect(data, asset_name="test").column("id").not_null().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_not_null_fail(self):
        """Test column not null check fails."""
        data = MockDataFrame({"id": [1, None, 3]})
        results = expect(data, asset_name="test").column("id").not_null().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert results[0].rows_failed == 1

    def test_column_unique_pass(self):
        """Test column unique check passes."""
        data = MockDataFrame({"id": [1, 2, 3]})
        results = expect(data, asset_name="test").column("id").unique().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_unique_fail(self):
        """Test column unique check fails."""
        data = MockDataFrame({"id": [1, 2, 2]})
        results = expect(data, asset_name="test").column("id").unique().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED

    def test_column_between_pass(self):
        """Test column between check passes."""
        data = MockDataFrame({"age": [25, 30, 35]})
        results = expect(data, asset_name="test").column("age").to_be_between(0, 100).run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_between_fail(self):
        """Test column between check fails."""
        data = MockDataFrame({"age": [25, 150, 35]})
        results = expect(data, asset_name="test").column("age").to_be_between(0, 100).run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED

    def test_column_regex_pass(self):
        """Test column regex check passes."""
        data = MockDataFrame({"email": ["a@b.com", "c@d.org"]})
        results = expect(data, asset_name="test").column("email").to_match_regex(r".+@.+\..+").run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_regex_fail(self):
        """Test column regex check fails."""
        data = MockDataFrame({"email": ["a@b.com", "invalid"]})
        results = expect(data, asset_name="test").column("email").to_match_regex(r".+@.+\..+").run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED

    def test_column_in_set_pass(self):
        """Test column in_set check passes."""
        data = MockDataFrame({"status": ["active", "pending"]})
        results = (
            expect(data, asset_name="test")
            .column("status")
            .to_be_in(["active", "pending", "inactive"])
            .run()
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_in_set_fail(self):
        """Test column in_set check fails."""
        data = MockDataFrame({"status": ["active", "unknown"]})
        results = (
            expect(data, asset_name="test")
            .column("status")
            .to_be_in(["active", "pending", "inactive"])
            .run()
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED

    def test_column_freshness_pass(self):
        """Test column freshness check passes with fresh data."""
        from datetime import datetime, timedelta, timezone

        now = datetime.now(timezone.utc)
        recent = now - timedelta(seconds=30)  # 30 seconds ago
        data = MockDataFrame({"updated_at": [recent, now]})

        results = (
            expect(data, asset_name="test")
            .column("updated_at")
            .to_be_fresh(max_age_seconds=3600)  # 1 hour
            .run()
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_column_freshness_fail(self):
        """Test column freshness check fails with stale data."""
        from datetime import datetime, timedelta, timezone

        now = datetime.now(timezone.utc)
        old = now - timedelta(hours=2)  # 2 hours ago
        data = MockDataFrame({"updated_at": [old]})

        results = (
            expect(data, asset_name="test")
            .column("updated_at")
            .to_be_fresh(max_age_seconds=3600)  # 1 hour
            .run()
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert "stale" in results[0].message.lower()

    def test_row_count_between_pass(self):
        """Test row count check passes."""
        data = MockDataFrame({"id": [1, 2, 3]})
        results = expect(data, asset_name="test").to_have_row_count_between(1, 10).run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_row_count_between_fail(self):
        """Test row count check fails."""
        data = MockDataFrame({"id": [1, 2, 3]})
        results = expect(data, asset_name="test").to_have_row_count_between(5, 10).run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert "3" in results[0].message  # Should mention actual count

    def test_chained_column_expectations(self):
        """Test multiple expectations on same column."""
        data = MockDataFrame({"id": [1, 2, 3]})
        results = expect(data, asset_name="test").column("id").not_null().unique().run()

        assert len(results) == 2
        assert all(r.status == CheckStatus.PASSED for r in results)

    def test_multiple_columns(self):
        """Test expectations across multiple columns."""
        data = MockDataFrame({"id": [1, 2, 3], "name": ["a", "b", "c"]})
        results = (
            expect(data, asset_name="test")
            .column("id")
            .not_null()
            .column("name")
            .not_null()
            .unique()
            .run()
        )

        assert len(results) == 3
        assert all(r.status == CheckStatus.PASSED for r in results)

    def test_validate_raises_on_blocking_failure(self):
        """Test validate() raises on blocking failures."""
        data = MockDataFrame({"id": [1, None, 3]})

        with pytest.raises(BlockingCheckError) as exc:
            expect(data, asset_name="test", blocking=True).column("id").not_null().validate()

        assert exc.value.asset_name == "test"
        assert len(exc.value.failed_results) == 1

    def test_validate_no_raise_on_non_blocking(self):
        """Test validate() doesn't raise on non-blocking failures."""
        data = MockDataFrame({"id": [1, None, 3]})
        results = (
            expect(data, asset_name="test", blocking=False)
            .column("id")
            .not_null()
            .validate(raise_on_failure=True)
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        # Should not raise

    def test_validate_no_raise_flag(self):
        """Test validate(raise_on_failure=False) returns results."""
        data = MockDataFrame({"id": [1, None, 3]})
        results = (
            expect(data, asset_name="test", blocking=True)
            .column("id")
            .not_null()
            .validate(raise_on_failure=False)
        )

        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED

    def test_no_duplicate_rows(self):
        """Test no duplicate rows check."""
        data = MockDataFrame({"id": [1, 2, 3], "name": ["a", "b", "c"]})
        results = expect(data, asset_name="test").to_have_no_duplicate_rows().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED


# ========== Test check.* Stacked Decorators ==========


class TestCheckDecorators:
    """Tests for check.* stacked decorators."""

    def test_not_null_decorator(self):
        """Test check.not_null decorator."""

        @check.not_null("customer_id")
        def customers():
            return MockDataFrame({"customer_id": [1, 2, 3]})

        # Check should be registered
        checks = get_checks_for_asset("customers")
        assert len(checks) == 1
        assert checks[0].name == "not_null_customer_id"

    def test_unique_decorator(self):
        """Test check.unique decorator."""

        @check.unique("order_id")
        def orders():
            return MockDataFrame({"order_id": [1, 2, 3]})

        checks = get_checks_for_asset("orders")
        assert len(checks) == 1
        assert checks[0].name == "unique_order_id"

    def test_between_decorator(self):
        """Test check.between decorator."""

        @check.between("amount", 0, 1000)
        def transactions():
            return MockDataFrame({"amount": [100, 200, 300]})

        checks = get_checks_for_asset("transactions")
        assert len(checks) == 1
        assert checks[0].name == "between_amount"

    def test_matches_decorator(self):
        """Test check.matches decorator."""

        @check.matches("email", r".+@.+\..+")
        def users():
            return MockDataFrame({"email": ["a@b.com"]})

        checks = get_checks_for_asset("users")
        assert len(checks) == 1
        assert checks[0].name == "matches_email"

    def test_in_set_decorator(self):
        """Test check.in_set decorator."""

        @check.in_set("status", ["active", "inactive"])
        def accounts():
            return MockDataFrame({"status": ["active"]})

        checks = get_checks_for_asset("accounts")
        assert len(checks) == 1
        assert checks[0].name == "in_set_status"

    def test_row_count_decorator(self):
        """Test check.row_count decorator."""

        @check.row_count(min_count=1, max_count=1000)
        def inventory():
            return MockDataFrame({"item": ["a", "b"]})

        checks = get_checks_for_asset("inventory")
        assert len(checks) == 1
        assert checks[0].name == "row_count"

    def test_freshness_decorator(self):
        """Test check.freshness decorator."""

        @check.freshness("updated_at", max_age_seconds=3600)
        def events():
            return MockDataFrame({"updated_at": []})

        checks = get_checks_for_asset("events")
        assert len(checks) == 1
        assert checks[0].name == "freshness_updated_at"

    def test_stacked_decorators(self):
        """Test multiple check decorators stacked."""

        @check.not_null("id")
        @check.unique("id")
        @check.between("age", 0, 150)
        def people():
            return MockDataFrame({"id": [1, 2], "age": [25, 30]})

        checks = get_checks_for_asset("people")
        assert len(checks) == 3
        check_names = {c.name for c in checks}
        assert "not_null_id" in check_names
        assert "unique_id" in check_names
        assert "between_age" in check_names

    def test_custom_check_name(self):
        """Test check decorator with custom name."""

        @check.not_null("field", name="custom_check_name")
        def test_asset():
            return {}

        checks = get_checks_for_asset("test_asset")
        assert checks[0].name == "custom_check_name"

    def test_decorator_severity(self):
        """Test check decorator with severity."""

        @check.not_null("field", severity=CheckSeverity.WARNING)
        def test_asset_warn():
            return {}

        checks = get_checks_for_asset("test_asset_warn")
        assert checks[0].severity == CheckSeverity.WARNING

    def test_decorator_blocking(self):
        """Test check decorator with blocking flag."""

        @check.not_null("field", blocking=False)
        def test_asset_non_blocking():
            return {}

        checks = get_checks_for_asset("test_asset_non_blocking")
        assert checks[0].blocking is False


# ========== Test Blocking Behavior ==========


class TestBlockingBehavior:
    """Tests for blocking check behavior."""

    def test_should_block_property(self):
        """Test should_block property on CheckResult."""
        blocking_failure = CheckResult.failure(
            check_name="test",
            asset_name="test",
            message="Failed",
            severity=CheckSeverity.ERROR,
            blocking=True,
        )
        assert blocking_failure.should_block is True

        non_blocking_failure = CheckResult.failure(
            check_name="test",
            asset_name="test",
            message="Failed",
            severity=CheckSeverity.ERROR,
            blocking=False,
        )
        assert non_blocking_failure.should_block is False

        success = CheckResult.success(check_name="test", asset_name="test")
        assert success.should_block is False

    def test_blocking_check_error_contains_results(self):
        """Test BlockingCheckError contains failed results."""
        data = MockDataFrame({"id": [None, None, None]})

        with pytest.raises(BlockingCheckError) as exc:
            expect(data, asset_name="test", blocking=True).column("id").not_null().validate()

        assert len(exc.value.failed_results) == 1
        assert exc.value.failed_results[0].rows_failed == 3


# ========== Test Check Registry ==========


class TestCheckRegistry:
    """Tests for check registry management."""

    def test_clear_registry(self):
        """Test clearing the check registry."""

        @asset_check(asset="test")
        def check_a(_data):
            return True

        assert len(get_check_registry()) == 1

        clear_check_registry()

        assert len(get_check_registry()) == 0
        assert get_checks_for_asset("test") == []

    def test_registry_isolation(self):
        """Test that registry is properly isolated between tests."""
        # This test relies on autouse fixture clearing registry
        assert len(get_check_registry()) == 0

    def test_run_checks_for_asset(self):
        """Test running all checks for an asset."""

        @asset_check(asset="products", name="check_positive_price")
        def check_price(_data):
            # Always pass for this test
            return True

        @asset_check(asset="products", name="check_has_name")
        def check_name(_data):
            return True

        data = {"price": [10, 20], "name": ["A", "B"]}
        results = run_checks_for_asset("products", data)

        assert len(results) == 2
        assert all(r.status == CheckStatus.PASSED for r in results)

    def test_run_checks_for_asset_no_checks(self):
        """Test running checks when no checks are registered."""
        results = run_checks_for_asset("nonexistent_asset", {})
        assert results == []


# ========== Test CheckResult ==========


class TestCheckResult:
    """Tests for CheckResult dataclass."""

    def test_success_factory(self):
        """Test CheckResult.success factory."""
        result = CheckResult.success(
            check_name="test_check",
            asset_name="test_asset",
            rows_checked=100,
        )
        assert result.status == CheckStatus.PASSED
        assert result.check_name == "test_check"
        assert result.asset_name == "test_asset"
        assert result.rows_checked == 100
        assert result.severity == CheckSeverity.ERROR  # default
        assert result.blocking is True  # default

    def test_failure_factory(self):
        """Test CheckResult.failure factory."""
        result = CheckResult.failure(
            check_name="test_check",
            asset_name="test_asset",
            message="Validation failed",
            severity=CheckSeverity.WARNING,
            blocking=False,
            rows_checked=100,
            rows_failed=10,
        )
        assert result.status == CheckStatus.FAILED
        assert result.message == "Validation failed"
        assert result.severity == CheckSeverity.WARNING
        assert result.blocking is False
        assert result.rows_failed == 10

    def test_duration_tracking(self):
        """Test that check duration is tracked."""

        @asset_check(asset="test")
        def slow_check(_data):
            import time

            time.sleep(0.01)  # 10ms
            return True

        result = slow_check({})
        assert result.duration_ms >= 10


# ========== Test Error Handling ==========


class TestErrorHandling:
    """Tests for error handling in quality checks."""

    def test_check_execution_error(self):
        """Test CheckExecutionError is raised properly."""

        @asset_check(asset="test")
        def error_check(_data):
            raise RuntimeError("Database connection failed")

        with pytest.raises(CheckExecutionError) as exc:
            error_check({})

        assert "error_check" in str(exc.value)
        assert isinstance(exc.value.original_error, RuntimeError)

    def test_invalid_check_return_type(self):
        """Test that invalid return types raise error."""

        @asset_check(asset="test")
        def bad_return_check(_data):
            return "invalid"  # Not bool or CheckResult

        with pytest.raises(ServoValidationError):
            bad_return_check({})

    def test_expectation_handles_empty_data(self):
        """Test expectation handles empty data gracefully."""
        data = MockDataFrame({"id": []})

        # Empty data should pass not_null (no nulls in empty set)
        results = expect(data, asset_name="test").column("id").not_null().run()

        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED
        assert results[0].rows_checked == 0


# ========== Test Referential Integrity Checks ==========


class MockDataFrameWithColumns(MockDataFrame):
    """Extended mock with columns attribute for schema extraction."""

    @property
    def columns(self):
        return list(self._data.keys())

    @property
    def dtypes(self):
        """Return mock dtypes dict (needed for pandas-like schema extraction)."""
        return dict.fromkeys(self._data, "int64")

    def keys(self):
        """Return column names (for dict-like behavior)."""
        return self._data.keys()


class TestReferentialIntegrityChecks:
    """Tests for referential integrity checks."""

    def test_referential_integrity_pass(self):
        """Test RI check passes when all references exist."""
        orders = MockDataFrame({"customer_id": [1, 2, 3]})
        customers = MockDataFrame({"id": [1, 2, 3, 4, 5]})

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id", lookup_data=customers)
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_referential_integrity_fail_orphans(self):
        """Test RI check fails with orphan references."""
        orders = MockDataFrame({"customer_id": [1, 2, 999]})
        customers = MockDataFrame({"id": [1, 2, 3]})

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id", lookup_data=customers)
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert results[0].rows_failed == 1
        assert "999" in results[0].message

    def test_referential_integrity_null_handling_ignore(self):
        """Test RI check ignores nulls by default."""
        orders = MockDataFrame({"customer_id": [1, None, 3]})
        customers = MockDataFrame({"id": [1, 3]})

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id", lookup_data=customers, null_handling="ignore")
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_referential_integrity_multiple_orphans(self):
        """Test RI check reports multiple orphans."""
        orders = MockDataFrame({"customer_id": [1, 100, 200, 300, 400, 500, 600]})
        customers = MockDataFrame({"id": [1]})

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id", lookup_data=customers)
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert results[0].rows_failed == 6
        assert "more" in results[0].message  # Should indicate more orphans

    def test_referential_integrity_with_lookup_query(self):
        """Test RI check with lazy lookup query."""
        orders = MockDataFrame({"customer_id": [1, 2]})
        customers = MockDataFrame({"id": [1, 2, 3]})

        def get_customers():
            return customers

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id", lookup_query=get_customers)
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_referential_integrity_no_lookup_server_side(self):
        """Test RI check returns success message for server-side execution."""
        orders = MockDataFrame({"customer_id": [1, 2]})

        results = (
            expect(orders, asset_name="orders")
            .column("customer_id")
            .to_reference("customers", "id")  # No lookup_data or lookup_query
            .run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED
        assert "server-side" in results[0].message

    def test_referential_integrity_decorator(self):
        """Test check.referential_integrity decorator."""

        @check.referential_integrity("customer_id", "customers", "id")
        def orders():
            return MockDataFrame({"customer_id": [1, 2, 3]})

        # Check should be registered
        checks = get_checks_for_asset("orders")
        assert len(checks) == 1
        assert checks[0].name == "referential_integrity_customer_id"
        assert checks[0].check_type == "referential_integrity"

    def test_referential_integrity_decorator_custom_name(self):
        """Test RI decorator with custom name."""

        @check.referential_integrity(
            "user_id", "users", "id", name="fk_user", severity=CheckSeverity.WARNING
        )
        def posts():
            return MockDataFrame({"user_id": [1]})

        checks = get_checks_for_asset("posts")
        assert checks[0].name == "fk_user"
        assert checks[0].severity == CheckSeverity.WARNING


# ========== Test Schema Match Checks ==========


class TestSchemaMatchChecks:
    """Tests for schema match checks."""

    def test_schema_match_pass(self):
        """Test schema match passes with expected columns."""
        data = MockDataFrameWithColumns({"id": [1], "name": ["test"]})

        results = expect(data, asset_name="test").to_have_schema(["id", "name"]).run()
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_schema_match_fail_missing_column(self):
        """Test schema match fails with missing column."""
        data = MockDataFrameWithColumns({"id": [1]})

        results = expect(data, asset_name="test").to_have_schema(["id", "name"]).run()
        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert "name" in results[0].message
        assert "Missing" in results[0].message

    def test_schema_match_extra_columns_allowed(self):
        """Test extra columns allowed by default."""
        data = MockDataFrameWithColumns({"id": [1], "extra": [True]})

        results = (
            expect(data, asset_name="test").to_have_schema(["id"], allow_extra_columns=True).run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_schema_match_extra_columns_not_allowed(self):
        """Test fails when extra columns not allowed."""
        data = MockDataFrameWithColumns({"id": [1], "extra": [True]})

        results = (
            expect(data, asset_name="test").to_have_schema(["id"], allow_extra_columns=False).run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        assert "extra" in results[0].message
        assert "Unexpected" in results[0].message

    def test_schema_match_multiple_missing(self):
        """Test reports multiple missing columns."""
        data = MockDataFrameWithColumns({"id": [1]})

        results = (
            expect(data, asset_name="test").to_have_schema(["id", "name", "email", "phone"]).run()
        )
        assert len(results) == 1
        assert results[0].status == CheckStatus.FAILED
        # Should mention all missing columns
        assert "name" in results[0].message or "name" in str(results[0].details)

    def test_schema_match_dict_data(self):
        """Test schema match with dict data structure."""
        data = {"id": [1, 2], "name": ["a", "b"]}

        results = expect(data, asset_name="test").to_have_schema(["id", "name"]).run()
        assert len(results) == 1
        assert results[0].status == CheckStatus.PASSED

    def test_schema_match_decorator(self):
        """Test check.schema_match decorator."""

        @check.schema_match(["id", "name", "email"])
        def customers():
            return MockDataFrameWithColumns({"id": [1], "name": ["test"], "email": ["a@b.com"]})

        # Check should be registered
        checks = get_checks_for_asset("customers")
        assert len(checks) == 1
        assert checks[0].name == "schema_match"
        assert checks[0].check_type == "schema_match"

    def test_schema_match_decorator_strict(self):
        """Test schema_match decorator with strict mode."""

        @check.schema_match(["id", "name"], allow_extra_columns=False, severity=CheckSeverity.ERROR)
        def strict_table():
            return MockDataFrameWithColumns({"id": [1], "name": ["test"]})

        checks = get_checks_for_asset("strict_table")
        assert len(checks) == 1
        assert checks[0].parameters["allow_extra_columns"] is False

    def test_schema_match_decorator_custom_name(self):
        """Test schema_match decorator with custom name."""

        @check.schema_match(["col1"], name="custom_schema_check")
        def test_table():
            return {}

        checks = get_checks_for_asset("test_table")
        assert checks[0].name == "custom_schema_check"

    def test_schema_match_execution(self):
        """Test schema_match decorator executes correctly."""

        @check.schema_match(["id", "name"])
        def test_asset_exec():
            return MockDataFrameWithColumns({"id": [1], "name": ["test"]})

        # The check callable should be registered
        from servo.quality import _check_callables

        check_callable = _check_callables.get("schema_match")
        assert check_callable is not None

        # Execute the check
        data = MockDataFrameWithColumns({"id": [1], "name": ["test"]})
        result = check_callable(data)
        assert result.status == CheckStatus.PASSED
