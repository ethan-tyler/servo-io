"""Integration tests for check backend sync.

These tests verify that:
1. CheckDefinition.to_rust_check_type() generates correct JSON for Rust backend
2. check.* decorators properly capture check_type and parameters
3. deploy_checks() syncs local registry to backend
"""

from __future__ import annotations

from unittest.mock import Mock

import pytest

from servo import CheckDefinition, CheckSeverity
from servo.quality import (
    clear_check_registry,
    deploy_checks,
    get_check_registry,
)


class TestCheckDefinitionSchema:
    """Test CheckDefinition.to_rust_check_type() generates correct JSON."""

    def test_not_null_schema(self) -> None:
        """Test not_null check generates correct schema."""
        defn = CheckDefinition(
            name="not_null_email",
            asset_name="customers",
            check_type="not_null",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="email",
            parameters={"columns": ["email"]},
        )
        assert defn.to_rust_check_type() == {
            "type": "not_null",
            "columns": ["email"],
        }

    def test_unique_schema(self) -> None:
        """Test unique check generates correct schema."""
        defn = CheckDefinition(
            name="unique_id",
            asset_name="customers",
            check_type="unique",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="id",
            parameters={"columns": ["id"]},
        )
        assert defn.to_rust_check_type() == {
            "type": "unique",
            "columns": ["id"],
        }

    def test_unique_multi_column_schema(self) -> None:
        """Test unique check with multiple columns."""
        defn = CheckDefinition(
            name="unique_composite",
            asset_name="orders",
            check_type="unique",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            parameters={"columns": ["order_id", "line_item"]},
        )
        assert defn.to_rust_check_type() == {
            "type": "unique",
            "columns": ["order_id", "line_item"],
        }

    def test_in_range_schema(self) -> None:
        """Test in_range check generates correct schema."""
        defn = CheckDefinition(
            name="age_range",
            asset_name="customers",
            check_type="in_range",
            severity=CheckSeverity.WARNING,
            blocking=False,
            function_name="test",
            module="test",
            column="age",
            parameters={"min": 0, "max": 120},
        )
        assert defn.to_rust_check_type() == {
            "type": "in_range",
            "column": "age",
            "min": 0,
            "max": 120,
        }

    def test_regex_schema(self) -> None:
        """Test regex check generates correct schema."""
        defn = CheckDefinition(
            name="email_format",
            asset_name="customers",
            check_type="regex",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="email",
            parameters={"pattern": r"^[^@]+@[^@]+\.[^@]+$"},
        )
        assert defn.to_rust_check_type() == {
            "type": "regex",
            "column": "email",
            "pattern": r"^[^@]+@[^@]+\.[^@]+$",
        }

    def test_row_count_schema(self) -> None:
        """Test row_count check generates correct schema."""
        defn = CheckDefinition(
            name="row_count",
            asset_name="customers",
            check_type="row_count",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            parameters={"min": 1, "max": 1000000},
        )
        assert defn.to_rust_check_type() == {
            "type": "row_count",
            "min": 1,
            "max": 1000000,
        }

    def test_accepted_values_schema(self) -> None:
        """Test accepted_values check generates correct schema."""
        defn = CheckDefinition(
            name="status_values",
            asset_name="orders",
            check_type="accepted_values",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="status",
            parameters={"values": ["pending", "shipped", "delivered", "cancelled"]},
        )
        assert defn.to_rust_check_type() == {
            "type": "accepted_values",
            "column": "status",
            "values": ["pending", "shipped", "delivered", "cancelled"],
        }

    def test_no_duplicate_rows_schema(self) -> None:
        """Test no_duplicate_rows check generates correct schema."""
        defn = CheckDefinition(
            name="no_dups",
            asset_name="customers",
            check_type="no_duplicate_rows",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            parameters={"columns": ["email", "phone"]},
        )
        assert defn.to_rust_check_type() == {
            "type": "no_duplicate_rows",
            "columns": ["email", "phone"],
        }

    def test_freshness_schema(self) -> None:
        """Test freshness check generates correct schema."""
        defn = CheckDefinition(
            name="freshness_updated_at",
            asset_name="customers",
            check_type="freshness",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="updated_at",
            parameters={"max_age_seconds": 3600},
        )
        assert defn.to_rust_check_type() == {
            "type": "freshness",
            "timestamp_column": "updated_at",
            "max_age_seconds": 3600,
        }

    def test_custom_schema(self) -> None:
        """Test custom check generates correct schema."""
        defn = CheckDefinition(
            name="custom_check",
            asset_name="customers",
            check_type="custom",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="my_custom_check",
            module="test",
            description="A custom validation check",
        )
        assert defn.to_rust_check_type() == {
            "type": "custom",
            "name": "custom_check",
            "description": "A custom validation check",
        }


