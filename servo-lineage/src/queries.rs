//! Lineage query operations
//!
//! This module provides query capabilities for traversing and analyzing the lineage graph.
//! It supports:
//! - Finding ancestors (upstream dependencies)
//! - Finding descendants (downstream dependencies)
//! - Path finding between assets
//! - Depth-limited traversals
//! - Common ancestor/descendant analysis

use crate::graph::LineageGraph;
use petgraph::graph::NodeIndex;
use petgraph::Direction;
use servo_core::AssetId;
use std::collections::{HashSet, VecDeque};

/// Query builder for lineage operations
pub struct LineageQuery<'a> {
    graph: &'a LineageGraph,
}

impl<'a> LineageQuery<'a> {
    /// Create a new query for the given graph
    pub fn new(graph: &'a LineageGraph) -> Self {
        Self { graph }
    }

    /// Find all direct ancestors (upstream dependencies) of an asset
    pub fn ancestors(&self, asset_id: &AssetId) -> Vec<AssetId> {
        self.graph
            .get_upstream(asset_id)
            .iter()
            .map(|node| node.asset_id)
            .collect()
    }

    /// Find all direct descendants (downstream dependencies) of an asset
    pub fn descendants(&self, asset_id: &AssetId) -> Vec<AssetId> {
        self.graph
            .get_downstream(asset_id)
            .iter()
            .map(|node| node.asset_id)
            .collect()
    }

    /// Find all ancestors (upstream) up to a maximum depth
    ///
    /// A depth of 0 returns no ancestors, depth of 1 returns direct ancestors,
    /// depth of 2 includes ancestors of ancestors, etc.
    ///
    /// # Arguments
    /// * `asset_id` - The starting asset
    /// * `max_depth` - Maximum traversal depth (None for unlimited)
    ///
    /// # Returns
    /// A vector of (AssetId, depth) tuples sorted by depth (closest first)
    pub fn ancestors_with_depth(
        &self,
        asset_id: &AssetId,
        max_depth: Option<usize>,
    ) -> Vec<(AssetId, usize)> {
        self.traverse_with_depth(asset_id, Direction::Incoming, max_depth)
    }

    /// Find all descendants (downstream) up to a maximum depth
    ///
    /// # Arguments
    /// * `asset_id` - The starting asset
    /// * `max_depth` - Maximum traversal depth (None for unlimited)
    ///
    /// # Returns
    /// A vector of (AssetId, depth) tuples sorted by depth (closest first)
    pub fn descendants_with_depth(
        &self,
        asset_id: &AssetId,
        max_depth: Option<usize>,
    ) -> Vec<(AssetId, usize)> {
        self.traverse_with_depth(asset_id, Direction::Outgoing, max_depth)
    }

    /// Get full lineage information for an asset
    ///
    /// Returns both ancestors and descendants with their metadata
    pub fn full_lineage(&self, asset_id: &AssetId) -> LineageInfo {
        let upstream = self.ancestors_with_depth(asset_id, None);
        let downstream = self.descendants_with_depth(asset_id, None);

        let max_upstream_depth = upstream.iter().map(|(_, d)| *d).max().unwrap_or(0);
        let max_downstream_depth = downstream.iter().map(|(_, d)| *d).max().unwrap_or(0);

        LineageInfo {
            asset_id: *asset_id,
            upstream_count: upstream.len(),
            downstream_count: downstream.len(),
            upstream_assets: upstream,
            downstream_assets: downstream,
            max_upstream_depth,
            max_downstream_depth,
        }
    }

    /// Find the shortest path between two assets
    ///
    /// Returns None if no path exists. The path is returned as a sequence of asset IDs
    /// from `from` to `to`, inclusive.
    pub fn path(&self, from: &AssetId, to: &AssetId) -> Option<Vec<AssetId>> {
        let graph = self.graph.inner_graph();

        let from_idx = self.graph.get_node_index(from)?;
        let to_idx = self.graph.get_node_index(to)?;

        // Use BFS to find shortest path
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        let mut parent: std::collections::HashMap<NodeIndex, NodeIndex> =
            std::collections::HashMap::new();

        queue.push_back(from_idx);
        visited.insert(from_idx);

        while let Some(current) = queue.pop_front() {
            if current == to_idx {
                // Reconstruct path
                let mut path = vec![graph[to_idx].asset_id];
                let mut curr = to_idx;
                while let Some(&prev) = parent.get(&curr) {
                    path.push(graph[prev].asset_id);
                    curr = prev;
                }
                path.reverse();
                return Some(path);
            }

            for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
                if !visited.contains(&neighbor) {
                    visited.insert(neighbor);
                    parent.insert(neighbor, current);
                    queue.push_back(neighbor);
                }
            }
        }

