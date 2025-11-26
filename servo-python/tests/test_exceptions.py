"""Tests for Servo SDK exceptions."""

from servo.exceptions import (
    AssetExecutionError,
    ServoAPIError,
    ServoConfigError,
    ServoConnectionError,
    ServoError,
    ServoTimeoutError,
    ServoValidationError,
)


class TestServoError:
    """Tests for base ServoError."""

    def test_basic_error(self):
        """Test basic error creation."""
        error = ServoError("Something went wrong")
        assert str(error) == "Something went wrong"
        assert error.message == "Something went wrong"
        assert error.details == {}

    def test_error_with_details(self):
        """Test error with details."""
        error = ServoError("Error occurred", details={"code": 123})
        assert "code" in error.details
        assert error.details["code"] == 123
        assert "123" in str(error)


class TestServoAPIError:
    """Tests for ServoAPIError."""

    def test_api_error_with_status(self):
        """Test API error with status code."""
        error = ServoAPIError("Not found", status_code=404)
        assert error.status_code == 404
        assert error.details["status_code"] == 404

    def test_api_error_with_response(self):
        """Test API error with response body."""
        error = ServoAPIError(
            "Validation failed",
            status_code=400,
            response_body={"errors": ["invalid field"]},
        )
        assert error.response_body == {"errors": ["invalid field"]}
        assert error.details["response"] == {"errors": ["invalid field"]}


class TestServoConfigError:
    """Tests for ServoConfigError."""

    def test_config_error_with_key(self):
        """Test config error with config key."""
        error = ServoConfigError("Missing config", config_key="api_key")
        assert error.config_key == "api_key"
        assert error.details["config_key"] == "api_key"


class TestServoValidationError:
    """Tests for ServoValidationError."""

    def test_validation_error_with_field(self):
        """Test validation error with field info."""
        error = ServoValidationError(
            "Invalid value",
            field="name",
            value="bad@value",
        )
        assert error.field == "name"
        assert error.value == "bad@value"
        assert error.details["field"] == "name"


class TestServoConnectionError:
    """Tests for ServoConnectionError."""

    def test_connection_error_with_url(self):
        """Test connection error with URL."""
        error = ServoConnectionError(
            "Failed to connect",
            url="https://api.servo.io",
        )
        assert error.url == "https://api.servo.io"
        assert "url" in error.details


class TestServoTimeoutError:
    """Tests for ServoTimeoutError."""

    def test_timeout_error_with_operation(self):
        """Test timeout error with operation info."""
        error = ServoTimeoutError(
            "Operation timed out",
            operation="materialize",
            timeout_seconds=60.0,
        )
        assert error.operation == "materialize"
        assert error.timeout_seconds == 60.0
        assert error.details["timeout_seconds"] == 60.0


class TestAssetExecutionError:
    """Tests for AssetExecutionError."""

    def test_execution_error_basic(self):
        """Test execution error with asset name."""
        error = AssetExecutionError(
            "Execution failed",
            asset_name="my_asset",
        )
        assert error.asset_name == "my_asset"
        assert error.original_error is None

    def test_execution_error_with_original(self):
        """Test execution error wrapping original."""
        original = ValueError("Bad input")
        error = AssetExecutionError(
            "Execution failed",
            asset_name="my_asset",
            original_error=original,
        )
        assert error.original_error is original
        assert "ValueError" in error.details["error_type"]
        assert "Bad input" in error.details["original_error"]
