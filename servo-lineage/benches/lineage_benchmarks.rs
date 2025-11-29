//! Criterion benchmarks for lineage graph operations
//!
//! These benchmarks measure the performance of lineage operations
//! at various graph sizes to ensure acceptable scaling characteristics.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use servo_core::AssetId;
use servo_lineage::graph::{LineageEdge, LineageGraph, LineageNode};
use servo_lineage::impact::ImpactAnalysis;
use servo_lineage::queries::LineageQuery;

/// Create a chain graph: A -> B -> C -> ... -> N
fn create_chain_graph(size: usize) -> (LineageGraph, Vec<AssetId>) {
    let mut graph = LineageGraph::new();
    let mut asset_ids = Vec::with_capacity(size);

    for i in 0..size {
        let asset_id = AssetId::new();
        asset_ids.push(asset_id);

        graph.add_node(LineageNode {
            asset_id,
            name: format!("asset_{}", i),
            node_type: "table".to_string(),
        });

        if i > 0 {
            graph
                .add_edge(
                    asset_ids[i - 1],
                    asset_id,
                    LineageEdge {
                        edge_type: "produces".to_string(),
                    },
                )
                .unwrap();
        }
    }

    (graph, asset_ids)
}

/// Create a fan-out graph: one source with N targets
fn create_fan_out_graph(fan_out: usize) -> (LineageGraph, AssetId, Vec<AssetId>) {
    let mut graph = LineageGraph::new();
    let source_id = AssetId::new();
    let mut target_ids = Vec::with_capacity(fan_out);

    graph.add_node(LineageNode {
        asset_id: source_id,
        name: "source".to_string(),
        node_type: "table".to_string(),
    });

    for i in 0..fan_out {
        let target_id = AssetId::new();
        target_ids.push(target_id);

        graph.add_node(LineageNode {
            asset_id: target_id,
            name: format!("target_{}", i),
            node_type: "table".to_string(),
        });

        graph
            .add_edge(
                source_id,
                target_id,
                LineageEdge {
                    edge_type: "produces".to_string(),
                },
            )
            .unwrap();
    }

    (graph, source_id, target_ids)
}

/// Create a diamond graph with multiple layers
/// Layer 0: 1 source
/// Layer 1..N-1: width nodes each, all connected to all in previous layer
/// Layer N: 1 sink
fn create_diamond_graph(layers: usize, width: usize) -> (LineageGraph, AssetId, AssetId) {
    let mut graph = LineageGraph::new();

    // Source node
    let source_id = AssetId::new();
    graph.add_node(LineageNode {
        asset_id: source_id,
        name: "source".to_string(),
        node_type: "table".to_string(),
    });

    let mut prev_layer = vec![source_id];

    // Middle layers
    for layer in 1..layers {
        let mut current_layer = Vec::with_capacity(width);

        for i in 0..width {
            let asset_id = AssetId::new();
            current_layer.push(asset_id);

            graph.add_node(LineageNode {
                asset_id,
                name: format!("layer{}_{}", layer, i),
                node_type: "table".to_string(),
            });

            // Connect to all nodes in previous layer
            for &prev_id in &prev_layer {
                graph
                    .add_edge(
                        prev_id,
                        asset_id,
                        LineageEdge {
                            edge_type: "produces".to_string(),
                        },
                    )
                    .unwrap();
            }
        }

        prev_layer = current_layer;
    }

    // Sink node
    let sink_id = AssetId::new();
    graph.add_node(LineageNode {
        asset_id: sink_id,
        name: "sink".to_string(),
        node_type: "table".to_string(),
    });

    for &prev_id in &prev_layer {
        graph
            .add_edge(
                prev_id,
                sink_id,
                LineageEdge {
                    edge_type: "produces".to_string(),
                },
            )
            .unwrap();
    }

    (graph, source_id, sink_id)
}

fn bench_graph_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_construction");

    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("chain", size), size, |b, &size| {
            b.iter(|| {
                let (graph, _) = create_chain_graph(size);
                black_box(graph)
            });
        });

        group.bench_with_input(BenchmarkId::new("fan_out", size), size, |b, &size| {
            b.iter(|| {
                let (graph, _, _) = create_fan_out_graph(size);
                black_box(graph)
            });
        });
    }

    group.finish();
}

fn bench_upstream_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("upstream_queries");

    for size in [10, 100, 500].iter() {
        // Create chain graph and query the last node (has most ancestors)
        let (graph, asset_ids) = create_chain_graph(*size);
        let query = LineageQuery::new(&graph);
        let last_id = asset_ids[size - 1];

        group.bench_with_input(
            BenchmarkId::new("chain_last_node", size),
            &last_id,
            |b, id| {
                b.iter(|| {
                    let ancestors = query.ancestors(black_box(id));
                    black_box(ancestors)
                });
            },
        );

        // Query middle node
        let mid_id = asset_ids[size / 2];
        group.bench_with_input(
            BenchmarkId::new("chain_mid_node", size),
            &mid_id,
            |b, id| {
                b.iter(|| {
                    let ancestors = query.ancestors(black_box(id));
                    black_box(ancestors)
                });
            },
        );
    }

    // Fan-in query (many sources -> one target)
    for fan_size in [10, 100, 500].iter() {
        let (graph, _, targets) = create_fan_out_graph(*fan_size);
        let query = LineageQuery::new(&graph);
        // Query first target
        let target_id = targets[0];

        group.bench_with_input(
            BenchmarkId::new("single_upstream", fan_size),
            &target_id,
            |b, id| {
                b.iter(|| {
                    let ancestors = query.ancestors(black_box(id));
                    black_box(ancestors)
                });
            },
        );
    }

    group.finish();
}

