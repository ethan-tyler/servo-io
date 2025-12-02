//! Lineage graph data structure

use petgraph::graph::{DiGraph, NodeIndex};
use serde::{Deserialize, Serialize};
use servo_core::AssetId;
use std::collections::HashMap;

/// Node in the lineage graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageNode {
    pub asset_id: AssetId,
    pub name: String,
    pub node_type: String,
}

/// Edge in the lineage graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageEdge {
    pub edge_type: String,
}

/// Lineage graph for tracking data dependencies
pub struct LineageGraph {
    graph: DiGraph<LineageNode, LineageEdge>,
    asset_to_node: HashMap<AssetId, NodeIndex>,
}

impl LineageGraph {
    /// Create a new empty lineage graph
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            asset_to_node: HashMap::new(),
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: LineageNode) -> NodeIndex {
        let asset_id = node.asset_id;
        let idx = self.graph.add_node(node);
        self.asset_to_node.insert(asset_id, idx);
        idx
    }

    /// Add an edge between two nodes
    pub fn add_edge(
        &mut self,
        from: AssetId,
        to: AssetId,
        edge: LineageEdge,
    ) -> Result<(), crate::Error> {
        let from_idx = self
            .asset_to_node
            .get(&from)
            .ok_or_else(|| crate::Error::NodeNotFound(format!("{:?}", from)))?;
        let to_idx = self
            .asset_to_node
            .get(&to)
            .ok_or_else(|| crate::Error::NodeNotFound(format!("{:?}", to)))?;

        self.graph.add_edge(*from_idx, *to_idx, edge);
        Ok(())
    }

    /// Get all upstream dependencies of an asset
    pub fn get_upstream(&self, asset_id: &AssetId) -> Vec<&LineageNode> {
        if let Some(&node_idx) = self.asset_to_node.get(asset_id) {
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Incoming)
                .map(|idx| &self.graph[idx])
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all downstream dependencies of an asset
    pub fn get_downstream(&self, asset_id: &AssetId) -> Vec<&LineageNode> {
        if let Some(&node_idx) = self.asset_to_node.get(asset_id) {
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .map(|idx| &self.graph[idx])
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Check if the graph has cycles
    pub fn has_cycles(&self) -> bool {
        petgraph::algo::is_cyclic_directed(&self.graph)
    }

    /// Get the total number of nodes
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Get the total number of edges
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Get the node index for an asset ID
    pub fn get_node_index(&self, asset_id: &AssetId) -> Option<NodeIndex> {
        self.asset_to_node.get(asset_id).copied()
    }

    /// Get access to the internal graph (for advanced queries)
    pub fn inner_graph(&self) -> &DiGraph<LineageNode, LineageEdge> {
        &self.graph
    }

    /// Get all root nodes (nodes with no incoming edges)
    pub fn roots(&self) -> Vec<AssetId> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                self.graph
                    .neighbors_directed(idx, petgraph::Direction::Incoming)
                    .next()
                    .is_none()
            })
            .map(|idx| self.graph[idx].asset_id)
            .collect()
    }

    /// Get all leaf nodes (nodes with no outgoing edges)
    pub fn leaves(&self) -> Vec<AssetId> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                self.graph
                    .neighbors_directed(idx, petgraph::Direction::Outgoing)
                    .next()
                    .is_none()
            })
            .map(|idx| self.graph[idx].asset_id)
            .collect()
    }

    /// Get a node by its asset ID
    pub fn get_node(&self, asset_id: &AssetId) -> Option<&LineageNode> {
        self.asset_to_node
            .get(asset_id)
            .map(|&idx| &self.graph[idx])
    }
}

impl Default for LineageGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_creation() {
        let graph = LineageGraph::new();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_add_node() {
        let mut graph = LineageGraph::new();
        let asset_id = AssetId::new();

        let node = LineageNode {
            asset_id,
            name: "test_asset".to_string(),
            node_type: "table".to_string(),
        };

        graph.add_node(node);
        assert_eq!(graph.node_count(), 1);
    }

    // =========================================================================
    // Circular Dependency Detection Tests
    // =========================================================================

    /// Helper to create a node with a given name
    fn create_node(name: &str) -> (AssetId, LineageNode) {
        let asset_id = AssetId::new();
        let node = LineageNode {
            asset_id,
            name: name.to_string(),
            node_type: "table".to_string(),
        };
        (asset_id, node)
    }

    #[test]
    fn test_no_cycle_linear_chain() {
        // Graph: A -> B -> C -> D (no cycle)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(!graph.has_cycles(), "Linear chain should not have cycles");
    }

