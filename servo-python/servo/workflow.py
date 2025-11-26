"""Workflow decorator for defining asset pipelines."""

from __future__ import annotations

import functools
import inspect
import sys
from typing import Callable, TypeVar, overload

if sys.version_info >= (3, 10):
    from typing import ParamSpec
else:
    from typing_extensions import ParamSpec

from servo.asset import Asset, get_asset
from servo.exceptions import ServoValidationError
from servo.types import WorkflowDefinition

P = ParamSpec("P")
R = TypeVar("R")

# Global registry of workflows
_workflow_registry: dict[str, WorkflowDefinition] = {}


def get_workflow_registry() -> dict[str, WorkflowDefinition]:
    """Get the global workflow registry."""
    return _workflow_registry.copy()


def clear_workflow_registry() -> None:
    """Clear the workflow registry (useful for testing)."""
    _workflow_registry.clear()


def get_workflow(name: str) -> WorkflowDefinition | None:
    """Get a workflow by name."""
    return _workflow_registry.get(name)


class Workflow:
    """Wrapper class for workflow functions."""

    def __init__(
        self,
        func: Callable[P, R],
        *,
        name: str | None = None,
        schedule: str | None = None,
        description: str | None = None,
        executor: str | None = None,
        timeout_seconds: int = 3600,
        retries: int = 0,
    ) -> None:
        self._func = func
        # Explicitly check for empty string (don't fall back to func.__name__)
        if name is not None and name == "":
            raise ServoValidationError("Workflow name cannot be empty", field="name")
        self._name = name if name is not None else func.__name__
        self._schedule = schedule
        self._description = description or func.__doc__
        self._executor = executor
        self._timeout_seconds = timeout_seconds
        self._retries = retries
        self._assets: list[str] = []

        # Preserve function metadata
        functools.update_wrapper(self, func)

        # Validate
        self._validate()

        # Note: We don't register immediately - we need to call the function
        # to get the list of assets first
        self._registered = False

    def _validate(self) -> None:
        """Validate workflow definition."""
        if not self._name:
            raise ServoValidationError("Workflow name cannot be empty", field="name")

        if not self._name.replace("_", "").replace("-", "").isalnum():
            raise ServoValidationError(
                f"Workflow name must be alphanumeric (with underscores/hyphens): {self._name}",
                field="name",
                value=self._name,
            )

        if self._schedule is not None:
            self._validate_cron(self._schedule)

        if self._timeout_seconds <= 0:
            raise ServoValidationError(
                "Timeout must be positive",
                field="timeout_seconds",
                value=self._timeout_seconds,
            )

        if self._retries < 0:
            raise ServoValidationError(
                "Retries cannot be negative",
                field="retries",
                value=self._retries,
            )

    def _validate_cron(self, schedule: str) -> None:
        """
        Validate cron expression format.

        Validates standard 5-part cron: minute hour day-of-month month day-of-week
        Supports: numbers, *, ranges (1-5), steps (*/5), lists (1,3,5)

        Note: This is basic validation. For production, consider using croniter.
        """
        parts = schedule.split()
        if len(parts) != 5:
            raise ServoValidationError(
                f"Invalid cron expression (expected 5 parts): {schedule}",
                field="schedule",
                value=schedule,
            )

        # Define valid ranges for each field
        ranges = [
            (0, 59, "minute"),      # minute
            (0, 23, "hour"),        # hour
            (1, 31, "day-of-month"),  # day of month
            (1, 12, "month"),       # month
            (0, 6, "day-of-week"),  # day of week (0=Sunday)
        ]

        for part, (min_val, max_val, field_name) in zip(parts, ranges):
            if not self._validate_cron_field(part, min_val, max_val):
                raise ServoValidationError(
                    f"Invalid {field_name} in cron expression: {part} (valid: {min_val}-{max_val})",
                    field="schedule",
                    value=schedule,
                )

    def _validate_cron_field(self, field: str, min_val: int, max_val: int) -> bool:
        """Validate a single cron field."""
        if field == "*":
            return True

        # Handle step syntax (*/5 or 1-10/2)
        if "/" in field:
            base, step = field.split("/", 1)
            if not step.isdigit() or int(step) < 1:
                return False
            if base == "*":
                return True
            field = base

        # Handle list syntax (1,3,5)
        if "," in field:
            return all(self._validate_cron_field(p, min_val, max_val) for p in field.split(","))

        # Handle range syntax (1-5)
        if "-" in field:
            try:
                start, end = field.split("-", 1)
                start_val, end_val = int(start), int(end)
                return min_val <= start_val <= max_val and min_val <= end_val <= max_val
            except ValueError:
                return False

        # Single value
        try:
            val = int(field)
            return min_val <= val <= max_val
        except ValueError:
            return False

    def _register(self, assets: list[str]) -> None:
        """Register workflow in the global registry."""
        # Check for duplicate registration
        if self._name in _workflow_registry:
            existing = _workflow_registry[self._name]
            raise ServoValidationError(
                f"Workflow '{self._name}' already registered from {existing.module}.{existing.function_name}",
                field="name",
                value=self._name,
            )

        self._assets = assets
        module = inspect.getmodule(self._func)
        module_name = module.__name__ if module else "__main__"

        definition = WorkflowDefinition(
            name=self._name,
            function_name=self._func.__name__,
            module=module_name,
            schedule=self._schedule,
            assets=assets,
            description=self._description,
            executor=self._executor,
            timeout_seconds=self._timeout_seconds,
            retries=self._retries,
        )
        _workflow_registry[self._name] = definition
        self._registered = True

    @property
    def name(self) -> str:
        """Get workflow name."""
        return self._name

    @property
    def schedule(self) -> str | None:
        """Get workflow schedule."""
        return self._schedule

    @property
    def assets(self) -> list[str]:
        """Get list of assets in this workflow."""
        if not self._registered:
            # Need to call the function to get assets
            self()
        return self._assets.copy()

    @property
    def definition(self) -> WorkflowDefinition | None:
        """Get the full workflow definition."""
        return _workflow_registry.get(self._name)

    def __call__(self, *args: P.args, **kwargs: P.kwargs) -> list[str]:
        """Execute the workflow function to get asset list."""
        result = self._func(*args, **kwargs)

        # Convert result to asset names
        asset_names: list[str] = []
        if isinstance(result, (list, tuple)):
            for item in result:
                if isinstance(item, Asset):
                    asset_names.append(item.name)
                elif isinstance(item, str):
                    # Verify asset exists
                    if get_asset(item) is None:
                        raise ServoValidationError(
                            f"Unknown asset in workflow: {item}",
                            field="assets",
                            value=item,
                        )
                    asset_names.append(item)
                else:
                    raise ServoValidationError(
                        f"Workflow must return Asset objects or asset names, got: {type(item)}",
                        field="return_value",
                        value=type(item).__name__,
                    )
        else:
            raise ServoValidationError(
                "Workflow function must return a list of assets",
                field="return_value",
                value=type(result).__name__,
            )

        # Register the workflow now that we have the assets
        if not self._registered:
            self._register(asset_names)

        return asset_names


