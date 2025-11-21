//! Workflow compiler
//!
//! The compiler validates workflows, resolves dependencies, and generates
//! execution plans.

use crate::{asset::Asset, workflow::Workflow};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

/// Errors that can occur during workflow compilation
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("Circular dependency detected in workflow")]
    CircularDependency,

    #[error("Missing asset referenced in workflow: {0}")]
    MissingAsset(String),

    #[error("Missing dependency {missing} referenced by asset {asset}")]
    MissingDependency { asset: String, missing: String },

    #[error("Duplicate asset registered: {0}")]
    DuplicateAsset(String),

    #[error("Invalid workflow: {0}")]
    InvalidWorkflow(String),
}

/// Compiles workflows into execution plans
pub struct WorkflowCompiler {
    assets_by_id: HashMap<crate::asset::AssetId, Asset>,
    assets_by_name: HashMap<String, crate::asset::AssetId>,
}

impl WorkflowCompiler {
    /// Create a new workflow compiler
    pub fn new() -> Self {
        Self {
            assets_by_id: HashMap::new(),
            assets_by_name: HashMap::new(),
        }
    }

    /// Register an asset with the compiler
    pub fn register_asset(&mut self, asset: Asset) -> Result<(), CompileError> {
        if self.assets_by_name.contains_key(&asset.name) {
            return Err(CompileError::DuplicateAsset(asset.name.clone()));
        }

        self.assets_by_name.insert(asset.name.clone(), asset.id);
        self.assets_by_id.insert(asset.id, asset);

        Ok(())
    }

    /// Compile a workflow and validate it
    pub fn compile(&self, workflow: &Workflow) -> Result<ExecutionPlan, CompileError> {
        self.validate_assets_exist(workflow)?;
        let execution_order = self.topological_sort(workflow)?;

        Ok(ExecutionPlan {
            workflow_id: workflow.id,
            execution_order,
        })
    }

    /// Ensure all assets referenced in the workflow have been registered
    fn validate_assets_exist(&self, workflow: &Workflow) -> Result<(), CompileError> {
        for asset_id in &workflow.assets {
            if !self.assets_by_id.contains_key(asset_id) {
                return Err(CompileError::MissingAsset(format!("{:?}", asset_id)));
            }
        }
        Ok(())
    }

    /// Perform a topological sort on the workflow's assets to derive execution order
    fn topological_sort(
        &self,
        workflow: &Workflow,
    ) -> Result<Vec<crate::asset::AssetId>, CompileError> {
        let workflow_assets: HashSet<_> = workflow.assets.iter().copied().collect();

        // Build adjacency and indegree only for assets that belong to the workflow
        let mut indegree: HashMap<_, usize> = workflow_assets
            .iter()
            .map(|asset_id| (*asset_id, 0_usize))
            .collect();

        let mut graph: HashMap<crate::asset::AssetId, Vec<crate::asset::AssetId>> = HashMap::new();

        for asset_id in &workflow.assets {
            let asset = self
                .assets_by_id
                .get(asset_id)
                .expect("validated asset existence above");

            for dependency in &asset.dependencies {
                let upstream = dependency.upstream_asset_id;

                if !self.assets_by_id.contains_key(&upstream) {
                    return Err(CompileError::MissingDependency {
                        asset: asset.name.clone(),
                        missing: format!("{:?}", upstream),
                    });
                }

                if !workflow_assets.contains(&upstream) {
                    return Err(CompileError::MissingDependency {
                        asset: asset.name.clone(),
                        missing: format!("{:?}", upstream),
                    });
                }

                graph.entry(upstream).or_default().push(*asset_id);
                *indegree.entry(*asset_id).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm for topological sorting
        let mut ready: VecDeque<_> = indegree
            .iter()
            .filter_map(|(asset_id, &deg)| if deg == 0 { Some(*asset_id) } else { None })
            .collect();

        let mut ordered = Vec::with_capacity(workflow.assets.len());

        while let Some(asset_id) = ready.pop_front() {
            ordered.push(asset_id);

            if let Some(children) = graph.get(&asset_id) {
                for child in children {
                    let entry = indegree
                        .get_mut(child)
                        .expect("child should be present in indegree map");
                    *entry -= 1;
                    if *entry == 0 {
                        ready.push_back(*child);
                    }
                }
            }
        }

        if ordered.len() != workflow.assets.len() {
            return Err(CompileError::CircularDependency);
        }

        Ok(ordered)
    }
}

impl Default for WorkflowCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// An execution plan generated by the compiler
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    /// The workflow being executed
    pub workflow_id: crate::workflow::WorkflowId,

    /// Assets in topologically sorted execution order
    pub execution_order: Vec<crate::asset::AssetId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Asset, DependencyType};
    use crate::workflow::Workflow;
    use std::collections::HashMap;

    #[test]
    fn test_compiler_creation() {
        let compiler = WorkflowCompiler::new();
        assert_eq!(compiler.assets_by_id.len(), 0);
    }