fn bench_downstream_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("downstream_queries");

    // Fan-out queries (one source -> many targets)
    for fan_size in [10, 100, 500].iter() {
        let (graph, source_id, _) = create_fan_out_graph(*fan_size);
        let query = LineageQuery::new(&graph);

        group.bench_with_input(
            BenchmarkId::new("fan_out", fan_size),
            &source_id,
            |b, id| {
                b.iter(|| {
                    let descendants = query.descendants(black_box(id));
                    black_box(descendants)
                });
            },
        );
    }

    // Chain queries
    for size in [10, 100, 500].iter() {
        let (graph, asset_ids) = create_chain_graph(*size);
        let query = LineageQuery::new(&graph);
        let first_id = asset_ids[0];

        group.bench_with_input(BenchmarkId::new("chain_first", size), &first_id, |b, id| {
            b.iter(|| {
                let descendants = query.descendants(black_box(id));
                black_box(descendants)
            });
        });
    }

    group.finish();
}

fn bench_cycle_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("cycle_detection");

    // Chain graphs (no cycles)
    for size in [10, 100, 500, 1000].iter() {
        let (graph, _) = create_chain_graph(*size);

        group.bench_with_input(BenchmarkId::new("chain_no_cycle", size), &graph, |b, g| {
            b.iter(|| {
                let has_cycle = g.has_cycles();
                black_box(has_cycle)
            });
        });
    }

    // Diamond graphs (no cycles but more complex)
    for size in [3, 5, 7].iter() {
        let (graph, _, _) = create_diamond_graph(*size, 10);

        group.bench_with_input(BenchmarkId::new("diamond_layers", size), &graph, |b, g| {
            b.iter(|| {
                let has_cycle = g.has_cycles();
                black_box(has_cycle)
            });
        });
    }

    group.finish();
}

fn bench_impact_analysis(c: &mut Criterion) {
    let mut group = c.benchmark_group("impact_analysis");

    // Fan-out impact (changing source affects many)
    for fan_size in [10, 100, 500].iter() {
        let (graph, source_id, _) = create_fan_out_graph(*fan_size);
        let analysis = ImpactAnalysis::new(&graph);

        group.bench_with_input(
            BenchmarkId::new("fan_out_source", fan_size),
            &source_id,
            |b, id| {
                b.iter(|| {
                    let impact = analysis.analyze_change(black_box(id));
                    black_box(impact)
                });
            },
        );
    }

    // Chain impact (changing first node)
    for size in [10, 100, 500].iter() {
        let (graph, asset_ids) = create_chain_graph(*size);
        let analysis = ImpactAnalysis::new(&graph);
        let first_id = asset_ids[0];

        group.bench_with_input(BenchmarkId::new("chain_first", size), &first_id, |b, id| {
            b.iter(|| {
                let impact = analysis.analyze_change(black_box(id));
                black_box(impact)
            });
        });
    }

    // Diamond graph impact
    for layers in [3, 5, 7].iter() {
        let (graph, source_id, _) = create_diamond_graph(*layers, 10);
        let analysis = ImpactAnalysis::new(&graph);

        group.bench_with_input(
            BenchmarkId::new("diamond_source", layers),
            &source_id,
            |b, id| {
                b.iter(|| {
                    let impact = analysis.analyze_change(black_box(id));
                    black_box(impact)
                });
            },
        );
    }

    group.finish();
}

fn bench_graph_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("graph_operations");

    // Node addition
    group.bench_function("add_single_node", |b| {
        b.iter(|| {
            let mut graph = LineageGraph::new();
            let asset_id = AssetId::new();
            graph.add_node(LineageNode {
                asset_id,
                name: "test".to_string(),
                node_type: "table".to_string(),
            });
            black_box(graph)
        });
    });

    // Edge addition
    group.bench_function("add_edge", |b| {
        b.iter_batched(
            || {
                let mut graph = LineageGraph::new();
                let id1 = AssetId::new();
                let id2 = AssetId::new();
                graph.add_node(LineageNode {
                    asset_id: id1,
                    name: "source".to_string(),
                    node_type: "table".to_string(),
                });
                graph.add_node(LineageNode {
                    asset_id: id2,
                    name: "target".to_string(),
                    node_type: "table".to_string(),
                });
                (graph, id1, id2)
            },
            |(mut graph, id1, id2)| {
                graph
                    .add_edge(
                        id1,
                        id2,
                        LineageEdge {
                            edge_type: "produces".to_string(),
                        },
                    )
                    .unwrap();
                black_box(graph)
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Node count (should be O(1))
    for size in [100, 1000, 10000].iter() {
        let (graph, _) = create_chain_graph(*size);

        group.bench_with_input(BenchmarkId::new("node_count", size), &graph, |b, g| {
            b.iter(|| {
                let count = g.node_count();
                black_box(count)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_graph_construction,
    bench_upstream_queries,
    bench_downstream_queries,
    bench_cycle_detection,
    bench_impact_analysis,
    bench_graph_operations,
);

criterion_main!(benches);
