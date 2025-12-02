//! Lineage command - displays asset dependency graphs

use anyhow::{Context, Result};
use servo_core::AssetId;
use servo_lineage::{LineageEdge, LineageGraph, LineageNode, LineageQuery};
use servo_storage::{PostgresStorage, TenantId};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

pub async fn execute(
    name: &str,
    upstream: bool,
    downstream: bool,
    tenant_id: &str,
    database_url: &str,
) -> Result<()> {
    let tenant = TenantId::new(tenant_id.to_string());
    let storage = PostgresStorage::new(database_url)
        .await
        .context("Failed to connect to database")?;

    // Look up the asset by name
    let asset = storage
        .get_asset_by_name(name, &tenant)
        .await
        .context(format!("Asset '{}' not found", name))?;

    println!("\n Lineage for: {}", asset.name);
    println!("   Type: {}", asset.asset_type);
    println!("   ID: {}", asset.id);
    println!();

    // Build the lineage graph by traversing dependencies
    let graph = build_lineage_graph(&storage, asset.id, &tenant, upstream, downstream).await?;
    let query = LineageQuery::new(&graph);

    // Determine what to show
    let show_both = !upstream && !downstream;
    let show_upstream = upstream || show_both;
    let show_downstream = downstream || show_both;

    // Get the asset ID for queries
    let asset_id = AssetId(asset.id);

    if show_upstream {
        let ancestors = query.ancestors_with_depth(&asset_id, None);
        print_upstream(&ancestors, &graph);
    }

    if show_downstream {
        let descendants = query.descendants_with_depth(&asset_id, None);
        print_downstream(&descendants, &graph);
    }

    // Show summary
    let info = query.full_lineage(&asset_id);
    println!("---------------------------------------------");
    println!(
        "Summary: {} upstream, {} downstream",
        info.upstream_count, info.downstream_count
    );

    Ok(())
}

