"""Data quality framework for Servo SDK.

This module provides three complementary APIs for data quality checks:

1. @asset_check decorator - Standalone check functions linked to assets
2. expect() fluent API - Chainable expectations for inline validation
3. check.* decorators - Inline check decorators on asset functions

Implemented Check Types:
    - not_null: Column must not contain null values
    - unique: Column must have unique values (no duplicates)
    - between/in_range: Column values must be within a numeric range
    - matches/regex: Column values must match a regex pattern
    - in_set/accepted_values: Column values must be in an allowed set
    - row_count: Table must have row count within range
    - freshness: Timestamp column must have recent data (within max_age)
    - no_duplicate_rows: Table must not have duplicate rows
    - custom: User-defined check functions via @asset_check

Deferred Check Types (defined in Rust core, not yet in Python):
    - referential_integrity: Foreign key relationships (requires DB access)
    - schema_match: Column names and types validation

Example usage:

    # API 1: @asset_check decorator
    @asset_check(asset="customers", blocking=True)
    def check_customer_ids(df):
        nulls = df["customer_id"].isna().sum()
        return nulls == 0

    # API 2: Fluent expect() API
    @asset(name="orders")
    def orders():
        df = load_orders()
        expect(df, asset_name="orders") \\
            .column("order_id").not_null().unique() \\
            .column("updated_at").to_be_fresh(max_age_seconds=3600) \\
            .validate(raise_on_failure=True)
        return df

    # API 3: Stacked check.* decorators
    @check.not_null("customer_id")
    @check.unique("customer_id")
    @check.freshness("updated_at", max_age_seconds=3600)
    @asset(name="customers")
    def customers():
        return load_customers()
"""

from __future__ import annotations

import functools
import inspect
import re
import time
from typing import TYPE_CHECKING, Any, Callable, TypeVar

from servo.exceptions import BlockingCheckError, CheckExecutionError, ServoValidationError
from servo.types import CheckDefinition, CheckResult, CheckSeverity, CheckStatus

if TYPE_CHECKING:
    from collections.abc import Iterator

F = TypeVar("F", bound=Callable[..., Any])

# ========== Global Registry ==========

# Global registry of checks: name -> CheckDefinition
_check_registry: dict[str, CheckDefinition] = {}

# Registry of callable check instances: name -> AssetCheck
_check_callables: dict[str, "AssetCheck"] = {}

# Mapping from asset name to list of check names
_asset_checks: dict[str, list[str]] = {}


def get_check_registry() -> dict[str, CheckDefinition]:
    """Get a copy of the global check registry."""
    return _check_registry.copy()


def clear_check_registry() -> None:
    """Clear the check registry (useful for testing)."""
    _check_registry.clear()
    _check_callables.clear()
    _asset_checks.clear()


def get_check(name: str) -> CheckDefinition | None:
    """Get a check definition by name."""
    return _check_registry.get(name)


def get_checks_for_asset(asset_name: str) -> list[CheckDefinition]:
    """Get all checks registered for an asset."""
    check_names = _asset_checks.get(asset_name, [])
    return [_check_registry[name] for name in check_names if name in _check_registry]


def _register_check(definition: CheckDefinition, callable_instance: "AssetCheck | None" = None) -> None:
    """Register a check in the global registry."""
    if definition.name in _check_registry:
        raise ServoValidationError(
            f"Check '{definition.name}' is already registered",
            field="name",
            value=definition.name,
        )
    _check_registry[definition.name] = definition
    if callable_instance is not None:
        _check_callables[definition.name] = callable_instance
    if definition.asset_name not in _asset_checks:
        _asset_checks[definition.asset_name] = []
    _asset_checks[definition.asset_name].append(definition.name)


# ========== API 1: @asset_check Decorator ==========


