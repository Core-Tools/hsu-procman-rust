//! In-memory storage for the service registry.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **concurrent data structures** in Rust.
//! 
//! ## DashMap vs HashMap
//! 
//! **Go:**
//! ```go
//! type Registry struct {
//!     mu      sync.RWMutex
//!     modules map[string]*ModuleInfo
//! }
//! 
//! func (r *Registry) Get(id string) *ModuleInfo {
//!     r.mu.RLock()
//!     defer r.mu.RUnlock()
//!     return r.modules[id]
//! }
//! ```
//! 
//! **Rust with DashMap:**
//! ```rust
//! struct Registry {
//!     modules: DashMap<ModuleID, ModuleInfo>,
//! }
//! 
//! fn get(&self, id: &ModuleID) -> Option<ModuleInfo> {
//!     self.modules.get(id).map(|entry| entry.clone())
//! }
//! // No manual locking needed!
//! ```
//! 
//! ## Why DashMap?
//! 
//! - **Lock-free**: Uses sharding to minimize contention
//! - **Type-safe**: Can't forget to lock/unlock
//! - **Fast**: Often faster than RwLock<HashMap>
//! - **Simple API**: Just like HashMap but thread-safe

use dashmap::DashMap;
use hsu_common::{Error, ModuleID, Result};
use crate::types::ModuleInfo;
use std::sync::Arc;

/// Thread-safe in-memory registry storage.
/// 
/// # Rust Learning Note
/// 
/// ## Arc vs Rc vs Box
/// 
/// - `Box<T>`: Single owner, heap allocated
/// - `Rc<T>`: Multiple owners, NOT thread-safe
/// - `Arc<T>`: Multiple owners, thread-safe (atomic reference counting)
/// 
/// We use DashMap directly (not Arc<DashMap>) because:
/// - DashMap is already thread-safe
/// - We'll wrap the whole Registry in Arc later
#[derive(Clone)]
pub struct Registry {
    /// Map of module IDs to module information.
    /// 
    /// DashMap is like Arc<RwLock<HashMap>> but faster!
    modules: Arc<DashMap<ModuleID, ModuleInfo>>,
}

