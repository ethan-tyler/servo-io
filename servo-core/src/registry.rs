//! Asset registry
//!
//! The registry maintains a catalog of all assets and their metadata.
//! Provides thread-safe concurrent access for multi-threaded environments.

use crate::asset::{Asset, AssetId};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
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

    #[error("Registry lock poisoned")]
    LockPoisoned,
}

/// Registry for managing assets with thread-safe concurrent access.
/// This type uses blocking locks and is intended for synchronous contexts; do
/// not hold `register`/`update`/`list` across async await points.
#[derive(Clone)]
pub struct AssetRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

struct RegistryInner {
    assets_by_id: HashMap<AssetId, Asset>,
    assets_by_name: HashMap<String, AssetId>,
}

impl AssetRegistry {
    /// Create a new asset registry
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner {
                assets_by_id: HashMap::new(),
                assets_by_name: HashMap::new(),
            })),
        }
    }

    /// Register a new asset
    pub fn register(&self, asset: Asset) -> Result<(), RegistryError> {
        let mut inner = self.write()?;

        if inner.assets_by_id.contains_key(&asset.id) {
            return Err(RegistryError::AssetAlreadyExists(format!("{:?}", asset.id)));
        }

        if inner.assets_by_name.contains_key(&asset.name) {
            return Err(RegistryError::AssetAlreadyExists(asset.name.clone()));
        }

        inner.assets_by_name.insert(asset.name.clone(), asset.id);
        inner.assets_by_id.insert(asset.id, asset);

        Ok(())
    }

    /// Register multiple assets in a batch
    pub fn register_batch(&self, assets: Vec<Asset>) -> Result<(), RegistryError> {
        let mut inner = self.write()?;
        let mut seen_names: HashSet<&str> = HashSet::new();
        let mut seen_ids: HashSet<AssetId> = HashSet::new();

        // Validate all assets first
        for asset in &assets {
            if !seen_names.insert(&asset.name) {
                return Err(RegistryError::AssetAlreadyExists(asset.name.clone()));
            }
            if !seen_ids.insert(asset.id) {
                return Err(RegistryError::AssetAlreadyExists(format!("{:?}", asset.id)));
            }
            if inner.assets_by_name.contains_key(&asset.name) {
                return Err(RegistryError::AssetAlreadyExists(asset.name.clone()));
            }
            if inner.assets_by_id.contains_key(&asset.id) {
                return Err(RegistryError::AssetAlreadyExists(format!("{:?}", asset.id)));
            }
        }

        // Register all assets
        for asset in assets {
            inner.assets_by_name.insert(asset.name.clone(), asset.id);
            inner.assets_by_id.insert(asset.id, asset);
        }

        Ok(())
    }

    /// Update an existing asset
    pub fn update(&self, asset: Asset) -> Result<(), RegistryError> {
        let mut inner = self.write()?;

        let old_asset = inner
            .assets_by_id
            .get(&asset.id)
            .cloned()
            .ok_or_else(|| RegistryError::AssetNotFound(format!("{:?}", asset.id)))?;

        if old_asset.name != asset.name {
            if let Some(existing_id) = inner.assets_by_name.get(&asset.name) {
                if existing_id != &asset.id {
                    return Err(RegistryError::AssetAlreadyExists(asset.name.clone()));
                }
            }
            inner.assets_by_name.remove(&old_asset.name);
            inner.assets_by_name.insert(asset.name.clone(), asset.id);
        }

        inner.assets_by_id.insert(asset.id, asset);

        Ok(())
    }

    /// Get an asset by ID (returns a clone for thread-safety)
    pub fn get_by_id(&self, id: &AssetId) -> Option<Asset> {
        let inner = self.read().ok()?;
        inner.assets_by_id.get(id).cloned()
    }

    /// Get an asset by name (returns a clone for thread-safety)
    pub fn get_by_name(&self, name: &str) -> Option<Asset> {
        let inner = self.read().ok()?;
        inner
            .assets_by_name
            .get(name)
            .and_then(|id| inner.assets_by_id.get(id).cloned())
    }

    /// List all assets (returns clones for thread-safety)
    pub fn list(&self) -> Vec<Asset> {
        let inner = self.read().ok();
        inner
            .map(|i| i.assets_by_id.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Count total number of registered assets
    pub fn count(&self) -> usize {
        let inner = self.read().ok();
        inner.map(|i| i.assets_by_id.len()).unwrap_or(0)
    }

    /// Check if an asset exists by ID
    pub fn contains_id(&self, id: &AssetId) -> bool {
        let inner = self.read().ok();
        inner
            .map(|i| i.assets_by_id.contains_key(id))
            .unwrap_or(false)
    }

    /// Check if an asset exists by name
    pub fn contains_name(&self, name: &str) -> bool {
        let inner = self.read().ok();
        inner
            .map(|i| i.assets_by_name.contains_key(name))
            .unwrap_or(false)
    }

    /// Remove an asset from the registry
    pub fn remove(&self, id: &AssetId) -> Option<Asset> {
        let mut inner = self.write().ok()?;

        if let Some(asset) = inner.assets_by_id.remove(id) {
            inner.assets_by_name.remove(&asset.name);
            Some(asset)
        } else {
            None
        }
    }

    /// Clear all assets from the registry
    pub fn clear(&self) {
        if let Ok(mut inner) = self.write() {
            inner.assets_by_id.clear();
            inner.assets_by_name.clear();
        }
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, RegistryInner>, RegistryError> {
        self.inner.read().map_err(|_| RegistryError::LockPoisoned)
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, RegistryInner>, RegistryError> {
        self.inner.write().map_err(|_| RegistryError::LockPoisoned)
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
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_register_asset() {
        let registry = AssetRegistry::new();
        let asset = Asset::new("test_asset");
        let asset_id = asset.id;

        assert!(registry.register(asset).is_ok());
        assert!(registry.get_by_id(&asset_id).is_some());
        assert!(registry.get_by_name("test_asset").is_some());
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_duplicate_asset() {
        let registry = AssetRegistry::new();
        let asset1 = Asset::new("test_asset");
        let asset2 = Asset::new("test_asset");

        assert!(registry.register(asset1).is_ok());
        assert!(registry.register(asset2).is_err());
    }

    #[test]
    fn test_update_asset() {
        let registry = AssetRegistry::new();
        let mut asset = Asset::new("test_asset");
        let asset_id = asset.id;

        registry.register(asset.clone()).unwrap();

        // Update description
        asset.metadata.description = Some("Updated description".to_string());
        assert!(registry.update(asset).is_ok());

        let retrieved = registry.get_by_id(&asset_id).unwrap();
        assert_eq!(
            retrieved.metadata.description,
            Some("Updated description".to_string())
        );
    }

    #[test]
    fn test_update_nonexistent_asset() {
        let registry = AssetRegistry::new();
        let asset = Asset::new("test_asset");

        let result = registry.update(asset);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RegistryError::AssetNotFound(_)
        ));
    }

    #[test]
    fn test_update_rejects_name_collision() {
        let registry = AssetRegistry::new();
        let mut asset1 = Asset::new("asset1");
        let asset2 = Asset::new("asset2");

        registry.register(asset1.clone()).unwrap();
        registry.register(asset2.clone()).unwrap();

        // Attempt to rename asset1 to asset2's name
        asset1.name = "asset2".to_string();
        let result = registry.update(asset1);
        assert!(matches!(result, Err(RegistryError::AssetAlreadyExists(_))));
    }

    #[test]
    fn test_register_batch() {
        let registry = AssetRegistry::new();
        let assets = vec![
            Asset::new("asset1"),
            Asset::new("asset2"),
            Asset::new("asset3"),
        ];

        assert!(registry.register_batch(assets).is_ok());
        assert_eq!(registry.count(), 3);
        assert!(registry.contains_name("asset1"));
        assert!(registry.contains_name("asset2"));
        assert!(registry.contains_name("asset3"));
    }

    #[test]
    fn test_register_batch_with_duplicate() {
        let registry = AssetRegistry::new();
        let asset1 = Asset::new("asset1");
        registry.register(asset1).unwrap();

        let assets = vec![Asset::new("asset1"), Asset::new("asset2")];

        let result = registry.register_batch(assets);
        assert!(result.is_err());
        // Transaction should rollback, so asset2 should not be registered
        assert_eq!(registry.count(), 1);
        assert!(!registry.contains_name("asset2"));
    }

    #[test]
    fn test_register_batch_with_duplicate_ids() {
        let registry = AssetRegistry::new();
        let asset1 = Asset::new("asset1");
        let mut asset2 = Asset::new("asset2");
        asset2.id = asset1.id;

        let result = registry.register_batch(vec![asset1, asset2]);
        assert!(matches!(result, Err(RegistryError::AssetAlreadyExists(_))));
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_remove_asset() {
        let registry = AssetRegistry::new();
        let asset = Asset::new("test_asset");
        let asset_id = asset.id;

        registry.register(asset).unwrap();
        assert_eq!(registry.count(), 1);

        let removed = registry.remove(&asset_id);
        assert!(removed.is_some());
        assert_eq!(registry.count(), 0);
        assert!(!registry.contains_id(&asset_id));
        assert!(!registry.contains_name("test_asset"));
    }

    #[test]
    fn test_clear_registry() {
        let registry = AssetRegistry::new();
        registry
            .register_batch(vec![
                Asset::new("asset1"),
                Asset::new("asset2"),
                Asset::new("asset3"),
            ])
            .unwrap();

        assert_eq!(registry.count(), 3);

        registry.clear();
        assert_eq!(registry.count(), 0);
        assert_eq!(registry.list().len(), 0);
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;

        let registry = AssetRegistry::new();

        // Spawn multiple threads that register assets concurrently
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let reg = registry.clone();
                thread::spawn(move || {
                    let asset = Asset::new(format!("asset_{}", i));
                    reg.register(asset).unwrap();
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // All assets should be registered
        assert_eq!(registry.count(), 10);
    }
}