class AssetCheck:
    """Wrapper class for asset check functions."""

    def __init__(
        self,
        func: Callable[..., Any],
        *,
        asset: str,
        name: str | None = None,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        description: str | None = None,
    ) -> None:
        self._func = func
        self._asset = asset
        self._name = name or func.__name__
        self._severity = severity
        self._blocking = blocking
        self._description = description or func.__doc__

        # Register the check
        module = getattr(func, "__module__", "<unknown>")
        definition = CheckDefinition(
            name=self._name,
            asset_name=asset,
            check_type="custom",
            severity=severity,
            blocking=blocking,
            function_name=func.__name__,
            module=module,
            description=self._description,
        )
        _register_check(definition, callable_instance=self)

        functools.update_wrapper(self, func)

    def __call__(self, data: Any, **kwargs: Any) -> CheckResult:
        """Execute the check function."""
        start_time = time.perf_counter()
        try:
            result = self._func(data, **kwargs)
            duration_ms = (time.perf_counter() - start_time) * 1000

            # Handle different return types
            if isinstance(result, CheckResult):
                result.duration_ms = duration_ms
                return result
            elif isinstance(result, bool):
                if result:
                    return CheckResult.success(
                        check_name=self._name,
                        asset_name=self._asset,
                        duration_ms=duration_ms,
                    )
                else:
                    return CheckResult.failure(
                        check_name=self._name,
                        asset_name=self._asset,
                        message=f"Check '{self._name}' failed",
                        severity=self._severity,
                        blocking=self._blocking,
                        duration_ms=duration_ms,
                    )
            else:
                raise ServoValidationError(
                    f"Check function must return bool or CheckResult, got {type(result).__name__}",
                    field="return_type",
                    value=type(result).__name__,
                )
        except ServoValidationError:
            raise
        except Exception as e:
            duration_ms = (time.perf_counter() - start_time) * 1000
            raise CheckExecutionError(
                f"Check '{self._name}' raised an exception: {e}",
                check_name=self._name,
                asset_name=self._asset,
                original_error=e,
            ) from e

    @property
    def name(self) -> str:
        """Get the check name."""
        return self._name

    @property
    def asset(self) -> str:
        """Get the asset name this check is attached to."""
        return self._asset


def asset_check(
    asset: str,
    *,
    name: str | None = None,
    severity: CheckSeverity = CheckSeverity.ERROR,
    blocking: bool = True,
    description: str | None = None,
) -> Callable[[F], AssetCheck]:
    """Decorator to define a check function for an asset.

    Args:
        asset: Name of the asset this check validates
        name: Optional name for the check (defaults to function name)
        severity: Severity level for failures (default: ERROR)
        blocking: Whether failure blocks downstream execution (default: True)
        description: Description of what this check validates

    Example:
        @asset_check(asset="customers", blocking=True)
        def check_no_nulls(df):
            return df["customer_id"].notna().all()
    """

    def decorator(func: F) -> AssetCheck:
        return AssetCheck(
            func,
            asset=asset,
            name=name,
            severity=severity,
            blocking=blocking,
            description=description,
        )

    return decorator


# ========== API 2: Fluent expect() API ==========


