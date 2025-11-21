//! Impact analysis for lineage changes

use crate::graph::LineageGraph;
use servo_core::AssetId;

/// Impact analysis for lineage changes
pub struct ImpactAnalysis<'a> {
    graph: &'a LineageGraph,
}

impl<'a> ImpactAnalysis<'a> {
    /// Create a new impact analysis for the given graph
    pub fn new(graph: &'a LineageGraph) -> Self {
        Self { graph }
    }

    /// Analyze the impact of changing an asset
    pub fn analyze_change(&self, asset_id: &AssetId) -> ChangeImpact {
        let affected_downstream = self.graph.get_downstream(asset_id);

        ChangeImpact {
            source_asset: *asset_id,
            affected_count: affected_downstream.len(),
            affected_assets: affected_downstream.iter().map(|n| n.asset_id).collect(),
        }
    }
}

/// Result of an impact analysis
#[derive(Debug)]
pub struct ChangeImpact {
    /// The asset that was changed
    pub source_asset: AssetId,

    /// Number of affected assets
    pub affected_count: usize,

    /// List of affected asset IDs
    pub affected_assets: Vec<AssetId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impact_analysis() {
        let graph = LineageGraph::new();
        let _analysis = ImpactAnalysis::new(&graph);
    }
}
