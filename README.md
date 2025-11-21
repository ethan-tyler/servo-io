# Servo

<p align="center">
  <strong>Serverless Data Orchestration with Asset Lineage</strong>
</p>

<p align="center">
  <a href="https://github.com/ethan-tyler/servo-io/actions"><img src="https://github.com/ethan-tyler/servo-io/workflows/CI/badge.svg" alt="CI Status"></a>
  <a href="https://github.com/ethan-tyler/servo-io/actions/workflows/coverage.yml"><img src="https://codecov.io/gh/ethan-tyler/servo-io/branch/main/graph/badge.svg" alt="Code Coverage"></a>
  <a href="https://github.com/ethan-tyler/servo-io/actions/workflows/deny.yml"><img src="https://github.com/ethan-tyler/servo-io/workflows/License%20%26%20Security%20Check/badge.svg" alt="Security"></a>
  <a href="https://crates.io/crates/servo-core"><img src="https://img.shields.io/crates/v/servo-core.svg" alt="Crates.io"></a>
  <a href="https://pypi.org/project/servo-orchestration/"><img src="https://img.shields.io/pypi/v/servo-orchestration.svg" alt="PyPI"></a>
  <a href="https://docs.servo.dev"><img src="https://img.shields.io/badge/docs-latest-blue.svg" alt="Documentation"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License"></a>
  <a href="https://github.com/ethan-tyler/servo-io/releases"><img src="https://img.shields.io/github/v/release/ethan-tyler/servo-io?include_prereleases" alt="GitHub Release"></a>
  <a href="https://github.com/ethan-tyler/servo-io/blob/main/.github/CONTRIBUTING.md"><img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg" alt="PRs Welcome"></a>
</p>

---

## What is Servo?

**Servo** is an open-source, asset-centric data orchestration platform designed for serverless compute environments and multi-tenant SaaS applications. Built with a high-performance Rust core and ergonomic Python SDK, Servo delivers enterprise-grade orchestration features, including comprehensive lineage tracking, observability, retry logic, and versioning, without requiring persistent infrastructure.

### Key Features

- **Asset-first orchestration**: Data assets are first-class citizens with automatic lineage tracking.
- **Serverless-native**: No persistent schedulers; uses cloud queuing services (Cloud Tasks, SQS, EventBridge).
- **Multi-tenant by design**: Tenant isolation, cost attribution, and protection against noisy neighbors.
- **Zero idle costs**: Scales to zero when inactive (unlike Airflow, Dagster, or Prefect).
- **Cloud-agnostic**: Works across GCP, AWS, and Azure with pluggable adapters.
- **High performance**: Rust core provides 10-100x performance improvements for critical paths.

### Quick Start

```bash
# Install Servo
pip install servo-orchestration

# Initialize metadata database
servo init --database-url postgresql://localhost/servo

# Create a workflow
cat > workflow.py << 'EOF'
from servo import asset, workflow, executor

@asset(name="clean_customers")
def clean_customers(raw_data):
    return raw_data.drop_duplicates()

@workflow(
    name="daily_etl",
    executor=executor.CloudRun(project="my-project")
)
def pipeline():
    raw = extract_data()
    clean = clean_customers(raw)
    return clean
EOF

# Deploy workflow
servo deploy workflow.py

# Trigger execution
servo run daily_etl
```

### Documentation

- [Getting Started](https://docs.servo.dev/getting-started)
- [Concepts](https://docs.servo.dev/concepts)
- [API Reference](https://docs.servo.dev/api-reference)
- [Deployment Guides](https://docs.servo.dev/guides)

### Why Servo?

**Problem**: Modern data teams use serverless compute (Cloud Run, Lambda, ECS) but orchestration tools still require persistent infrastructure with fixed costs.

**Solution**: Servo orchestrates workflows using cloud-native queuing services and stores execution metadata in standard relational databases. No persistent schedulers, no daemons, and no fixed costs.

**Comparison**:

| Feature | Servo | Dagster | Airflow | Step Functions |
|---------|-------|---------|---------|----------------|
| Persistent processes | None | Required | Required | None |
| Asset lineage | Built-in | Core | Plugin-based | Not available |
| Multi-tenant (first-class) | Yes | Partial | No | Partial |
| Zero idle costs | Yes | No | No | Yes |
| Cloud-agnostic | Yes | Yes | Yes | No |

### Architecture

```
User Code (Python) -> Servo SDK -> Rust Core -> Cloud Queue -> Worker -> PostgreSQL
                                                         |
                                                         -> Cloud Storage
```

### Community

- **Discord**: [Join our Discord](https://discord.gg/servo)
- **GitHub Discussions**: [Ask questions](https://github.com/servo-orchestration/servo/discussions)
- **Twitter**: [@ServoOrch](https://twitter.com/ServoOrch)

### Contributing

We welcome contributions! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### License

Apache 2.0 - See [LICENSE](LICENSE) for details.

### Roadmap

See [ROADMAP.md](ROADMAP.md) for our development plans.

---

Built with care by the Servo community.