class ColumnExpectation:
    """Fluent API for column-level expectations."""

    def __init__(self, expectation: Expectation, column: str) -> None:
        self._expectation = expectation
        self._column = column
        self._checks: list[Callable[[Any], CheckResult]] = []

    def not_null(self) -> ColumnExpectation:
        """Expect column values to not be null."""

        def check(data: Any) -> CheckResult:
            try:
                if hasattr(data, "isna"):  # pandas
                    null_count = int(data[self._column].isna().sum())
                    total = len(data)
                elif hasattr(data, "null_count"):  # polars
                    null_count = data[self._column].null_count()
                    total = len(data)
                else:
                    # Generic fallback
                    null_count = sum(1 for v in data[self._column] if v is None)
                    total = len(data[self._column])

                if null_count == 0:
                    return CheckResult.success(
                        check_name=f"not_null_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name=f"not_null_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has {null_count} null values",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                        rows_failed=null_count,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"not_null_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking not_null: {e}",
                )

        self._checks.append(check)
        return self

    def unique(self) -> ColumnExpectation:
        """Expect column values to be unique (no duplicates)."""

        def check(data: Any) -> CheckResult:
            try:
                if hasattr(data, "duplicated"):  # pandas
                    dup_count = int(data[self._column].duplicated().sum())
                    total = len(data)
                elif hasattr(data, "is_duplicated"):  # polars
                    dup_count = data[self._column].is_duplicated().sum()
                    total = len(data)
                else:
                    # Generic fallback
                    values = list(data[self._column])
                    dup_count = len(values) - len(set(values))
                    total = len(values)

                if dup_count == 0:
                    return CheckResult.success(
                        check_name=f"unique_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name=f"unique_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has {dup_count} duplicate values",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                        rows_failed=dup_count,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"unique_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking unique: {e}",
                )

        self._checks.append(check)
        return self

    def to_be_between(self, min_value: Any, max_value: Any) -> ColumnExpectation:
        """Expect column values to be within a range (inclusive)."""

        def check(data: Any) -> CheckResult:
            try:
                col_data = data[self._column]
                if hasattr(col_data, "between"):  # pandas
                    out_of_range = int((~col_data.between(min_value, max_value)).sum())
                else:
                    # Generic fallback
                    out_of_range = sum(
                        1 for v in col_data if v is not None and (v < min_value or v > max_value)
                    )
                total = len(data)

                if out_of_range == 0:
                    return CheckResult.success(
                        check_name=f"between_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name=f"between_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has {out_of_range} values outside [{min_value}, {max_value}]",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                        rows_failed=out_of_range,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"between_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking between: {e}",
                )

        self._checks.append(check)
        return self

    def to_match_regex(self, pattern: str) -> ColumnExpectation:
        """Expect column values to match a regex pattern."""
        compiled = re.compile(pattern)

        def check(data: Any) -> CheckResult:
            try:
                col_data = data[self._column]
                if hasattr(col_data, "str"):  # pandas
                    non_match = int((~col_data.str.match(pattern, na=False)).sum())
                else:
                    # Generic fallback
                    non_match = sum(
                        1 for v in col_data if v is not None and not compiled.match(str(v))
                    )
                total = len(data)

                if non_match == 0:
                    return CheckResult.success(
                        check_name=f"regex_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name=f"regex_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has {non_match} values not matching pattern",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                        rows_failed=non_match,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"regex_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking regex: {e}",
                )

        self._checks.append(check)
        return self

    def to_be_in(self, allowed_values: list[Any]) -> ColumnExpectation:
        """Expect column values to be in a set of allowed values."""
        allowed_set = set(allowed_values)

        def check(data: Any) -> CheckResult:
            try:
                col_data = data[self._column]
                if hasattr(col_data, "isin"):  # pandas
                    not_in = int((~col_data.isin(allowed_set)).sum())
                else:
                    # Generic fallback
                    not_in = sum(1 for v in col_data if v not in allowed_set)
                total = len(data)

                if not_in == 0:
                    return CheckResult.success(
                        check_name=f"in_set_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name=f"in_set_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has {not_in} values not in allowed set",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                        rows_failed=not_in,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"in_set_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking in_set: {e}",
                )

        self._checks.append(check)
        return self

    def to_be_fresh(self, max_age_seconds: int) -> ColumnExpectation:
        """Expect column (timestamp) to have values within max_age_seconds of now.

        This checks data freshness by ensuring the maximum timestamp value
        in the column is not older than max_age_seconds from the current time.
        """
        from datetime import datetime, timedelta, timezone

        def check(data: Any) -> CheckResult:
            try:
                col_data = data[self._column]
                now = datetime.now(timezone.utc)
                max_age = timedelta(seconds=max_age_seconds)
                cutoff = now - max_age

                # Get max timestamp from column
                if hasattr(col_data, "max"):  # pandas/polars
                    max_ts = col_data.max()
                    # Handle pandas Timestamp
                    if hasattr(max_ts, "to_pydatetime"):
                        max_ts = max_ts.to_pydatetime()
                    # Ensure timezone aware
                    if max_ts.tzinfo is None:
                        max_ts = max_ts.replace(tzinfo=timezone.utc)
                else:
                    # Generic fallback
                    timestamps = [v for v in col_data if v is not None]
                    max_ts = max(timestamps) if timestamps else None

                total = len(data)

                if max_ts is None:
                    return CheckResult.failure(
                        check_name=f"freshness_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' has no timestamp values",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                    )

                if max_ts >= cutoff:
                    return CheckResult.success(
                        check_name=f"freshness_{self._column}",
                        asset_name=self._expectation._asset_name,
                        rows_checked=total,
                    )
                else:
                    age_seconds = (now - max_ts).total_seconds()
                    return CheckResult.failure(
                        check_name=f"freshness_{self._column}",
                        asset_name=self._expectation._asset_name,
                        message=f"Column '{self._column}' is stale: max timestamp is {age_seconds:.0f}s old (limit: {max_age_seconds}s)",
                        severity=self._expectation._severity,
                        blocking=self._expectation._blocking,
                        rows_checked=total,
                    )
            except Exception as e:
                return CheckResult(
                    check_name=f"freshness_{self._column}",
                    status=CheckStatus.ERROR,
                    severity=self._expectation._severity,
                    asset_name=self._expectation._asset_name,
                    blocking=self._expectation._blocking,
                    message=f"Error checking freshness: {e}",
                )

        self._checks.append(check)
        return self

    # Chain back to parent expectation
    def column(self, name: str) -> ColumnExpectation:
        """Start defining expectations for another column."""
        return self._expectation.column(name)

    def to_have_row_count_between(self, min_count: int, max_count: int) -> Expectation:
        """Expect table to have row count within range."""
        return self._expectation.to_have_row_count_between(min_count, max_count)

    def run(self) -> list[CheckResult]:
        """Execute all expectations and return results."""
        return self._expectation.run()

    def validate(self, raise_on_failure: bool = True) -> list[CheckResult]:
        """Execute all expectations and optionally raise on failure."""
        return self._expectation.validate(raise_on_failure)

    def _run_checks(self, data: Any) -> Iterator[CheckResult]:
        """Run all checks for this column."""
        for check in self._checks:
            yield check(data)


