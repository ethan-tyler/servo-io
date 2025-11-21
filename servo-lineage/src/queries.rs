//! Lineage query operations

use crate::graph::LineageGraph;
use servo_core::AssetId;

/// Query builder for lineage operations
pub struct LineageQuery<'a> {
    graph: &'a LineageGraph,
}

impl<'a> LineageQuery<'a> {
    /// Create a new query for the given graph
    pub fn new(graph: &'a LineageGraph) -> Self {
        Self { graph }
    }

    /// Find all ancestors (upstream dependencies) of an asset
    pub fn ancestors(&self, asset_id: &AssetId) -> Vec<AssetId> {
        self.graph
            .get_upstream(asset_id)
            .iter()
            .map(|node| node.asset_id)
            .collect()
    }

    /// Find all descendants (downstream dependencies) of an asset
    pub fn descendants(&self, asset_id: &AssetId) -> Vec<AssetId> {
        self.graph
            .get_downstream(asset_id)
            .iter()
            .map(|node| node.asset_id)
            .collect()
    }

    /// Find the path between two assets
    pub fn path(&self, _from: &AssetId, _to: &AssetId) -> Option<Vec<AssetId>> {
        // TODO: Implement pathfinding algorithm
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_creation() {
        let graph = LineageGraph::new();
        let _query = LineageQuery::new(&graph);
    }
}
