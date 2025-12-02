"""Test module for LocalExecutor integration tests.

This module provides test functions that can be executed by LocalExecutor
to validate Python subprocess execution.
"""

import json
import os
import sys
import time


def success_function():
    """A function that succeeds and returns JSON-compatible output."""
    return {"status": "success", "message": "test passed"}


def env_vars_function():
    """A function that verifies environment variables are passed correctly."""
    execution_id = os.environ.get("SERVO_EXECUTION_ID")
    asset_id = os.environ.get("SERVO_ASSET_ID")
    asset_name = os.environ.get("SERVO_ASSET_NAME")
    tenant_id = os.environ.get("SERVO_TENANT_ID")
    custom_var = os.environ.get("CUSTOM_TEST_VAR")

    return {
        "execution_id": execution_id,
        "asset_id": asset_id,
        "asset_name": asset_name,
        "tenant_id": tenant_id,
        "custom_var": custom_var,
    }


def failure_function():
    """A function that raises an exception."""
    raise ValueError("Intentional test failure")


def slow_function():
    """A function that takes a long time (for timeout testing)."""
    # Sleep for 10 seconds - tests should use a short timeout
    time.sleep(10)
    return {"status": "completed"}


def output_function():
    """A function that produces output on stdout."""
    print("This is stdout output")
    return {"logged": True}


def large_output_function():
    """A function that produces a lot of output (for size limit testing)."""
    # Generate ~100KB of output
    large_data = "x" * 100000
    print(large_data)
    return {"size": len(large_data)}