@overload
def workflow(func: Callable[P, R]) -> Workflow: ...


@overload
def workflow(
    *,
    name: str | None = None,
    schedule: str | None = None,
    description: str | None = None,
    executor: str | None = None,
    timeout_seconds: int = 3600,
    retries: int = 0,
) -> Callable[[Callable[P, R]], Workflow]: ...


def workflow(
    func: Callable[P, R] | None = None,
    *,
    name: str | None = None,
    schedule: str | None = None,
    description: str | None = None,
    executor: str | None = None,
    timeout_seconds: int = 3600,
    retries: int = 0,
) -> Workflow | Callable[[Callable[P, R]], Workflow]:
    """
    Decorator to define a workflow (pipeline of assets).

    Can be used with or without arguments:

        @workflow
        def my_pipeline():
            return [asset1, asset2]

        @workflow(name="daily_etl", schedule="0 0 * * *")
        def my_pipeline():
            return [asset1, asset2]

    Args:
        func: The function to wrap (when used without parentheses)
        name: Override workflow name (defaults to function name)
        schedule: Cron expression for scheduled execution
        description: Human-readable description (defaults to docstring)
        executor: Executor type (e.g., "cloudrun", "local")
        timeout_seconds: Maximum execution time (default: 3600)
        retries: Number of retry attempts (default: 0)

    Returns:
        Workflow wrapper or decorator function
    """
    if func is not None:
        # Called without parentheses: @workflow
        return Workflow(func)

    # Called with arguments: @workflow(...)
    def decorator(f: Callable[P, R]) -> Workflow:
        return Workflow(
            f,
            name=name,
            schedule=schedule,
            description=description,
            executor=executor,
            timeout_seconds=timeout_seconds,
            retries=retries,
        )

    return decorator