        None
    }

    /// Check if there is any path from one asset to another
    pub fn is_reachable(&self, from: &AssetId, to: &AssetId) -> bool {
        self.path(from, to).is_some()
    }

    /// Find common ancestors of two assets
    ///
    /// Returns assets that are ancestors of both input assets
    pub fn common_ancestors(&self, asset1: &AssetId, asset2: &AssetId) -> Vec<AssetId> {
        let ancestors1: HashSet<_> = self
            .ancestors_with_depth(asset1, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        let ancestors2: HashSet<_> = self
            .ancestors_with_depth(asset2, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        ancestors1.intersection(&ancestors2).copied().collect()
    }

    /// Find common descendants of two assets
    ///
    /// Returns assets that are descendants of both input assets
    pub fn common_descendants(&self, asset1: &AssetId, asset2: &AssetId) -> Vec<AssetId> {
        let descendants1: HashSet<_> = self
            .descendants_with_depth(asset1, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        let descendants2: HashSet<_> = self
            .descendants_with_depth(asset2, None)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        descendants1.intersection(&descendants2).copied().collect()
    }

    /// Get all root nodes (nodes with no ancestors)
    pub fn roots(&self) -> Vec<AssetId> {
        self.graph.roots()
    }

    /// Get all leaf nodes (nodes with no descendants)
    pub fn leaves(&self) -> Vec<AssetId> {
        self.graph.leaves()
    }

    /// Calculate the distance (shortest path length) between two assets
    ///
    /// Returns None if no path exists
    pub fn distance(&self, from: &AssetId, to: &AssetId) -> Option<usize> {
        self.path(from, to).map(|p| p.len().saturating_sub(1))
    }

    /// Internal helper for depth-limited traversal
    fn traverse_with_depth(
        &self,
        start: &AssetId,
        direction: Direction,
        max_depth: Option<usize>,
    ) -> Vec<(AssetId, usize)> {
        let graph = self.graph.inner_graph();

        let start_idx = match self.graph.get_node_index(start) {
            Some(idx) => idx,
            None => return Vec::new(),
        };

        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // Initialize with direct neighbors at depth 1
        for neighbor in graph.neighbors_directed(start_idx, direction) {
            queue.push_back((neighbor, 1usize));
            visited.insert(neighbor);
        }

        while let Some((node_idx, depth)) = queue.pop_front() {
            // Check depth limit
            if let Some(max) = max_depth {
                if depth > max {
                    continue;
                }
            }

            result.push((graph[node_idx].asset_id, depth));

            // Add neighbors at next depth
            let should_continue = max_depth.map(|m| depth < m).unwrap_or(true);
            if should_continue {
                for neighbor in graph.neighbors_directed(node_idx, direction) {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor);
                        queue.push_back((neighbor, depth + 1));
                    }
                }
            }
        }

        // Sort by depth
        result.sort_by_key(|(_, depth)| *depth);
        result
    }
}

/// Full lineage information for an asset
#[derive(Debug, Clone)]
pub struct LineageInfo {
    /// The asset being queried
    pub asset_id: AssetId,

    /// Total count of upstream assets
    pub upstream_count: usize,

    /// Total count of downstream assets
    pub downstream_count: usize,

    /// Upstream assets with their depth from the queried asset
    pub upstream_assets: Vec<(AssetId, usize)>,

    /// Downstream assets with their depth from the queried asset
    pub downstream_assets: Vec<(AssetId, usize)>,

    /// Maximum depth in the upstream direction
    pub max_upstream_depth: usize,

    /// Maximum depth in the downstream direction
    pub max_downstream_depth: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{LineageEdge, LineageNode};

    #[allow(dead_code)]
    fn create_test_graph() -> LineageGraph {
        // Create a graph: A -> B -> C -> D
        //                       \-> E
        let mut graph = LineageGraph::new();

        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();
        let d = AssetId::new();
        let e = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: c,
            name: "C".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: d,
            name: "D".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: e,
            name: "E".to_string(),
            node_type: "table".to_string(),
        });

        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                c,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                c,
                d,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                e,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        graph
    }

    #[test]
    fn test_query_creation() {
        let graph = LineageGraph::new();
        let _query = LineageQuery::new(&graph);
    }

    #[test]
    fn test_ancestors() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        let ancestors = query.ancestors(&b);
        assert_eq!(ancestors.len(), 1);
        assert!(ancestors.contains(&a));
    }

    #[test]
    fn test_descendants() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        let descendants = query.descendants(&a);
        assert_eq!(descendants.len(), 1);
        assert!(descendants.contains(&b));
    }

    #[test]
    fn test_path_exists() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: c,
            name: "C".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                c,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        let path = query.path(&a, &c);
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], a);
        assert_eq!(path[1], b);
        assert_eq!(path[2], c);
    }

    #[test]
    fn test_path_not_exists() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        // No edge between them

        let query = LineageQuery::new(&graph);
        let path = query.path(&a, &b);
        assert!(path.is_none());
    }

    #[test]
    fn test_is_reachable() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        assert!(query.is_reachable(&a, &b));
        assert!(!query.is_reachable(&b, &a)); // Reverse direction should not be reachable
    }

    #[test]
    fn test_ancestors_with_depth() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: c,
            name: "C".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                c,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);

        // All ancestors of C
        let ancestors = query.ancestors_with_depth(&c, None);
        assert_eq!(ancestors.len(), 2);

        // Only depth 1 ancestors of C (just B)
        let ancestors = query.ancestors_with_depth(&c, Some(1));
        assert_eq!(ancestors.len(), 1);
        assert_eq!(ancestors[0].0, b);
        assert_eq!(ancestors[0].1, 1);
    }

    #[test]
    fn test_distance() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: c,
            name: "C".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                c,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        assert_eq!(query.distance(&a, &b), Some(1));
        assert_eq!(query.distance(&a, &c), Some(2));
        assert_eq!(query.distance(&c, &a), None); // No reverse path
    }

    #[test]
    fn test_full_lineage() {
        let mut graph = LineageGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();
        let c = AssetId::new();

        graph.add_node(LineageNode {
            asset_id: a,
            name: "A".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: b,
            name: "B".to_string(),
            node_type: "table".to_string(),
        });
        graph.add_node(LineageNode {
            asset_id: c,
            name: "C".to_string(),
            node_type: "table".to_string(),
        });
        graph
            .add_edge(
                a,
                b,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();
        graph
            .add_edge(
                b,
                c,
                LineageEdge {
                    edge_type: "data".to_string(),
                },
            )
            .unwrap();

        let query = LineageQuery::new(&graph);
        let lineage = query.full_lineage(&b);

        assert_eq!(lineage.asset_id, b);
        assert_eq!(lineage.upstream_count, 1);
        assert_eq!(lineage.downstream_count, 1);
    }
}
