"""Servo SDK exceptions."""

from __future__ import annotations

from typing import Any


class ServoError(Exception):
    """Base exception for all Servo SDK errors."""

    def __init__(self, message: str, details: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.message = message
        self.details = details or {}

    def __str__(self) -> str:
        if self.details:
            return f"{self.message} - Details: {self.details}"
        return self.message


class ServoAPIError(ServoError):
    """Error from Servo API calls."""

    def __init__(
        self,
        message: str,
        status_code: int | None = None,
        response_body: dict[str, Any] | None = None,
    ) -> None:
        details: dict[str, Any] = {}
        if status_code is not None:
            details["status_code"] = status_code
        if response_body is not None:
            details["response"] = response_body
        super().__init__(message, details)
        self.status_code = status_code
        self.response_body = response_body


class ServoConfigError(ServoError):
    """Configuration error."""

    def __init__(self, message: str, config_key: str | None = None) -> None:
        details = {}
        if config_key is not None:
            details["config_key"] = config_key
        super().__init__(message, details)
        self.config_key = config_key


class ServoValidationError(ServoError):
    """Validation error for assets, workflows, or inputs."""

    def __init__(
        self,
        message: str,
        field: str | None = None,
        value: Any = None,
    ) -> None:
        details: dict[str, Any] = {}
        if field is not None:
            details["field"] = field
        if value is not None:
            details["value"] = repr(value)
        super().__init__(message, details)
        self.field = field
        self.value = value


class ServoConnectionError(ServoError):
    """Connection error to Servo API."""

    def __init__(self, message: str, url: str | None = None) -> None:
        details = {}
        if url is not None:
            details["url"] = url
        super().__init__(message, details)
        self.url = url


class ServoTimeoutError(ServoError):
    """Timeout error waiting for operation."""

    def __init__(
        self,
        message: str,
        operation: str | None = None,
        timeout_seconds: float | None = None,
    ) -> None:
        details: dict[str, Any] = {}
        if operation is not None:
            details["operation"] = operation
        if timeout_seconds is not None:
            details["timeout_seconds"] = timeout_seconds
        super().__init__(message, details)
        self.operation = operation
        self.timeout_seconds = timeout_seconds


class AssetExecutionError(ServoError):
    """Error during asset execution."""

    def __init__(
        self,
        message: str,
        asset_name: str,
        original_error: Exception | None = None,
    ) -> None:
        details: dict[str, Any] = {"asset_name": asset_name}
        if original_error is not None:
            details["original_error"] = str(original_error)
            details["error_type"] = type(original_error).__name__
        super().__init__(message, details)
        self.asset_name = asset_name
        self.original_error = original_error


class CheckExecutionError(ServoError):
    """Error during check execution."""

    def __init__(
        self,
        message: str,
        check_name: str,
        asset_name: str,
        original_error: Exception | None = None,
    ) -> None:
        details: dict[str, Any] = {
            "check_name": check_name,
            "asset_name": asset_name,
        }
        if original_error is not None:
            details["original_error"] = str(original_error)
            details["error_type"] = type(original_error).__name__
        super().__init__(message, details)
        self.check_name = check_name
        self.asset_name = asset_name
        self.original_error = original_error


class BlockingCheckError(ServoError):
    """Error raised when a blocking check fails."""

    def __init__(
        self,
        message: str,
        check_name: str,
        asset_name: str,
        failed_results: list[Any] | None = None,
    ) -> None:
        details: dict[str, Any] = {
            "check_name": check_name,
            "asset_name": asset_name,
        }
        if failed_results:
            details["failed_count"] = len(failed_results)
        super().__init__(message, details)
        self.check_name = check_name
        self.asset_name = asset_name
        self.failed_results = failed_results or []