    #[test]
    fn test_register_asset() {
        let mut compiler = WorkflowCompiler::new();
        let asset = Asset::new("test_asset");

        compiler.register_asset(asset.clone()).unwrap();

        assert_eq!(compiler.assets_by_id.len(), 1);
        assert!(compiler.assets_by_name.contains_key("test_asset"));
    }

    #[test]
    fn test_compile_topological_order() {
        let mut compiler = WorkflowCompiler::new();
        let upstream = Asset::new("upstream");
        let mut downstream = Asset::new("downstream");

        downstream.add_dependency(upstream.id, DependencyType::Data);

        compiler.register_asset(upstream.clone()).unwrap();
        compiler.register_asset(downstream.clone()).unwrap();

        let mut workflow = Workflow::new("test_workflow");
        workflow.add_asset(upstream.id);
        workflow.add_asset(downstream.id);

        let plan = compiler.compile(&workflow).expect("plan should compile");
        assert_eq!(plan.execution_order.len(), 2);
        assert_eq!(plan.execution_order[0], upstream.id);
        assert_eq!(plan.execution_order[1], downstream.id);
    }

    #[test]
    fn test_compile_detects_cycle() {
        let mut compiler = WorkflowCompiler::new();
        let mut a = Asset::new("a");
        let mut b = Asset::new("b");

        a.add_dependency(b.id, DependencyType::Data);
        b.add_dependency(a.id, DependencyType::Data);

        compiler.register_asset(a.clone()).unwrap();
        compiler.register_asset(b.clone()).unwrap();

        let mut workflow = Workflow::new("cyclic_workflow");
        workflow.add_asset(a.id);
        workflow.add_asset(b.id);

        let err = compiler.compile(&workflow).unwrap_err();
        assert!(matches!(err, CompileError::CircularDependency));
    }

    #[test]
    fn test_compile_missing_dependency() {
        let mut compiler = WorkflowCompiler::new();
        let mut a = Asset::new("a");
        let b = Asset::new("b");

        a.add_dependency(b.id, DependencyType::Data);

        compiler.register_asset(a.clone()).unwrap();
        // b intentionally not registered

        let mut workflow = Workflow::new("invalid_workflow");
        workflow.add_asset(a.id);

        let err = compiler.compile(&workflow).unwrap_err();
        assert!(matches!(err, CompileError::MissingDependency { .. }));
    }

    #[test]
    fn test_compile_large_dag_respects_dependencies() {
        // Build a 10-node DAG with multiple diamonds.
        let mut compiler = WorkflowCompiler::new();
        let mut assets = Vec::new();
        for i in 0..10 {
            assets.push(Asset::new(format!("asset_{i}")));
        }
        let ids: Vec<_> = assets.iter().map(|a| a.id).collect();

        // Dependencies:
        // 3 depends on 0,1
        assets[3].add_dependency(ids[0], DependencyType::Data);
        assets[3].add_dependency(ids[1], DependencyType::Data);
        // 4 depends on 1,2
        assets[4].add_dependency(ids[1], DependencyType::Data);
        assets[4].add_dependency(ids[2], DependencyType::Data);
        // 5 depends on 3,4
        assets[5].add_dependency(ids[3], DependencyType::Data);
        assets[5].add_dependency(ids[4], DependencyType::Data);
        // 6 depends on 2
        assets[6].add_dependency(ids[2], DependencyType::Data);
        // 7 depends on 5,6
        assets[7].add_dependency(ids[5], DependencyType::Data);
        assets[7].add_dependency(ids[6], DependencyType::Data);
        // 8 depends on 5
        assets[8].add_dependency(ids[5], DependencyType::Data);
        // 9 depends on 7,8
        assets[9].add_dependency(ids[7], DependencyType::Data);
        assets[9].add_dependency(ids[8], DependencyType::Data);

        for asset in &assets {
            compiler.register_asset(asset.clone()).unwrap();
        }

        let mut workflow = Workflow::new("large_dag");
        for asset in &assets {
            workflow.add_asset(asset.id);
        }

        let plan = compiler.compile(&workflow).expect("plan should compile");
        let position: HashMap<_, _> = plan
            .execution_order
            .iter()
            .enumerate()
            .map(|(idx, id)| (*id, idx))
            .collect();

        let must_precede = [
            (assets[0].id, assets[3].id),
            (assets[1].id, assets[3].id),
            (assets[1].id, assets[4].id),
            (assets[2].id, assets[4].id),
            (assets[3].id, assets[5].id),
            (assets[4].id, assets[5].id),
            (assets[2].id, assets[6].id),
            (assets[5].id, assets[7].id),
            (assets[6].id, assets[7].id),
            (assets[5].id, assets[8].id),
            (assets[7].id, assets[9].id),
            (assets[8].id, assets[9].id),
        ];

        for (up, down) in must_precede {
            let up_pos = position.get(&up).unwrap();
            let down_pos = position.get(&down).unwrap();
            assert!(
                up_pos < down_pos,
                "expected {up:?} to precede {down:?} but got {up_pos:?} >= {down_pos:?}"
            );
        }
    }
}
