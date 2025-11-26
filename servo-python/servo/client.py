"""Servo API client for interacting with the Servo backend."""

from __future__ import annotations

import os
from datetime import datetime
from typing import Any

import httpx

from servo.exceptions import (
    ServoAPIError,
    ServoConfigError,
    ServoConnectionError,
    ServoTimeoutError,
)
from servo.types import AssetStatus, Materialization, MaterializationTrigger


class ServoClient:
    """Client for interacting with Servo API."""

    DEFAULT_TIMEOUT = 30.0
    DEFAULT_BASE_URL = "https://api.servo.io"

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        tenant_id: str | None = None,
        timeout: float = DEFAULT_TIMEOUT,
    ) -> None:
        """
        Initialize Servo client.

        Args:
            base_url: Servo API base URL (or SERVO_API_URL env var)
            api_key: API key for authentication (or SERVO_API_KEY env var)
            tenant_id: Tenant ID for multi-tenant isolation (or SERVO_TENANT_ID env var)
            timeout: Request timeout in seconds
        """
        self.base_url = base_url or os.environ.get("SERVO_API_URL", self.DEFAULT_BASE_URL)
        self.api_key = api_key or os.environ.get("SERVO_API_KEY")
        self.tenant_id = tenant_id or os.environ.get("SERVO_TENANT_ID")
        self.timeout = timeout

        if not self.api_key:
            raise ServoConfigError(
                "API key required. Set SERVO_API_KEY environment variable or pass api_key parameter.",
                config_key="api_key",
            )

        if not self.tenant_id:
            raise ServoConfigError(
                "Tenant ID required. Set SERVO_TENANT_ID environment variable or pass tenant_id parameter.",
                config_key="tenant_id",
            )

        self._client = httpx.Client(
            base_url=self.base_url,
            timeout=timeout,
            headers=self._build_headers(),
        )

    def _build_headers(self) -> dict[str, str]:
        """Build request headers."""
        return {
            "Authorization": f"Bearer {self.api_key}",
            "X-Tenant-ID": self.tenant_id or "",
            "Content-Type": "application/json",
            "User-Agent": "servo-python/0.1.0",
        }

    def _handle_response(self, response: httpx.Response) -> dict[str, Any]:
        """Handle API response and raise appropriate errors."""
        try:
            data = response.json()
        except Exception:
            data = {"raw": response.text}

        if response.status_code >= 400:
            error_message = data.get("error", {}).get("message", response.text)
            raise ServoAPIError(
                f"API request failed: {error_message}",
                status_code=response.status_code,
                response_body=data,
            )

        return data

    def _request(
        self,
        method: str,
        path: str,
        **kwargs: Any,
    ) -> dict[str, Any]:
        """Make an API request."""
        try:
            response = self._client.request(method, path, **kwargs)
            return self._handle_response(response)
        except httpx.ConnectError as e:
            raise ServoConnectionError(
                f"Failed to connect to Servo API: {e}",
                url=f"{self.base_url}{path}",
            ) from e
        except httpx.TimeoutException as e:
            raise ServoTimeoutError(
                f"Request timed out: {e}",
                operation=f"{method} {path}",
                timeout_seconds=self.timeout,
            ) from e

    def close(self) -> None:
        """Close the client."""
        self._client.close()

    def __enter__(self) -> ServoClient:
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()

    # Asset Operations

    def list_assets(self) -> list[dict[str, Any]]:
        """List all registered assets."""
        response = self._request("GET", "/api/v1/assets")
        return response.get("assets", [])

    def get_asset(self, name: str) -> dict[str, Any]:
        """Get asset details by name."""
        response = self._request("GET", f"/api/v1/assets/{name}")
        return response.get("asset", {})

    def materialize(
        self,
        asset_name: str,
        partition_key: str | None = None,
        wait: bool = False,
        timeout_seconds: int = 3600,
    ) -> Materialization:
        """
        Trigger materialization of an asset.

        Args:
            asset_name: Name of the asset to materialize
            partition_key: Optional partition key for partitioned assets
            wait: If True, wait for completion
            timeout_seconds: Timeout when waiting

        Returns:
            Materialization record
        """
        payload: dict[str, Any] = {"asset_name": asset_name}
        if partition_key:
            payload["partition_key"] = partition_key

        response = self._request("POST", "/api/v1/materialize", json=payload)
        run_id = response["run_id"]

        materialization = Materialization(
            asset_name=asset_name,
            run_id=run_id,
            status=AssetStatus.PENDING,
            started_at=datetime.utcnow(),
            trigger=MaterializationTrigger.MANUAL,
            partition_key=partition_key,
        )

        if wait:
            return self.wait_for_materialization(run_id, timeout_seconds)

        return materialization

    def wait_for_materialization(
        self,
        run_id: str,
        timeout_seconds: int = 3600,
        poll_interval: float = 2.0,
        max_poll_interval: float = 30.0,
        backoff_factor: float = 1.5,
    ) -> Materialization:
        """
        Wait for a materialization to complete.

        Args:
            run_id: The run ID to wait for
            timeout_seconds: Maximum time to wait
            poll_interval: Initial polling interval in seconds
            max_poll_interval: Maximum polling interval (caps exponential backoff)
            backoff_factor: Multiplier for exponential backoff (1.0 = no backoff)

        Returns:
            Materialization record when complete

        Raises:
            ServoTimeoutError: If not complete within timeout
        """
        import time

        start_time = time.time()
        current_interval = poll_interval

        while time.time() - start_time < timeout_seconds:
            response = self._request("GET", f"/api/v1/runs/{run_id}")
            status = AssetStatus(response["status"])

            if status in (AssetStatus.SUCCESS, AssetStatus.FAILED, AssetStatus.SKIPPED):
                return Materialization(
                    asset_name=response["asset_name"],
                    run_id=run_id,
                    status=status,
                    started_at=datetime.fromisoformat(response["started_at"]),
                    completed_at=datetime.fromisoformat(response["completed_at"])
                    if response.get("completed_at")
                    else None,
                    trigger=MaterializationTrigger(response.get("trigger", "manual")),
                    partition_key=response.get("partition_key"),
                    metadata=response.get("metadata", {}),
                    error=response.get("error"),
                )

            time.sleep(current_interval)
            # Apply exponential backoff, capped at max_poll_interval
            current_interval = min(current_interval * backoff_factor, max_poll_interval)

        raise ServoTimeoutError(
            f"Materialization {run_id} did not complete within {timeout_seconds}s",
            operation="wait_for_materialization",
            timeout_seconds=float(timeout_seconds),
        )

    def get_lineage(self, asset_name: str) -> dict[str, Any]:
        """Get lineage information for an asset."""
        response = self._request("GET", f"/api/v1/assets/{asset_name}/lineage")
        return response

    # Workflow Operations

    def list_workflows(self) -> list[dict[str, Any]]:
        """List all registered workflows."""
        response = self._request("GET", "/api/v1/workflows")
        return response.get("workflows", [])

    def get_workflow(self, name: str) -> dict[str, Any]:
        """Get workflow details by name."""
        response = self._request("GET", f"/api/v1/workflows/{name}")
        return response.get("workflow", {})

    def trigger_workflow(
        self,
        workflow_name: str,
        wait: bool = False,
        timeout_seconds: int = 3600,
    ) -> dict[str, Any]:
        """
        Trigger execution of a workflow.

        Args:
            workflow_name: Name of the workflow to trigger
            wait: If True, wait for completion
            timeout_seconds: Timeout when waiting

        Returns:
            Workflow run information
        """
        response = self._request(
            "POST",
            f"/api/v1/workflows/{workflow_name}/trigger",
        )

        if wait:
            run_id = response["run_id"]
            return self.wait_for_workflow(run_id, timeout_seconds)

        return response

    def wait_for_workflow(
        self,
        run_id: str,
        timeout_seconds: int = 3600,
    ) -> dict[str, Any]:
        """Wait for a workflow run to complete."""
        import time

        start_time = time.time()

        while time.time() - start_time < timeout_seconds:
            response = self._request("GET", f"/api/v1/workflow-runs/{run_id}")
            status = response.get("status", "pending")

            if status in ("success", "failed", "cancelled"):
                return response

            time.sleep(2)

        raise ServoTimeoutError(
            f"Workflow run {run_id} did not complete within {timeout_seconds}s",
            operation="wait_for_workflow",
            timeout_seconds=float(timeout_seconds),
        )

    # Run History

    def list_runs(
        self,
        asset_name: str | None = None,
        workflow_name: str | None = None,
        status: AssetStatus | None = None,
        limit: int = 100,
    ) -> list[dict[str, Any]]:
        """
        List execution runs.

        Args:
            asset_name: Filter by asset name
            workflow_name: Filter by workflow name
            status: Filter by status
            limit: Maximum number of results

        Returns:
            List of run records
        """
        params: dict[str, Any] = {"limit": limit}
        if asset_name:
            params["asset_name"] = asset_name
        if workflow_name:
            params["workflow_name"] = workflow_name
        if status:
            params["status"] = status.value

        response = self._request("GET", "/api/v1/runs", params=params)
        return response.get("runs", [])

    def get_run(self, run_id: str) -> dict[str, Any]:
        """Get details of a specific run."""
        response = self._request("GET", f"/api/v1/runs/{run_id}")
        return response

    # Health Check

    def health(self) -> dict[str, Any]:
        """Check API health."""
        return self._request("GET", "/api/v1/health")


