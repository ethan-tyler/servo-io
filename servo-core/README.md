# servo-core

Core orchestration engine for Servo, providing workflow compilation, asset management, and execution scheduling.

## Overview

The `servo-core` crate contains the fundamental types and logic for Servo's orchestration system:

- **Assets**: Persistent data artifacts (tables, files, ML models)
- **Workflows**: Collections of assets with defined execution order
- **Compiler**: Validates workflows and generates execution plans
- **Registry**: Maintains a catalog of all assets
- **Scheduler**: Determines when and how workflows execute

## Usage

```rust
use servo_core::{Asset, Workflow, AssetRegistry, WorkflowCompiler};

// Create an asset
let mut asset = Asset::new("customer_data");
asset.metadata.description = Some("Cleaned customer records".to_string());

// Register the asset
let mut registry = AssetRegistry::new();
registry.register(asset.clone())?;

// Create a workflow
let mut workflow = Workflow::new("daily_etl");
workflow.add_asset(asset.id);

// Compile the workflow
let mut compiler = WorkflowCompiler::new();
compiler.register_asset(asset)?;
let plan = compiler.compile(&workflow)?;
```

## Features

- Asset dependency tracking
- Workflow validation and compilation
- Topological sorting of execution order
- Circular dependency detection
- Multi-tenant support

## Testing

```bash
cargo test -p servo-core
```

## Documentation

For more details, see the [main Servo documentation](https://docs.servo.dev).