    #[test]
    fn test_no_cycle_branching_dag() {
        // Graph:    A
        //          / \
        //         B   C
        //          \ /
        //           D
        // This is a DAG (no cycles)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                a_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            !graph.has_cycles(),
            "DAG with diamond shape should not have cycles"
        );
    }

    #[test]
    fn test_cycle_simple_triangle() {
        // Graph: A -> B -> C -> A (cycle!)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                a_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            graph.has_cycles(),
            "Triangle A -> B -> C -> A should be detected as a cycle"
        );
    }

    #[test]
    fn test_cycle_self_loop() {
        // Graph: A -> A (self-referential cycle)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        graph.add_node(a_node);

        graph
            .add_edge(
                a_id,
                a_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            graph.has_cycles(),
            "Self-loop A -> A should be detected as a cycle"
        );
    }

    #[test]
    fn test_cycle_two_node() {
        // Graph: A -> B -> A (two-node cycle)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");

        graph.add_node(a_node);
        graph.add_node(b_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                a_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            graph.has_cycles(),
            "Two-node cycle A <-> B should be detected"
        );
    }

    #[test]
    fn test_cycle_in_larger_graph() {
        // Graph: A -> B -> C -> D -> E
        //             ^         |
        //             |_________|
        // (B -> C -> D -> B forms a cycle within a larger graph)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");
        let (e_id, e_node) = create_node("E");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);
        graph.add_node(e_node);

        // Linear part
        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                d_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        // Create the cycle
        graph
            .add_edge(
                d_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            graph.has_cycles(),
            "Cycle B -> C -> D -> B should be detected in larger graph"
        );
    }

    #[test]
    fn test_cycle_isolated_from_rest() {
        // Graph: A -> B (no cycle)
        //        C -> D -> E -> C (isolated cycle)
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");
        let (e_id, e_node) = create_node("E");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);
        graph.add_node(e_node);

        // Isolated acyclic component
        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        // Isolated cyclic component
        graph
            .add_edge(
                c_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                d_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                e_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            graph.has_cycles(),
            "Isolated cycle C -> D -> E -> C should be detected even with acyclic components"
        );
    }

    #[test]
    fn test_multiple_disconnected_dags_no_cycle() {
        // Graph: A -> B -> C (DAG 1)
        //        D -> E -> F (DAG 2)
        // No cycles
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");
        let (e_id, e_node) = create_node("E");
        let (f_id, f_node) = create_node("F");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);
        graph.add_node(e_node);
        graph.add_node(f_node);

        // DAG 1
        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        // DAG 2
        graph
            .add_edge(
                d_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                e_id,
                f_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            !graph.has_cycles(),
            "Multiple disconnected DAGs should not report cycles"
        );
    }

    #[test]
    fn test_no_cycle_empty_graph() {
        let graph = LineageGraph::new();
        assert!(!graph.has_cycles(), "Empty graph should not have cycles");
    }

    #[test]
    fn test_no_cycle_single_node() {
        let mut graph = LineageGraph::new();
        let (_a_id, a_node) = create_node("A");
        graph.add_node(a_node);

        assert!(
            !graph.has_cycles(),
            "Single node without edges should not have cycles"
        );
    }

    #[test]
    fn test_cycle_long_chain_back_to_start() {
        // Graph: A -> B -> C -> D -> E -> F -> G -> H -> A
        // (8-node cycle)
        let mut graph = LineageGraph::new();

        let nodes: Vec<(AssetId, LineageNode)> =
            (0..8).map(|i| create_node(&format!("Node{}", i))).collect();

        for (_, node) in &nodes {
            graph.add_node(node.clone());
        }

        // Create chain
        for i in 0..7 {
            graph
                .add_edge(
                    nodes[i].0,
                    nodes[i + 1].0,
                    LineageEdge {
                        edge_type: "data".to_string(),
                    },
                )
                .unwrap();
        }

        // Close the cycle
        graph
            .add_edge(
                nodes[7].0,
                nodes[0].0,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(graph.has_cycles(), "8-node cycle should be detected");
    }

    #[test]
    fn test_complex_dag_with_multiple_paths_no_cycle() {
        // Complex DAG with multiple paths but no cycles
        //        A
        //       /|\
        //      B C D
        //      |X|/
        //      E F
        //       \|
        //        G
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");
        let (e_id, e_node) = create_node("E");
        let (f_id, f_node) = create_node("F");
        let (g_id, g_node) = create_node("G");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);
        graph.add_node(e_node);
        graph.add_node(f_node);
        graph.add_node(g_node);

        // From A
        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                a_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                a_id,
                d_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        // Cross connections
        graph
            .add_edge(
                b_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                f_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c_id,
                f_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                d_id,
                f_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        // To G
        graph
            .add_edge(
                e_id,
                g_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                f_id,
                g_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        assert!(
            !graph.has_cycles(),
            "Complex DAG with multiple paths but no back-edges should not have cycles"
        );
    }

    #[test]
    fn test_roots_and_leaves() {
        // Graph: A -> B -> C
        //        D -> E -> C
        // Roots: A, D
        // Leaves: C
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");
        let (d_id, d_node) = create_node("D");
        let (e_id, e_node) = create_node("E");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);
        graph.add_node(d_node);
        graph.add_node(e_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                d_id,
                e_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                e_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let roots = graph.roots();
        let leaves = graph.leaves();

        assert_eq!(roots.len(), 2, "Should have 2 roots (A and D)");
        assert!(roots.contains(&a_id), "A should be a root");
        assert!(roots.contains(&d_id), "D should be a root");

        assert_eq!(leaves.len(), 1, "Should have 1 leaf (C)");
        assert!(leaves.contains(&c_id), "C should be a leaf");
    }

    #[test]
    fn test_upstream_downstream() {
        // Graph: A -> B -> C
        let mut graph = LineageGraph::new();

        let (a_id, a_node) = create_node("A");
        let (b_id, b_node) = create_node("B");
        let (c_id, c_node) = create_node("C");

        graph.add_node(a_node);
        graph.add_node(b_node);
        graph.add_node(c_node);

        graph
            .add_edge(
                a_id,
                b_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b_id,
                c_id,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        // Test upstream (incoming edges)
        let b_upstream = graph.get_upstream(&b_id);
        assert_eq!(b_upstream.len(), 1);
        assert_eq!(b_upstream[0].asset_id, a_id);

        let c_upstream = graph.get_upstream(&c_id);
        assert_eq!(c_upstream.len(), 1);
        assert_eq!(c_upstream[0].asset_id, b_id);

        // Test downstream (outgoing edges)
        let a_downstream = graph.get_downstream(&a_id);
        assert_eq!(a_downstream.len(), 1);
        assert_eq!(a_downstream[0].asset_id, b_id);

        let b_downstream = graph.get_downstream(&b_id);
        assert_eq!(b_downstream.len(), 1);
        assert_eq!(b_downstream[0].asset_id, c_id);
    }
}
