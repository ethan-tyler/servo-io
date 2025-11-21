# servo-lineage

Lineage graph operations for tracking data dependencies and impact analysis.

## Overview

The `servo-lineage` crate provides tools for managing and querying data lineage:

- **Graph**: Directed graph structure for asset dependencies
- **Queries**: Query upstream/downstream dependencies, find paths
- **Impact Analysis**: Analyze the effect of changes on downstream assets

## Usage

```rust
use servo_lineage::{LineageGraph, LineageNode, LineageEdge, LineageQuery};

// Create a lineage graph
let mut graph = LineageGraph::new();

// Add nodes
let node1 = LineageNode {
    asset_id: asset_id_1,
    name: "raw_data".to_string(),
    node_type: "table".to_string(),
};
graph.add_node(node1);

// Add edges
let edge = LineageEdge {
    edge_type: "transforms".to_string(),
};
graph.add_edge(asset_id_1, asset_id_2, edge)?;

// Query lineage
let query = LineageQuery::new(&graph);
let upstream = query.ancestors(&asset_id_2);
let downstream = query.descendants(&asset_id_1);

// Impact analysis
let analysis = ImpactAnalysis::new(&graph);
let impact = analysis.analyze_change(&asset_id_1);
```

## Features

- Directed acyclic graph (DAG) for lineage
- Cycle detection
- Upstream/downstream queries
- Path finding between assets
- Impact analysis for changes

## Testing

```bash
cargo test -p servo-lineage
```

## Documentation

For more details, see the [main Servo documentation](https://docs.servo.dev).