class TestToApiPayload:
    """Test CheckDefinition.to_api_payload() for backend sync."""

    def test_api_payload_includes_rust_schema(self) -> None:
        """Test that to_api_payload includes the Rust-compatible check_type."""
        defn = CheckDefinition(
            name="not_null_email",
            asset_name="customers",
            check_type="not_null",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            description="Email must not be null",
            column="email",
            parameters={"columns": ["email"]},
        )
        payload = defn.to_api_payload()

        assert payload["name"] == "not_null_email"
        assert payload["description"] == "Email must not be null"
        assert payload["severity"] == "error"
        assert payload["blocking"] is True
        assert payload["check_type"] == {
            "type": "not_null",
            "columns": ["email"],
        }

    def test_api_payload_with_warning_severity(self) -> None:
        """Test payload with warning severity."""
        defn = CheckDefinition(
            name="age_range",
            asset_name="customers",
            check_type="in_range",
            severity=CheckSeverity.WARNING,
            blocking=False,
            function_name="test",
            module="test",
            column="age",
            parameters={"min": 0, "max": 120},
        )
        payload = defn.to_api_payload()

        assert payload["severity"] == "warning"
        assert payload["blocking"] is False


class TestDeployChecks:
    """Test deploy_checks() syncs to backend."""

    @pytest.fixture(autouse=True)
    def setup(self) -> None:
        """Clear registry before and after each test."""
        clear_check_registry()
        yield
        clear_check_registry()

    def test_deploy_creates_checks(self) -> None:
        """Test that deploy_checks creates checks in the backend."""
        # Manually register a check
        defn = CheckDefinition(
            name="not_null_email",
            asset_name="customers",
            check_type="not_null",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="email",
            parameters={"columns": ["email"]},
        )
        from servo.quality import _register_check

        _register_check(defn)

        mock_client = Mock()
        mock_client.get_asset.return_value = {"id": "asset-123"}
        mock_client.create_check.return_value = {"id": "check-456"}

        result = deploy_checks(mock_client, asset_name="customers")

        assert "not_null_email" in result["created"]
        mock_client.create_check.assert_called_once()
        # Verify the payload includes Rust-compatible schema
        call_args = mock_client.create_check.call_args
        assert call_args[0][0] == "asset-123"  # asset_id
        check_defn = call_args[0][1]
        assert check_defn.check_type == "not_null"

    def test_deploy_skips_missing_asset(self) -> None:
        """Test that deploy_checks skips checks when asset not found."""
        defn = CheckDefinition(
            name="unique_id",
            asset_name="nonexistent",
            check_type="unique",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="id",
            parameters={"columns": ["id"]},
        )
        from servo.quality import _register_check

        _register_check(defn)

        from servo.exceptions import ServoAPIError

        mock_client = Mock()
        mock_client.get_asset.side_effect = ServoAPIError(
            "Asset not found", status_code=404, response_body={}
        )

        result = deploy_checks(mock_client, asset_name="nonexistent")

        assert "unique_id" in result["skipped"]
        mock_client.create_check.assert_not_called()

    def test_deploy_skips_existing_checks(self) -> None:
        """Test that deploy_checks handles 409 conflict for existing checks."""
        defn = CheckDefinition(
            name="existing_check",
            asset_name="customers",
            check_type="not_null",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="id",
            parameters={"columns": ["id"]},
        )
        from servo.quality import _register_check

        _register_check(defn)

        from servo.exceptions import ServoAPIError

        mock_client = Mock()
        mock_client.get_asset.return_value = {"id": "asset-123"}
        mock_client.create_check.side_effect = ServoAPIError(
            "Check already exists", status_code=409, response_body={}
        )

        result = deploy_checks(mock_client, asset_name="customers")

        assert "existing_check" in result["skipped"]