class Expectation:
    """Fluent API for building data quality expectations."""

    def __init__(
        self,
        data: Any,
        asset_name: str = "unknown",
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
    ) -> None:
        self._data = data
        self._asset_name = asset_name
        self._severity = severity
        self._blocking = blocking
        self._columns: list[ColumnExpectation] = []
        self._table_checks: list[Callable[[Any], CheckResult]] = []

    def column(self, name: str) -> ColumnExpectation:
        """Start defining expectations for a column."""
        col_exp = ColumnExpectation(self, name)
        self._columns.append(col_exp)
        return col_exp

    def to_have_row_count_between(self, min_count: int, max_count: int) -> Expectation:
        """Expect table to have row count within range."""

        def check(data: Any) -> CheckResult:
            row_count = len(data)
            if min_count <= row_count <= max_count:
                return CheckResult.success(
                    check_name="row_count",
                    asset_name=self._asset_name,
                    rows_checked=row_count,
                )
            else:
                return CheckResult.failure(
                    check_name="row_count",
                    asset_name=self._asset_name,
                    message=f"Row count {row_count} not in range [{min_count}, {max_count}]",
                    severity=self._severity,
                    blocking=self._blocking,
                    rows_checked=row_count,
                )

        self._table_checks.append(check)
        return self

    def to_have_no_duplicate_rows(self, subset: list[str] | None = None) -> Expectation:
        """Expect no duplicate rows (optionally based on subset of columns)."""

        def check(data: Any) -> CheckResult:
            try:
                if hasattr(data, "duplicated"):  # pandas
                    if subset:
                        dup_count = int(data.duplicated(subset=subset).sum())
                    else:
                        dup_count = int(data.duplicated().sum())
                else:
                    # Generic fallback - basic duplicate check
                    if hasattr(data, "__iter__"):
                        rows = [tuple(row) if hasattr(row, "__iter__") else row for row in data]
                        dup_count = len(rows) - len(set(rows))
                    else:
                        dup_count = 0
                total = len(data)

                if dup_count == 0:
                    return CheckResult.success(
                        check_name="no_duplicate_rows",
                        asset_name=self._asset_name,
                        rows_checked=total,
                    )
                else:
                    return CheckResult.failure(
                        check_name="no_duplicate_rows",
                        asset_name=self._asset_name,
                        message=f"Found {dup_count} duplicate rows",
                        severity=self._severity,
                        blocking=self._blocking,
                        rows_checked=total,
                        rows_failed=dup_count,
                    )
            except Exception as e:
                return CheckResult(
                    check_name="no_duplicate_rows",
                    status=CheckStatus.ERROR,
                    severity=self._severity,
                    asset_name=self._asset_name,
                    blocking=self._blocking,
                    message=f"Error checking duplicates: {e}",
                )

        self._table_checks.append(check)
        return self

    def run(self) -> list[CheckResult]:
        """Execute all expectations and return results."""
        results: list[CheckResult] = []

        # Run table-level checks
        for check in self._table_checks:
            results.append(check(self._data))

        # Run column-level checks
        for col_exp in self._columns:
            results.extend(col_exp._run_checks(self._data))

        return results

    def validate(self, raise_on_failure: bool = True) -> list[CheckResult]:
        """Execute all expectations and optionally raise on blocking failures."""
        results = self.run()

        if raise_on_failure:
            failures = [r for r in results if r.should_block]
            if failures:
                raise BlockingCheckError(
                    f"{len(failures)} blocking check(s) failed for asset '{self._asset_name}'",
                    check_name=failures[0].check_name,
                    asset_name=self._asset_name,
                    failed_results=failures,
                )

        return results


