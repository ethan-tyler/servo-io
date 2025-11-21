# Contributing to Servo

Thank you for your interest in contributing to Servo! We welcome contributions from the community.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [Code Style](#code-style)
- [Testing](#testing)
- [Documentation](#documentation)

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

1. **Fork the repository** on GitHub
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/servo.git
   cd servo
   ```
3. **Add upstream remote**:
   ```bash
   git remote add upstream https://github.com/servo-orchestration/servo.git
   ```

## Development Setup

### Prerequisites

- **Rust 1.75+**: Install via [rustup](https://rustup.rs/)
- **Python 3.9+**: Install via [python.org](https://www.python.org/)
- **PostgreSQL 15+**: Install via your package manager or [Docker](#using-docker)
- **Docker** (optional but recommended)

### Setup Steps

```bash
# Install Rust dependencies
cargo build

# Install Python dependencies
cd servo-python
python -m venv .venv
source .venv/bin/activate  # On Windows: .venv\Scripts\activate
pip install -r requirements-dev.txt
pip install -e .

# Setup PostgreSQL (via Docker)
docker run -d \
  --name servo-postgres \
  -e POSTGRES_PASSWORD=servo \
  -e POSTGRES_USER=servo \
  -e POSTGRES_DB=servo_dev \
  -p 5432:5432 \
  postgres:15

# Run database migrations
cargo run --bin servo-cli -- migrate up

# Run tests
cargo test
cd servo-python && pytest
```

## Making Changes

### Creating a Branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/your-bug-fix
```

### Commit Messages

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
feat: add support for Azure Queue Storage
fix: resolve race condition in task scheduling
docs: update deployment guide for AWS
test: add integration tests for lineage queries
```

## Submitting a Pull Request

1. **Push your branch** to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

2. **Open a Pull Request** on GitHub against the `develop` branch

3. **Fill out the PR template** with all required information

4. **Wait for review** - maintainers will review your PR and may request changes

5. **Address feedback** - make requested changes and push new commits

6. **Merge** - once approved, a maintainer will merge your PR

## Code Style

### Rust

- Follow the [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)
- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Keep functions small and focused
- Add documentation comments for public APIs

### Python

- Follow [PEP 8](https://www.python.org/dev/peps/pep-0008/)
- Use `black` for formatting: `black .`
- Use `ruff` for linting: `ruff check .`
- Add type hints to all functions
- Write docstrings for public functions

## Testing

### Running Tests

```bash
# Rust tests
cargo test --all-features

# Python tests
cd servo-python
pytest -v

# Integration tests
cargo test --test integration

# Specific test
cargo test test_workflow_compilation
```

### Writing Tests

- Write unit tests for all new functions
- Write integration tests for end-to-end workflows
- Use meaningful test names: `test_asset_dependency_resolution`
- Mock external services (cloud APIs, databases)

### Test Coverage

We aim for >80% code coverage. Check coverage with:

```bash
# Rust
cargo tarpaulin --out Html

# Python
pytest --cov=servo --cov-report=html
```

## Documentation

### Code Documentation

- Add rustdoc comments to all public Rust APIs:
  ```rust
  /// Executes a workflow and returns the execution ID.
  ///
  /// # Arguments
  ///
  /// * `workflow` - The workflow to execute
  /// * `params` - Execution parameters
  ///
  /// # Returns
  ///
  /// The UUID of the created execution
  pub fn execute_workflow(workflow: &Workflow, params: Params) -> Result<Uuid>
  ```

- Add docstrings to all Python functions:
  ```python
  def execute_workflow(workflow: Workflow, params: dict) -> str:
      """Execute a workflow and return the execution ID.

      Args:
          workflow: The workflow to execute
          params: Execution parameters

      Returns:
          The UUID of the created execution

      Raises:
          ServoError: If execution fails
      """
  ```

### User Documentation

- Update relevant docs in `docs/src/`
- Add examples to `examples/`
- Update README.md if adding major features

## Questions?

- Open a [GitHub Discussion](https://github.com/servo-orchestration/servo/discussions)
- Join our [Discord](https://discord.gg/servo)
- Email: team@servo.dev

Thank you for contributing to Servo!