class TestDecoratorCheckTypes:
    """Test that check.* decorators set proper check_type."""

    @pytest.fixture(autouse=True)
    def setup(self) -> None:
        """Clear registry before and after each test."""
        clear_check_registry()
        yield
        clear_check_registry()

    def test_not_null_decorator_sets_check_type(self) -> None:
        """Test check.not_null sets correct check_type."""
        from servo import asset, check

        @check.not_null("email")
        @asset(name="customers")
        def customers() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("not_null_email")

        assert check_def is not None
        assert check_def.check_type == "not_null"
        assert check_def.column == "email"
        assert check_def.parameters == {"columns": ["email"]}

    def test_unique_decorator_sets_check_type(self) -> None:
        """Test check.unique sets correct check_type."""
        from servo import asset, check

        @check.unique("id")
        @asset(name="users")
        def users() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("unique_id")

        assert check_def is not None
        assert check_def.check_type == "unique"
        assert check_def.parameters == {"columns": ["id"]}

    def test_between_decorator_sets_check_type(self) -> None:
        """Test check.between sets correct check_type (in_range)."""
        from servo import asset, check

        @check.between("age", 0, 150)
        @asset(name="people")
        def people() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("between_age")

        assert check_def is not None
        assert check_def.check_type == "in_range"
        assert check_def.column == "age"
        assert check_def.parameters == {"min": 0, "max": 150}

    def test_matches_decorator_sets_check_type(self) -> None:
        """Test check.matches sets correct check_type (regex)."""
        from servo import asset, check

        @check.matches("email", r"^[^@]+@[^@]+$")
        @asset(name="contacts")
        def contacts() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("matches_email")

        assert check_def is not None
        assert check_def.check_type == "regex"
        assert check_def.parameters == {"pattern": r"^[^@]+@[^@]+$"}

    def test_in_set_decorator_sets_check_type(self) -> None:
        """Test check.in_set sets correct check_type (accepted_values)."""
        from servo import asset, check

        @check.in_set("status", ["active", "inactive"])
        @asset(name="accounts")
        def accounts() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("in_set_status")

        assert check_def is not None
        assert check_def.check_type == "accepted_values"
        assert check_def.parameters == {"values": ["active", "inactive"]}

    def test_row_count_decorator_sets_check_type(self) -> None:
        """Test check.row_count sets correct check_type."""
        from servo import asset, check

        @check.row_count(min_count=1, max_count=1000)
        @asset(name="records")
        def records() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("row_count")

        assert check_def is not None
        assert check_def.check_type == "row_count"
        assert check_def.parameters == {"min": 1, "max": 1000}

    def test_freshness_decorator_sets_check_type(self) -> None:
        """Test check.freshness sets correct check_type."""
        from servo import asset, check

        @check.freshness("updated_at", max_age_seconds=3600)
        @asset(name="events")
        def events() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("freshness_updated_at")

        assert check_def is not None
        assert check_def.check_type == "freshness"
        assert check_def.column == "updated_at"
        assert check_def.parameters == {"max_age_seconds": 3600}

    def test_referential_integrity_decorator_sets_check_type(self) -> None:
        """Test check.referential_integrity sets correct check_type."""
        from servo import asset, check

        @check.referential_integrity("customer_id", "customers", "id")
        @asset(name="orders")
        def orders() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("referential_integrity_customer_id")

        assert check_def is not None
        assert check_def.check_type == "referential_integrity"
        assert check_def.column == "customer_id"
        assert check_def.parameters == {
            "reference_table": "customers",
            "reference_column": "id",
        }

    def test_schema_match_decorator_sets_check_type(self) -> None:
        """Test check.schema_match sets correct check_type."""
        from servo import asset, check

        @check.schema_match(["id", "name", "email"], allow_extra_columns=False)
        @asset(name="user_profiles")
        def user_profiles() -> list:
            return []

        registry = get_check_registry()
        check_def = registry.get("schema_match")

        assert check_def is not None
        assert check_def.check_type == "schema_match"
        assert check_def.parameters == {
            "expected_columns": ["id", "name", "email"],
            "allow_extra_columns": False,
        }