/// Build a lineage graph by traversing dependencies from storage
async fn build_lineage_graph(
    storage: &PostgresStorage,
    start_asset_id: Uuid,
    tenant_id: &TenantId,
    include_upstream: bool,
    include_downstream: bool,
) -> Result<LineageGraph> {
    let mut graph = LineageGraph::new();
    let mut visited: HashSet<Uuid> = HashSet::new();
    let mut to_visit: VecDeque<Uuid> = VecDeque::new();
    let mut asset_cache: HashMap<Uuid, (String, String)> = HashMap::new(); // id -> (name, type)

    // Start with the target asset
    to_visit.push_back(start_asset_id);

    // If neither direction specified, include both
    let traverse_upstream = include_upstream || !include_downstream;
    let traverse_downstream = include_downstream || !include_upstream;

    while let Some(current_id) = to_visit.pop_front() {
        if visited.contains(&current_id) {
            continue;
        }
        visited.insert(current_id);

        // Fetch asset details if not cached
        let (name, asset_type) = if let Some(cached) = asset_cache.get(&current_id) {
            cached.clone()
        } else {
            // Try to get asset, skip if not found
            match storage.get_asset(current_id, tenant_id).await {
                Ok(a) => {
                    let result = (a.name.clone(), a.asset_type.clone());
                    asset_cache.insert(current_id, result.clone());
                    result
                }
                Err(_) => {
                    // Asset not found or error, skip
                    continue;
                }
            }
        };

        // Add node to graph
        let node = LineageNode {
            asset_id: AssetId(current_id),
            name,
            node_type: asset_type,
        };
        graph.add_node(node);

        // Get dependencies
        let (upstream_deps, downstream_deps) = storage
            .get_asset_dependencies(current_id, tenant_id)
            .await?;

        // Process upstream (what this asset depends on)
        if traverse_upstream {
            for dep in &upstream_deps {
                let upstream_id = dep.upstream_asset_id;

                // Ensure upstream asset is in graph
                if !visited.contains(&upstream_id) {
                    to_visit.push_back(upstream_id);
                }

                // Cache asset info for edge creation
                if let std::collections::hash_map::Entry::Vacant(e) = asset_cache.entry(upstream_id)
                {
                    if let Ok(a) = storage.get_asset(upstream_id, tenant_id).await {
                        e.insert((a.name.clone(), a.asset_type.clone()));
                    }
                }
            }
        }

        // Process downstream (what depends on this asset)
        if traverse_downstream {
            for dep in &downstream_deps {
                let downstream_id = dep.downstream_asset_id;

                // Ensure downstream asset is in graph
                if !visited.contains(&downstream_id) {
                    to_visit.push_back(downstream_id);
                }

                // Cache asset info for edge creation
                if let std::collections::hash_map::Entry::Vacant(e) =
                    asset_cache.entry(downstream_id)
                {
                    if let Ok(a) = storage.get_asset(downstream_id, tenant_id).await {
                        e.insert((a.name.clone(), a.asset_type.clone()));
                    }
                }
            }
        }
    }

    // Now add all nodes that were cached but not visited (for edge endpoints)
    for (id, (name, asset_type)) in &asset_cache {
        if !visited.contains(id) {
            let node = LineageNode {
                asset_id: AssetId(*id),
                name: name.clone(),
                node_type: asset_type.clone(),
            };
            graph.add_node(node);
        }
    }

    // Second pass: add edges
    // We need to re-query dependencies for all visited nodes to create edges
    for &node_id in &visited {
        let (upstream_deps, downstream_deps) =
            storage.get_asset_dependencies(node_id, tenant_id).await?;

        // Add upstream edges (upstream -> current)
        if traverse_upstream {
            for dep in &upstream_deps {
                let from_id = AssetId(dep.upstream_asset_id);
                let to_id = AssetId(node_id);

                // Only add edge if both nodes exist in graph
                if graph.get_node(&from_id).is_some() && graph.get_node(&to_id).is_some() {
                    let _ = graph.add_edge(
                        from_id,
                        to_id,
                        LineageEdge {
                            edge_type: dep.dependency_type.clone(),
                        },
                    );
                }
            }
        }

        // Add downstream edges (current -> downstream)
        if traverse_downstream {
            for dep in &downstream_deps {
                let from_id = AssetId(node_id);
                let to_id = AssetId(dep.downstream_asset_id);

                // Only add edge if both nodes exist in graph
                if graph.get_node(&from_id).is_some() && graph.get_node(&to_id).is_some() {
                    let _ = graph.add_edge(
                        from_id,
                        to_id,
                        LineageEdge {
                            edge_type: dep.dependency_type.clone(),
                        },
                    );
                }
            }
        }
    }

    Ok(graph)
}

fn print_upstream(ancestors: &[(AssetId, usize)], graph: &LineageGraph) {
    println!("Upstream Dependencies:");
    if ancestors.is_empty() {
        println!("   (none - this is a root asset)");
    } else {
        // Group by depth
        let max_depth = ancestors.iter().map(|(_, d)| *d).max().unwrap_or(0);
        for depth in 1..=max_depth {
            let at_depth: Vec<_> = ancestors.iter().filter(|(_, d)| *d == depth).collect();

            if !at_depth.is_empty() {
                let indent = "   ".repeat(depth);
                for (asset_id, _) in at_depth {
                    if let Some(node) = graph.get_node(asset_id) {
                        println!("{} {} ({})", indent, node.name, node.node_type);
                    }
                }
            }
        }
    }
    println!();
}

fn print_downstream(descendants: &[(AssetId, usize)], graph: &LineageGraph) {
    println!("Downstream Dependencies:");
    if descendants.is_empty() {
        println!("   (none - this is a leaf asset)");
    } else {
        // Group by depth
        let max_depth = descendants.iter().map(|(_, d)| *d).max().unwrap_or(0);
        for depth in 1..=max_depth {
            let at_depth: Vec<_> = descendants.iter().filter(|(_, d)| *d == depth).collect();

            if !at_depth.is_empty() {
                let indent = "   ".repeat(depth);
                for (asset_id, _) in at_depth {
                    if let Some(node) = graph.get_node(asset_id) {
                        println!("{} {} ({})", indent, node.name, node.node_type);
                    }
                }
            }
        }
    }
    println!();
}