def expect(
    data: Any,
    asset_name: str = "unknown",
    severity: CheckSeverity = CheckSeverity.ERROR,
    blocking: bool = True,
) -> Expectation:
    """Create an expectation builder for data quality validation.

    Args:
        data: The data to validate (DataFrame, list, etc.)
        asset_name: Name of the asset being validated
        severity: Default severity for check failures
        blocking: Whether failures should block downstream by default

    Returns:
        Expectation builder for chaining validations

    Example:
        expect(df, asset_name="orders") \\
            .column("order_id").not_null().unique() \\
            .column("amount").to_be_between(0, 1000000) \\
            .validate()
    """
    return Expectation(data, asset_name, severity, blocking)


# ========== API 3: check.* Stacked Decorators ==========


class _InlineCheckCallable:
    """Wrapper for inline check decorators that doesn't auto-register.

    Unlike AssetCheck, this class does not register itself in __init__.
    Registration is handled explicitly by the check.* decorators.
    """

    def __init__(
        self,
        func: Callable[..., CheckResult],
        name: str,
        asset_name: str,
        severity: CheckSeverity,
        blocking: bool,
        description: str | None,
    ) -> None:
        self._func = func
        self._name = name
        self._asset = asset_name
        self._severity = severity
        self._blocking = blocking
        self._description = description

    def __call__(self, data: Any, **kwargs: Any) -> CheckResult:
        """Execute the check function with timing and error handling."""
        start_time = time.perf_counter()
        try:
            result = self._func(data, **kwargs)
            duration_ms = (time.perf_counter() - start_time) * 1000
            result.duration_ms = duration_ms
            return result
        except Exception as e:
            duration_ms = (time.perf_counter() - start_time) * 1000
            raise CheckExecutionError(
                f"Check '{self._name}' raised an exception: {e}",
                check_name=self._name,
                asset_name=self._asset,
                original_error=e,
            ) from e

    @property
    def name(self) -> str:
        """Get the check name."""
        return self._name

    @property
    def asset(self) -> str:
        """Get the asset name."""
        return self._asset