class TestReferentialIntegritySchema:
    """Test referential_integrity check generates correct Rust schema."""

    def test_referential_integrity_rust_schema(self) -> None:
        """Test RI check generates correct Rust-compatible schema."""
        defn = CheckDefinition(
            name="fk_customer",
            asset_name="orders",
            check_type="referential_integrity",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            column="customer_id",
            parameters={
                "reference_table": "customers",
                "reference_column": "id",
            },
        )
        assert defn.to_rust_check_type() == {
            "type": "referential_integrity",
            "column": "customer_id",
            "reference_table": "customers",
            "reference_column": "id",
        }

    def test_referential_integrity_api_payload(self) -> None:
        """Test RI check generates correct API payload."""
        defn = CheckDefinition(
            name="fk_user",
            asset_name="posts",
            check_type="referential_integrity",
            severity=CheckSeverity.WARNING,
            blocking=False,
            function_name="test",
            module="test",
            description="Posts must reference valid users",
            column="user_id",
            parameters={
                "reference_table": "users",
                "reference_column": "id",
            },
        )
        payload = defn.to_api_payload()

        assert payload["name"] == "fk_user"
        assert payload["description"] == "Posts must reference valid users"
        assert payload["severity"] == "warning"
        assert payload["blocking"] is False
        assert payload["check_type"] == {
            "type": "referential_integrity",
            "column": "user_id",
            "reference_table": "users",
            "reference_column": "id",
        }


class TestSchemaMatchSchema:
    """Test schema_match check generates correct Rust schema."""

    def test_schema_match_rust_schema(self) -> None:
        """Test schema_match generates correct Rust-compatible schema."""
        defn = CheckDefinition(
            name="schema_check",
            asset_name="customers",
            check_type="schema_match",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            parameters={
                "expected_columns": ["id", "name", "email"],
                "allow_extra_columns": True,
            },
        )
        assert defn.to_rust_check_type() == {
            "type": "schema_match",
            "expected_columns": ["id", "name", "email"],
            "allow_extra_columns": True,
        }

    def test_schema_match_strict_mode_schema(self) -> None:
        """Test schema_match with strict mode (no extra columns)."""
        defn = CheckDefinition(
            name="strict_schema",
            asset_name="contracts",
            check_type="schema_match",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            parameters={
                "expected_columns": ["contract_id", "value"],
                "allow_extra_columns": False,
            },
        )
        assert defn.to_rust_check_type() == {
            "type": "schema_match",
            "expected_columns": ["contract_id", "value"],
            "allow_extra_columns": False,
        }

    def test_schema_match_api_payload(self) -> None:
        """Test schema_match generates correct API payload."""
        defn = CheckDefinition(
            name="required_columns",
            asset_name="invoices",
            check_type="schema_match",
            severity=CheckSeverity.ERROR,
            blocking=True,
            function_name="test",
            module="test",
            description="Invoices must have required columns",
            parameters={
                "expected_columns": ["invoice_id", "amount", "date"],
                "allow_extra_columns": True,
            },
        )
        payload = defn.to_api_payload()

        assert payload["name"] == "required_columns"
        assert payload["description"] == "Invoices must have required columns"
        assert payload["severity"] == "error"
        assert payload["blocking"] is True
        assert payload["check_type"] == {
            "type": "schema_match",
            "expected_columns": ["invoice_id", "amount", "date"],
            "allow_extra_columns": True,
        }
