"""
Simple ETL Pipeline Example

This example demonstrates how to define assets and workflows
using the Servo Python SDK.
"""

from servo import asset, workflow

# Define a source asset (no dependencies)
@asset(
    name="raw_orders",
    description="Raw orders from the e-commerce database",
    metadata={
        "owner": "data-platform",
        "team": "analytics",
        "tags": ["production", "pii"],
    },
    is_source=True,
)
def raw_orders() -> list[dict]:
    """Extract raw order data from the source system."""
    # In a real scenario, this would query a database or API
    return [
        {"order_id": "001", "customer_id": "C1", "amount": 100.0, "status": "completed"},
        {"order_id": "002", "customer_id": "C2", "amount": 250.0, "status": "pending"},
        {"order_id": "003", "customer_id": "C1", "amount": 75.0, "status": "completed"},
    ]


@asset(
    name="raw_customers",
    description="Raw customer data from the CRM",
    metadata={"owner": "data-platform"},
    is_source=True,
)
def raw_customers() -> list[dict]:
    """Extract customer data from CRM."""
    return [
        {"customer_id": "C1", "name": "Alice", "segment": "premium"},
        {"customer_id": "C2", "name": "Bob", "segment": "standard"},
    ]


# Define a derived asset with dependencies
@asset(
    name="completed_orders",
    dependencies=["raw_orders"],
    description="Filtered orders that have been completed",
    metadata={"owner": "analytics-team"},
)
def completed_orders(raw_orders: list[dict]) -> list[dict]:
    """Filter to only completed orders."""
    return [order for order in raw_orders if order["status"] == "completed"]


@asset(
    name="customer_order_summary",
    dependencies=["completed_orders", "raw_customers"],
    description="Aggregated order statistics per customer",
    metadata={
        "owner": "analytics-team",
        "tags": ["reporting", "dashboard"],
    },
)
def customer_order_summary(
    completed_orders: list[dict],
    raw_customers: list[dict],
) -> list[dict]:
    """Aggregate orders by customer."""
    # Build customer lookup
    customer_map = {c["customer_id"]: c for c in raw_customers}

    # Aggregate orders
    summaries: dict[str, dict] = {}
    for order in completed_orders:
        cid = order["customer_id"]
        if cid not in summaries:
            customer = customer_map.get(cid, {})
            summaries[cid] = {
                "customer_id": cid,
                "customer_name": customer.get("name", "Unknown"),
                "segment": customer.get("segment", "unknown"),
                "total_orders": 0,
                "total_amount": 0.0,
            }
        summaries[cid]["total_orders"] += 1
        summaries[cid]["total_amount"] += order["amount"]

    return list(summaries.values())


# Define a workflow that executes assets in order
@workflow(
    name="daily_orders_etl",
    schedule="0 6 * * *",  # Run daily at 6 AM
    description="Daily ETL pipeline for order analytics",
    timeout_seconds=1800,
    retries=2,
)
def daily_orders_etl():
    """Execute the daily orders ETL pipeline."""
    return [
        raw_orders,
        raw_customers,
        completed_orders,
        customer_order_summary,
    ]


# Example of running assets locally
if __name__ == "__main__":
    print("=== Running Simple ETL Example ===\n")

    # Execute assets manually
    orders_data = raw_orders()
    print(f"Raw Orders: {len(orders_data)} records")

    customers_data = raw_customers()
    print(f"Raw Customers: {len(customers_data)} records")

    completed = completed_orders(orders_data)
    print(f"Completed Orders: {len(completed)} records")

    summary = customer_order_summary(completed, customers_data)
    print(f"\nCustomer Order Summary:")
    for record in summary:
        print(f"  {record['customer_name']}: {record['total_orders']} orders, ${record['total_amount']:.2f}")

    # Show workflow assets
    print(f"\n=== Workflow: {daily_orders_etl.name} ===")
    print(f"Schedule: {daily_orders_etl.schedule}")
    print(f"Assets: {daily_orders_etl.assets}")