class check:
    """Namespace for built-in check decorators that attach to assets.

    These decorators can be stacked on top of @asset decorators to
    declaratively attach checks to assets.

    Example:
        @check.not_null("customer_id")
        @check.unique("customer_id")
        @check.between("age", 0, 150, severity=CheckSeverity.WARNING)
        @asset(name="customers")
        def customers():
            return load_customers()
    """

    @staticmethod
    def not_null(
        column: str,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a not-null check for a column."""

        def decorator(func: F) -> F:
            check_name = name or f"not_null_{column}"
            # Get the asset name from the decorated function
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type and parameters for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="not_null",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must not contain null values",
                column=column,
                parameters={"columns": [column]},
            )

            # Create the check callable
            def _check_impl(data: Any) -> CheckResult:
                return expect(data, asset_name=asset_name, severity=severity, blocking=blocking).column(
                    column
                ).not_null().run()[0]

            # Wrap with timing/error handling (doesn't auto-register)
            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            # Explicitly register the check with correct definition
            _register_check(definition, callable_instance=check_callable)

            # Attach check to the function for later discovery
            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def unique(
        column: str,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a uniqueness check for a column."""

        def decorator(func: F) -> F:
            check_name = name or f"unique_{column}"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="unique",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must have unique values",
                column=column,
                parameters={"columns": [column]},
            )

            def _check_impl(data: Any) -> CheckResult:
                return expect(data, asset_name=asset_name, severity=severity, blocking=blocking).column(
                    column
                ).unique().run()[0]

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def between(
        column: str,
        min_value: Any,
        max_value: Any,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a range check for a column."""

        def decorator(func: F) -> F:
            check_name = name or f"between_{column}"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="in_range",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must be between {min_value} and {max_value}",
                column=column,
                parameters={"min": min_value, "max": max_value},
            )

            def _check_impl(data: Any) -> CheckResult:
                return (
                    expect(data, asset_name=asset_name, severity=severity, blocking=blocking)
                    .column(column)
                    .to_be_between(min_value, max_value)
                    .run()[0]
                )

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def matches(
        column: str,
        pattern: str,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a regex pattern check for a column."""

        def decorator(func: F) -> F:
            check_name = name or f"matches_{column}"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="regex",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must match pattern '{pattern}'",
                column=column,
                parameters={"pattern": pattern},
            )

            def _check_impl(data: Any) -> CheckResult:
                return (
                    expect(data, asset_name=asset_name, severity=severity, blocking=blocking)
                    .column(column)
                    .to_match_regex(pattern)
                    .run()[0]
                )

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def in_set(
        column: str,
        allowed_values: list[Any],
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add an allowed values check for a column."""

        def decorator(func: F) -> F:
            check_name = name or f"in_set_{column}"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="accepted_values",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must be one of {allowed_values}",
                column=column,
                parameters={"values": allowed_values},
            )

            def _check_impl(data: Any) -> CheckResult:
                return (
                    expect(data, asset_name=asset_name, severity=severity, blocking=blocking)
                    .column(column)
                    .to_be_in(allowed_values)
                    .run()[0]
                )

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def row_count(
        min_count: int | None = None,
        max_count: int | None = None,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a row count check."""

        def decorator(func: F) -> F:
            check_name = name or "row_count"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="row_count",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Row count must be between {min_count} and {max_count}",
                parameters={"min": min_count, "max": max_count},
            )

            def _check_impl(data: Any) -> CheckResult:
                actual_row_count = len(data)
                min_val = min_count if min_count is not None else 0
                max_val = max_count if max_count is not None else float("inf")

                if min_val <= actual_row_count <= max_val:
                    return CheckResult.success(
                        check_name=check_name,
                        asset_name=asset_name,
                        rows_checked=actual_row_count,
                    )
                else:
                    return CheckResult.failure(
                        check_name=check_name,
                        asset_name=asset_name,
                        message=f"Row count {actual_row_count} not in range [{min_val}, {max_val}]",
                        severity=severity,
                        blocking=blocking,
                        rows_checked=actual_row_count,
                    )

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator

    @staticmethod
    def freshness(
        column: str,
        max_age_seconds: int,
        severity: CheckSeverity = CheckSeverity.ERROR,
        blocking: bool = True,
        name: str | None = None,
    ) -> Callable[[F], F]:
        """Add a freshness check for a timestamp column.

        Validates that the maximum timestamp value in the column is not older
        than max_age_seconds from the current time.
        """

        def decorator(func: F) -> F:
            check_name = name or f"freshness_{column}"
            asset_name = getattr(func, "_servo_asset_name", func.__name__)
            module = getattr(func, "__module__", "<unknown>")

            # Create definition with proper check_type for Rust backend
            definition = CheckDefinition(
                name=check_name,
                asset_name=asset_name,
                check_type="freshness",  # Matches Rust enum variant
                severity=severity,
                blocking=blocking,
                function_name=func.__name__,
                module=module,
                description=f"Column '{column}' must have data fresher than {max_age_seconds}s",
                column=column,
                parameters={"max_age_seconds": max_age_seconds},
            )

            def _check_impl(data: Any) -> CheckResult:
                return (
                    expect(data, asset_name=asset_name, severity=severity, blocking=blocking)
                    .column(column)
                    .to_be_fresh(max_age_seconds)
                    .run()[0]
                )

            check_callable = _InlineCheckCallable(
                _check_impl,
                check_name,
                asset_name,
                severity,
                blocking,
                definition.description,
            )

            _register_check(definition, callable_instance=check_callable)

            if not hasattr(func, "_servo_checks"):
                func._servo_checks = []  # type: ignore
            func._servo_checks.append(check_callable)  # type: ignore

            return func

        return decorator


