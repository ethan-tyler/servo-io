//! Asset registry
//!
//! The registry maintains a catalog of all assets and their metadata.

use crate::asset::{Asset, AssetId};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur in the registry
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Asset not found: {0}")]
    AssetNotFound(String),

    #[error("Asset already exists: {0}")]
    AssetAlreadyExists(String),

    #[error("Invalid asset: {0}")]
    InvalidAsset(String),
}

/// Registry for managing assets
pub struct AssetRegistry {
    assets_by_id: HashMap<AssetId, Asset>,
    assets_by_name: HashMap<String, AssetId>,
}

impl AssetRegistry {
    /// Create a new asset registry
    pub fn new() -> Self {
        Self {
            assets_by_id: HashMap::new(),
            assets_by_name: HashMap::new(),
        }
    }

    /// Register a new asset
    pub fn register(&mut self, asset: Asset) -> Result<(), RegistryError> {
        if self.assets_by_name.contains_key(&asset.name) {
            return Err(RegistryError::AssetAlreadyExists(asset.name.clone()));
        }

        self.assets_by_name.insert(asset.name.clone(), asset.id);
        self.assets_by_id.insert(asset.id, asset);

        Ok(())
    }

    /// Get an asset by ID
    pub fn get_by_id(&self, id: &AssetId) -> Option<&Asset> {
        self.assets_by_id.get(id)
    }

    /// Get an asset by name
    pub fn get_by_name(&self, name: &str) -> Option<&Asset> {
        self.assets_by_name
            .get(name)
            .and_then(|id| self.assets_by_id.get(id))
    }

    /// List all assets
    pub fn list(&self) -> Vec<&Asset> {
        self.assets_by_id.values().collect()
    }

    /// Remove an asset from the registry
    pub fn remove(&mut self, id: &AssetId) -> Option<Asset> {
        if let Some(asset) = self.assets_by_id.remove(id) {
            self.assets_by_name.remove(&asset.name);
            Some(asset)
        } else {
            None
        }
    }
}

impl Default for AssetRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = AssetRegistry::new();
        assert_eq!(registry.list().len(), 0);
    }

    #[test]
    fn test_register_asset() {
        let mut registry = AssetRegistry::new();
        let asset = Asset::new("test_asset");
        let asset_id = asset.id;

        assert!(registry.register(asset).is_ok());
        assert!(registry.get_by_id(&asset_id).is_some());
        assert!(registry.get_by_name("test_asset").is_some());
    }

    #[test]
    fn test_duplicate_asset() {
        let mut registry = AssetRegistry::new();
        let asset1 = Asset::new("test_asset");
        let asset2 = Asset::new("test_asset");

        assert!(registry.register(asset1).is_ok());
        assert!(registry.register(asset2).is_err());
    }
}