impl Registry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            modules: Arc::new(DashMap::new()),
        }
    }

    /// Publishes or updates module APIs.
    /// 
    /// # Rust Learning Note
    /// 
    /// Notice the signature: `&self` not `&mut self`!
    /// 
    /// **This is key:** DashMap allows interior mutability.
    /// - In Go: You'd need `*Registry` or explicit mutex
    /// - In Rust: DashMap provides safe concurrent mutation
    /// 
    /// This is called **interior mutability** - mutating through
    /// a shared reference. It's safe because DashMap uses locks internally.
    pub fn publish(&self, info: ModuleInfo) -> Result<()> {
        let module_id = info.module_id.clone();
        
        // Check if module already exists
        if let Some(mut existing) = self.modules.get_mut(&module_id) {
            // Update existing entry
            existing.update_apis(info.apis);
            tracing::info!("Updated module: {}", module_id);
        } else {
            // Insert new entry
            self.modules.insert(module_id.clone(), info);
            tracing::info!("Registered new module: {}", module_id);
        }
        
        Ok(())
    }

    /// Discovers APIs for a specific module.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Option<T> vs Null
    /// 
    /// Go:
    /// ```go
    /// func (r *Registry) Find(id string) *ModuleInfo {
    ///     return r.modules[id]  // Returns nil if not found
    /// }
    /// ```
    /// 
    /// Rust:
    /// ```rust
    /// fn discover(&self, id: &ModuleID) -> Result<ModuleInfo> {
    ///     self.modules.get(id)
    ///         .map(|entry| entry.clone())
    ///         .ok_or_else(|| Error::module_not_found(id.clone()))
    /// }
    /// ```
    /// 
    /// **Key difference:**
    /// - Go: `nil` is implicit, can be forgotten
    /// - Rust: `Option` is explicit, must be handled
    /// - Rust: Can convert `Option` to `Result` with `ok_or_else`
    pub fn discover(&self, module_id: &ModuleID) -> Result<ModuleInfo> {
        self.modules
            .get(module_id)
            .map(|entry| entry.clone())
            .ok_or_else(|| Error::module_not_found(module_id.clone()))
    }

    /// Lists all registered modules.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Iterators in Rust
    /// 
    /// ```rust
    /// self.modules.iter()
    ///     .map(|entry| entry.value().clone())
    ///     .collect()
    /// ```
    /// 
    /// This is a **lazy iterator chain**:
    /// 1. `iter()` creates an iterator (no allocation yet)
    /// 2. `map()` transforms each item (still lazy)
    /// 3. `collect()` actually runs and allocates Vec
    /// 
    /// In Go:
    /// ```go
    /// modules := make([]*ModuleInfo, 0, len(r.modules))
    /// for _, info := range r.modules {
    ///     modules = append(modules, info)
    /// }
    /// ```
    /// 
    /// Rust's version is often faster due to iterator optimizations!
    pub fn list_all(&self) -> Vec<ModuleInfo> {
        self.modules
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Removes a module from the registry.
    pub fn remove(&self, module_id: &ModuleID) -> Result<()> {
        self.modules
            .remove(module_id)
            .ok_or_else(|| Error::module_not_found(module_id.clone()))?;
        
        tracing::info!("Removed module: {}", module_id);
        Ok(())
    }

    /// Returns the number of registered modules.
    pub fn count(&self) -> usize {
        self.modules.len()
    }

    /// Clears all entries from the registry.
    pub fn clear(&self) {
        self.modules.clear();
        tracing::info!("Cleared all registry entries");
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RemoteModuleAPI;
    use hsu_common::Protocol;

    #[test]
    fn test_registry_publish_and_discover() {
        let registry = Registry::new();
        let module_id = ModuleID::from("test-module");
        
        let info = ModuleInfo::new(
            module_id.clone(),
            12345,
            vec![RemoteModuleAPI {
                service_id: "service1".to_string(),
                protocol: Protocol::Grpc,
                address: Some("localhost:50051".to_string()),
                metadata: None,
            }],
        );
        
        // Publish
        registry.publish(info).unwrap();
        
        // Discover
        let discovered = registry.discover(&module_id).unwrap();
        assert_eq!(discovered.module_id, module_id);
        assert_eq!(discovered.apis.len(), 1);
    }

    #[test]
    fn test_registry_update() {
        let registry = Registry::new();
        let module_id = ModuleID::from("test-module");
        
        // Initial publish
        let info1 = ModuleInfo::new(module_id.clone(), 12345, vec![]);
        registry.publish(info1).unwrap();
        
        // Update with new APIs
        let info2 = ModuleInfo::new(
            module_id.clone(),
            12345,
            vec![RemoteModuleAPI {
                service_id: "service1".to_string(),
                protocol: Protocol::Grpc,
                address: Some("localhost:50051".to_string()),
                metadata: None,
            }],
        );
        registry.publish(info2).unwrap();
        
        // Verify update
        let discovered = registry.discover(&module_id).unwrap();
        assert_eq!(discovered.apis.len(), 1);
    }

    #[test]
    fn test_registry_not_found() {
        let registry = Registry::new();
        let result = registry.discover(&ModuleID::from("nonexistent"));
        
        assert!(result.is_err());
        match result {
            Err(Error::ModuleNotFound { module_id }) => {
                assert_eq!(module_id.as_str(), "nonexistent");
            }
            _ => panic!("Expected ModuleNotFound error"),
        }
    }

    #[test]
    fn test_registry_list_all() {
        let registry = Registry::new();
        
        // Add multiple modules
        for i in 0..3 {
            let module_id = ModuleID::from(format!("module-{}", i));
            let info = ModuleInfo::new(module_id, 10000 + i, vec![]);
            registry.publish(info).unwrap();
        }
        
        let all = registry.list_all();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_registry_remove() {
        let registry = Registry::new();
        let module_id = ModuleID::from("test-module");
        
        let info = ModuleInfo::new(module_id.clone(), 12345, vec![]);
        registry.publish(info).unwrap();
        
        // Remove
        registry.remove(&module_id).unwrap();
        
        // Verify removed
        assert!(registry.discover(&module_id).is_err());
    }

    #[tokio::test]
    async fn test_registry_concurrent_access() {
        use std::sync::Arc;
        use tokio::task;
        
        let registry = Arc::new(Registry::new());
        let mut handles = vec![];
        
        // Spawn multiple tasks that publish concurrently
        for i in 0..10 {
            let registry = Arc::clone(&registry);
            let handle = task::spawn(async move {
                let module_id = ModuleID::from(format!("module-{}", i));
                let info = ModuleInfo::new(module_id, 10000 + i, vec![]);
                registry.publish(info).unwrap();
            });
            handles.push(handle);
        }
        
        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify all modules were registered
        assert_eq!(registry.count(), 10);
    }
}