# ========== Utility Functions ==========


def deploy_checks(
    client: Any,  # ServoClient - avoid circular import
    asset_name: str | None = None,
) -> dict[str, list[str]]:
    """Deploy locally-registered checks to the backend.

    This function syncs check definitions from the local Python registry
    to the Servo backend, enabling server-side execution by the worker.

    Args:
        client: ServoClient instance for API calls
        asset_name: Optional asset name to filter checks. If None, deploys all.

    Returns:
        Dict with "created", "updated", "skipped" lists of check names

    Example:
        from servo import ServoClient
        from servo.quality import deploy_checks

        client = ServoClient(api_key="...", tenant_id="...")

        # Deploy all registered checks
        result = deploy_checks(client)
        print(f"Created: {result['created']}")

        # Deploy checks for a specific asset
        result = deploy_checks(client, asset_name="customers")
    """
    from servo.exceptions import ServoAPIError

    # Get checks to deploy
    if asset_name:
        checks = get_checks_for_asset(asset_name)
    else:
        checks = list(_check_registry.values())

    results: dict[str, list[str]] = {"created": [], "updated": [], "skipped": []}

    # Group checks by asset
    checks_by_asset: dict[str, list[CheckDefinition]] = {}
    for check_def in checks:
        if check_def.asset_name not in checks_by_asset:
            checks_by_asset[check_def.asset_name] = []
        checks_by_asset[check_def.asset_name].append(check_def)

    # Deploy checks for each asset
    for asset, asset_checks in checks_by_asset.items():
        try:
            # Get asset ID from backend
            asset_info = client.get_asset(asset)
            asset_id = asset_info.get("id")

            if not asset_id:
                for check_def in asset_checks:
                    results["skipped"].append(check_def.name)
                continue

            # Try to create each check
            for check_def in asset_checks:
                try:
                    client.create_check(asset_id, check_def)
                    results["created"].append(check_def.name)
                except ServoAPIError as e:
                    if e.status_code == 409:  # Conflict - already exists
                        results["skipped"].append(check_def.name)
                    else:
                        raise

        except ServoAPIError:
            # Asset not found or other error - skip these checks
            for check_def in asset_checks:
                results["skipped"].append(check_def.name)

    return results


def run_checks_for_asset(asset_name: str, data: Any) -> list[CheckResult]:
    """Run all registered checks for an asset.

    Args:
        asset_name: Name of the asset to run checks for
        data: The data to validate

    Returns:
        List of check results
    """
    checks = get_checks_for_asset(asset_name)
    results: list[CheckResult] = []

    for check_def in checks:
        # Find the callable check instance
        check_func = _check_callables.get(check_def.name)
        if check_func is not None:
            try:
                result = check_func(data)
                results.append(result)
            except CheckExecutionError as e:
                results.append(
                    CheckResult(
                        check_name=check_def.name,
                        status=CheckStatus.ERROR,
                        severity=check_def.severity,
                        asset_name=asset_name,
                        blocking=check_def.blocking,
                        message=str(e),
                    )
                )

    return results
