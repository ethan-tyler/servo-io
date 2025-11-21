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
}
