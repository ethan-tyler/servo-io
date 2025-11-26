"""Asset decorator for defining data assets."""

from __future__ import annotations

import functools
import inspect
import sys
from typing import Any, Callable, TypeVar, overload

if sys.version_info >= (3, 10):
    from typing import ParamSpec
else:
    from typing_extensions import ParamSpec

from servo.exceptions import AssetExecutionError, ServoValidationError
from servo.types import AssetDefinition, AssetMetadata, AssetOutput

P = ParamSpec("P")
R = TypeVar("R")

# Global registry of assets
_asset_registry: dict[str, AssetDefinition] = {}


def get_asset_registry() -> dict[str, AssetDefinition]:
    """Get the global asset registry."""
    return _asset_registry.copy()


def clear_asset_registry() -> None:
    """Clear the asset registry (useful for testing)."""
    _asset_registry.clear()


def get_asset(name: str) -> AssetDefinition | None:
    """Get an asset by name."""
    return _asset_registry.get(name)


def validate_dependencies() -> list[str]:
    """
    Validate all asset dependencies exist in the registry.

    Call this after all assets are registered to ensure dependency graph is valid.

    Returns:
        List of error messages (empty if all dependencies are valid)

    Raises:
        ServoValidationError: If any dependencies are missing (when raise_on_error=True)
    """
    errors: list[str] = []
    for name, definition in _asset_registry.items():
        for dep in definition.dependencies:
            if dep not in _asset_registry:
                errors.append(f"Asset '{name}' depends on unknown asset '{dep}'")
    return errors


def validate_dependencies_strict() -> None:
    """
    Validate all asset dependencies exist, raising an error if any are missing.

    Raises:
        ServoValidationError: If any dependencies are missing
    """
    errors = validate_dependencies()
    if errors:
        raise ServoValidationError(
            f"Invalid dependencies: {'; '.join(errors)}",
            field="dependencies",
        )


class Asset:
    """Wrapper class for asset functions."""

    _func: Callable[..., Any]

    def __init__(
        self,
        func: Callable[..., Any],
        *,
        name: str | None = None,
        dependencies: list[str] | None = None,
        description: str | None = None,
        metadata: dict[str, Any] | None = None,
        group: str | None = None,
        is_source: bool = False,
    ) -> None:
        self._func = func
        # Explicitly check for empty string (don't fall back to func.__name__)
        if name is not None and name == "":
            raise ServoValidationError("Asset name cannot be empty", field="name")
        self._name = name if name is not None else func.__name__
        self._dependencies = dependencies or []
        self._description = description or func.__doc__
        self._group = group
        self._is_source = is_source

        # Build metadata
        meta_dict = metadata or {}
        self._metadata = AssetMetadata(
            owner=meta_dict.get("owner"),
            team=meta_dict.get("team"),
            tags=meta_dict.get("tags", []),
            description=meta_dict.get("description"),
            custom={
                k: v
                for k, v in meta_dict.items()
                if k not in ("owner", "team", "tags", "description")
            },
        )

        # Preserve function metadata
        functools.update_wrapper(self, func)

        # Validate
        self._validate()

        # Register the asset
        self._register()

    def _validate(self) -> None:
        """Validate asset definition."""
        if not self._name:
            raise ServoValidationError("Asset name cannot be empty", field="name")

        if not self._name.replace("_", "").replace("-", "").isalnum():
            raise ServoValidationError(
                f"Asset name must be alphanumeric (with underscores/hyphens): {self._name}",
                field="name",
                value=self._name,
            )

        # Check for circular dependencies (self-reference)
        if self._name in self._dependencies:
            raise ServoValidationError(
                f"Asset cannot depend on itself: {self._name}",
                field="dependencies",
                value=self._dependencies,
            )

    def _register(self) -> None:
        """Register asset in the global registry."""
        # Check for duplicate registration
        if self._name in _asset_registry:
            existing = _asset_registry[self._name]
            raise ServoValidationError(
                f"Asset '{self._name}' already registered from {existing.module}.{existing.function_name}",
                field="name",
                value=self._name,
            )

        module = inspect.getmodule(self._func)
        module_name = module.__name__ if module else "__main__"

        definition = AssetDefinition(
            name=self._name,
            function_name=self._func.__name__,
            module=module_name,
            dependencies=self._dependencies,
            description=self._description,
            metadata=self._metadata,
            group=self._group,
            is_source=self._is_source,
        )
        _asset_registry[self._name] = definition

    @property
    def name(self) -> str:
        """Get asset name."""
        return self._name

    @property
    def dependencies(self) -> list[str]:
        """Get asset dependencies."""
        return self._dependencies.copy()

    @property
    def metadata(self) -> AssetMetadata:
        """Get asset metadata."""
        return self._metadata

    @property
    def description(self) -> str | None:
        """Get asset description."""
        return self._description

    @property
    def definition(self) -> AssetDefinition:
        """Get the full asset definition."""
        return _asset_registry[self._name]

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        """Execute the asset function."""
        try:
            return self._func(*args, **kwargs)
        except Exception as e:
            raise AssetExecutionError(
                f"Failed to execute asset '{self._name}'",
                asset_name=self._name,
                original_error=e,
            ) from e

    def materialize(self, **kwargs: Any) -> AssetOutput:
        """Execute the asset and wrap result in AssetOutput."""
        result: Any = self(**kwargs)

        # If already AssetOutput, return as-is
        if isinstance(result, AssetOutput):
            return result

        # Try to detect DataFrame types and use from_dataframe
        if hasattr(result, "shape") and (
            hasattr(result, "memory_usage") or hasattr(result, "estimated_size")
        ):
            return AssetOutput.from_dataframe(result)

        # Wrap plain values
        return AssetOutput(value=result)


@overload
def asset(func: Callable[..., Any]) -> Asset: ...


@overload
def asset(
    func: None = None,
    *,
    name: str | None = None,
    dependencies: list[str] | None = None,
    description: str | None = None,
    metadata: dict[str, Any] | None = None,
    group: str | None = None,
    is_source: bool = False,
) -> Callable[[Callable[..., Any]], Asset]: ...


def asset(
    func: Callable[..., Any] | None = None,
    *,
    name: str | None = None,
    dependencies: list[str] | None = None,
    description: str | None = None,
    metadata: dict[str, Any] | None = None,
    group: str | None = None,
    is_source: bool = False,
) -> Asset | Callable[[Callable[..., Any]], Asset]:
    """
    Decorator to define a data asset.

    Can be used with or without arguments:

        @asset
        def my_asset():
            return data

        @asset(name="custom_name", dependencies=["upstream"])
        def my_asset():
            return data

    Args:
        func: The function to wrap (when used without parentheses)
        name: Override asset name (defaults to function name)
        dependencies: List of upstream asset names this asset depends on
        description: Human-readable description (defaults to docstring)
        metadata: Dictionary of metadata (owner, team, tags, etc.)
        group: Optional grouping for organization
        is_source: True if this is a source asset (no upstream)

    Returns:
        Asset wrapper or decorator function
    """
    if func is not None:
        # Called without parentheses: @asset
        return Asset(func)

    # Called with arguments: @asset(...)
    def decorator(f: Callable[..., Any]) -> Asset:
        return Asset(
            f,
            name=name,
            dependencies=dependencies,
            description=description,
            metadata=metadata,
            group=group,
            is_source=is_source,
        )

    return decorator