class AsyncServoClient:
    """Async client for interacting with Servo API."""

    DEFAULT_TIMEOUT = 30.0
    DEFAULT_BASE_URL = "https://api.servo.io"

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        tenant_id: str | None = None,
        timeout: float = DEFAULT_TIMEOUT,
    ) -> None:
        """Initialize async Servo client."""
        self.base_url = base_url or os.environ.get("SERVO_API_URL", self.DEFAULT_BASE_URL)
        self.api_key = api_key or os.environ.get("SERVO_API_KEY")
        self.tenant_id = tenant_id or os.environ.get("SERVO_TENANT_ID")
        self.timeout = timeout

        if not self.api_key:
            raise ServoConfigError(
                "API key required. Set SERVO_API_KEY environment variable or pass api_key parameter.",
                config_key="api_key",
            )

        if not self.tenant_id:
            raise ServoConfigError(
                "Tenant ID required. Set SERVO_TENANT_ID environment variable or pass tenant_id parameter.",
                config_key="tenant_id",
            )

        self._client = httpx.AsyncClient(
            base_url=self.base_url,
            timeout=timeout,
            headers=self._build_headers(),
        )

    def _build_headers(self) -> dict[str, str]:
        """Build request headers."""
        return {
            "Authorization": f"Bearer {self.api_key}",
            "X-Tenant-ID": self.tenant_id or "",
            "Content-Type": "application/json",
            "User-Agent": "servo-python/0.1.0",
        }

    async def _handle_response(self, response: httpx.Response) -> dict[str, Any]:
        """Handle API response and raise appropriate errors."""
        try:
            data = response.json()
        except Exception:
            data = {"raw": response.text}

        if response.status_code >= 400:
            error_message = data.get("error", {}).get("message", response.text)
            raise ServoAPIError(
                f"API request failed: {error_message}",
                status_code=response.status_code,
                response_body=data,
            )

        return data

    async def _request(
        self,
        method: str,
        path: str,
        **kwargs: Any,
    ) -> dict[str, Any]:
        """Make an async API request."""
        try:
            response = await self._client.request(method, path, **kwargs)
            return await self._handle_response(response)
        except httpx.ConnectError as e:
            raise ServoConnectionError(
                f"Failed to connect to Servo API: {e}",
                url=f"{self.base_url}{path}",
            ) from e
        except httpx.TimeoutException as e:
            raise ServoTimeoutError(
                f"Request timed out: {e}",
                operation=f"{method} {path}",
                timeout_seconds=self.timeout,
            ) from e

    async def close(self) -> None:
        """Close the client."""
        await self._client.aclose()

    async def __aenter__(self) -> AsyncServoClient:
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.close()

    async def materialize(
        self,
        asset_name: str,
        partition_key: str | None = None,
    ) -> Materialization:
        """Trigger materialization of an asset."""
        payload: dict[str, Any] = {"asset_name": asset_name}
        if partition_key:
            payload["partition_key"] = partition_key

        response = await self._request("POST", "/api/v1/materialize", json=payload)
        run_id = response["run_id"]

        return Materialization(
            asset_name=asset_name,
            run_id=run_id,
            status=AssetStatus.PENDING,
            started_at=datetime.utcnow(),
            trigger=MaterializationTrigger.MANUAL,
            partition_key=partition_key,
        )

    async def health(self) -> dict[str, Any]:
        """Check API health."""
        return await self._request("GET", "/api/v1/health")
