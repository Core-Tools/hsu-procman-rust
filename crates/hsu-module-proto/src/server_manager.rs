//! Server manager for managing multiple protocol servers.
//!
//! # Rust Learning Note
//!
//! This module demonstrates **lifecycle coordination** in Rust.
//!
//! ## ServerManager Pattern
//!
//! ```
//! ServerManager
//! ├── Registers protocol servers
//! ├── Starts all servers
//! ├── Manages their lifecycle
//! └── Stops all servers gracefully
//! ```
//!
//! ## Rust vs Golang
//!
//! **Golang Pattern:**
//! ```go
//! type ServerManager struct {
//!     servers map[ServerID]ProtocolServer
//!     mutex   sync.Mutex
//! }
//!
//! func (m *ServerManager) Start(ctx context.Context) error {
//!     for _, server := range m.servers {
//!         if err := server.Start(ctx); err != nil {
//!             return err
//!         }
//!     }
//!     return nil
//! }
//! ```
//!
//! **Rust Pattern:**
//! ```rust
//! pub struct ServerManager {
//!     servers: HashMap<String, Box<dyn ProtocolServer>>,
//! }
//!
//! impl ServerManager {
//!     pub async fn start_all(&mut self) -> Result<()> {
//!         for server in self.servers.values_mut() {
//!             server.start().await?;
//!         }
//!         Ok(())
//!     }
//! }
//! ```
//!
//! Key differences:
//! 1. **No mutex needed** - Rust's borrow checker prevents concurrent access
//! 2. **&mut self** - Exclusive access guaranteed at compile time
//! 3. **async/await** - Explicit async without goroutines

use std::collections::HashMap;
use hsu_common::{Result, Error};
use tracing::{info, debug, warn};

use crate::server::ProtocolServer;

/// Server identifier (e.g., "grpc-server-1", "http-api").
pub type ServerID = String;

/// Server manager for managing multiple protocol servers.
///
/// # Rust Learning Note
///
/// ## Ownership of Servers
///
/// ```rust
/// servers: HashMap<String, Box<dyn ProtocolServer>>
/// ```
///
/// This means:
/// - ServerManager **owns** all servers
/// - Servers are heap-allocated (Box)
/// - Servers are trait objects (dyn ProtocolServer)
/// - When manager is dropped, all servers are stopped and dropped
///
/// This is **RAII** (Resource Acquisition Is Initialization):
/// - Constructor acquires resources
/// - Destructor releases resources
/// - Automatic, no manual cleanup!
///
/// ## No Need for Mutex
///
/// In Golang, you need `sync.Mutex` to protect the map.
/// In Rust, the type system prevents concurrent access:
/// - `&self` methods: Multiple readers OK
/// - `&mut self` methods: Exclusive access
/// - Compiler enforces this at compile time!
pub struct ServerManager {
    /// Registered protocol servers.
    servers: HashMap<ServerID, Box<dyn ProtocolServer>>,
}

impl ServerManager {
    /// Creates a new server manager.
    ///
    /// # Rust Learning Note
    ///
    /// ## Simple Constructor
    ///
    /// No complex initialization needed:
    /// - Empty HashMap
    /// - No mutexes to initialize
    /// - No cleanup required
    ///
    /// Compare to Golang:
    /// ```go
    /// &ServerManager{
    ///     servers: make(map[ServerID]ProtocolServer),
    ///     mutex:   sync.Mutex{},
    /// }
    /// ```
    ///
    /// Rust is simpler because the type system handles safety!
    pub fn new() -> Self {
        info!("Creating server manager");
        Self {
            servers: HashMap::new(),
        }
    }

    /// Registers a protocol server.
    ///
    /// # Rust Learning Note
    ///
    /// ## Taking Ownership
    ///
    /// ```rust
    /// pub fn register(&mut self, id: String, server: Box<dyn ProtocolServer>)
    /// ```
    ///
    /// The server is **moved** into the manager:
    /// - Caller can't use it anymore
    /// - Manager owns it completely
    /// - Will be stopped when manager is dropped
    ///
    /// ## Error on Duplicate
    ///
    /// Using `HashMap::insert()` would silently replace.
    /// We check explicitly to catch configuration errors.
    pub fn register(&mut self, id: ServerID, server: Box<dyn ProtocolServer>) -> Result<()> {
        debug!("Registering server: {}", id);

        if self.servers.contains_key(&id) {
            return Err(Error::validation(format!(
                "Server with ID '{}' already registered",
                id
            )));
        }

        self.servers.insert(id, server);
        Ok(())
    }

    /// Finds a protocol server by ID.
    ///
    /// # Rust Learning Note
    ///
    /// ## Returning References
    ///
    /// ```rust
    /// pub fn find(&self, id: &str) -> Option<&dyn ProtocolServer>
    /// ```
    ///
    /// - Returns **reference** (not ownership)
    /// - Lifetime tied to self
    /// - Can't outlive the manager
    /// - Multiple readers can call this simultaneously!
    ///
    /// ## Option vs Error
    ///
    /// We return `Option<&dyn ProtocolServer>`:
    /// - `Some(&server)` if found
    /// - `None` if not found
    ///
    /// This is more idiomatic than returning Result for "not found".
    pub fn find(&self, id: &str) -> Option<&dyn ProtocolServer> {
        self.servers.get(id).map(|boxed| boxed.as_ref())
    }

    /// Finds a mutable protocol server by ID.
    ///
    /// # Rust Learning Note
    ///
    /// ## Mutable References
    ///
    /// ```rust
    /// pub fn find_mut(&mut self, id: &str) -> Option<&mut dyn ProtocolServer>
    /// ```
    ///
    /// This is a **separate method** from `find()` because:
    /// - `&mut self` requires exclusive access
    /// - Can't have multiple mutable references
    /// - Borrow checker enforces this
    ///
    /// In Golang, you'd use the same method and hope for no data races!
    pub fn find_mut(&mut self, id: &str) -> Option<&mut dyn ProtocolServer> {
        match self.servers.get_mut(id) {
            Some(boxed) => Some(boxed.as_mut()),
            None => None,
        }
    }

    /// Returns the number of registered servers.
    pub fn count(&self) -> usize {
        self.servers.len()
    }

    /// Returns an iterator over server IDs.
    pub fn server_ids(&self) -> std::collections::hash_map::Keys<'_, ServerID, Box<dyn ProtocolServer>> {
        self.servers.keys()
    }

    /// Starts all registered servers.
    ///
    /// # Rust Learning Note
    ///
    /// ## Error Handling Strategy
    ///
    /// We have two options:
    ///
    /// 1. **Fail fast** (current implementation):
    /// ```rust
    /// for server in servers {
    ///     server.start().await?;  // Stop on first error
    /// }
    /// ```
    ///
    /// 2. **Collect all errors**:
    /// ```rust
    /// let mut errors = Vec::new();
    /// for server in servers {
    ///     if let Err(e) = server.start().await {
    ///         errors.push(e);
    ///     }
    /// }
    /// ```
    ///
    /// We use fail-fast for simplicity. In production, you might want to
    /// collect all errors and return them together.
    ///
    /// ## Mutable Iteration
    ///
    /// ```rust
    /// for server in self.servers.values_mut() {
    ///     server.start().await?;
    /// }
    /// ```
    ///
    /// - `values_mut()`: Iterator over &mut values
    /// - Each server can be modified
    /// - No data races (enforced by borrow checker)
    pub async fn start_all(&mut self) -> Result<()> {
        info!("Starting {} protocol servers", self.servers.len());

        let server_ids: Vec<String> = self.servers.keys().cloned().collect();

        for id in server_ids {
            if let Some(server) = self.servers.get_mut(&id) {
                info!("Starting server: {} (protocol: {:?})", id, server.protocol());
                server.start().await
                    .map_err(|e| Error::Internal(format!("Failed to start server '{}': {}", id, e)))?;
            }
        }

        info!("✅ All protocol servers started successfully");
        Ok(())
    }

    /// Stops all registered servers.
    ///
    /// # Rust Learning Note
    ///
    /// ## Graceful Shutdown
    ///
    /// We stop servers in **reverse order**:
    /// ```rust
    /// let server_ids: Vec<_> = self.servers.keys().cloned().collect();
    /// for id in server_ids.iter().rev() {  // ← Reverse!
    ///     // Stop server
    /// }
    /// ```
    ///
    /// Why reverse order?
    /// - Mirrors startup order (LIFO)
    /// - Dependencies are stopped first
    /// - Common pattern in lifecycle management
    ///
    /// ## Continue on Error
    ///
    /// Unlike `start_all()`, we continue stopping even if one fails:
    /// ```rust
    /// if let Err(e) = server.stop().await {
    ///     warn!("Error stopping server '{}': {}", id, e);
    ///     // Continue with other servers
    /// }
    /// ```
    ///
    /// This ensures best-effort cleanup.
    pub async fn stop_all(&mut self) -> Result<()> {
        info!("Stopping {} protocol servers", self.servers.len());

        // Stop in reverse order
        let server_ids: Vec<String> = self.servers.keys().cloned().collect();
        
        let mut errors = Vec::new();

        for id in server_ids.iter().rev() {
            if let Some(server) = self.servers.get_mut(id) {
                info!("Stopping server: {}", id);
                
                if let Err(e) = server.stop().await {
                    warn!("Error stopping server '{}': {}", id, e);
                    errors.push((id.clone(), e));
                }
            }
        }

        if !errors.is_empty() {
            let error_msg = errors
                .iter()
                .map(|(id, e)| format!("  - {}: {}", id, e))
                .collect::<Vec<_>>()
                .join("\n");
            
            warn!("Some servers had errors during shutdown:\n{}", error_msg);
            
            // Return first error (could aggregate them instead)
            return Err(Error::Internal(format!(
                "Failed to stop {} server(s):\n{}",
                errors.len(),
                error_msg
            )));
        }

        info!("✅ All protocol servers stopped successfully");
        Ok(())
    }
}

impl Default for ServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hsu_common::Protocol;
    use crate::grpc_server::{GrpcProtocolServer, GrpcServerOptions};

    #[test]
    fn test_server_manager_creation() {
        let manager = ServerManager::new();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_server_registration() {
        let mut manager = ServerManager::new();

        let options = GrpcServerOptions::new().with_port(0);
        let server = Box::new(GrpcProtocolServer::new(options));

        manager.register("test-server".to_string(), server).unwrap();
        assert_eq!(manager.count(), 1);
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let mut manager = ServerManager::new();

        let options = GrpcServerOptions::new().with_port(0);
        let server1 = Box::new(GrpcProtocolServer::new(options.clone()));
        let server2 = Box::new(GrpcProtocolServer::new(options));

        manager.register("test-server".to_string(), server1).unwrap();
        
        let result = manager.register("test-server".to_string(), server2);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_server() {
        let mut manager = ServerManager::new();

        let options = GrpcServerOptions::new().with_port(0);
        let server = Box::new(GrpcProtocolServer::new(options));

        manager.register("test-server".to_string(), server).unwrap();

        let found = manager.find("test-server");
        assert!(found.is_some());
        assert_eq!(found.unwrap().protocol(), Protocol::Grpc);

        let not_found = manager.find("nonexistent");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_start_stop_all() {
        let mut manager = ServerManager::new();

        // Register two servers
        let options1 = GrpcServerOptions::new().with_port(0);
        let server1 = Box::new(GrpcProtocolServer::new(options1));
        manager.register("server1".to_string(), server1).unwrap();

        let options2 = GrpcServerOptions::new().with_port(0);
        let server2 = Box::new(GrpcProtocolServer::new(options2));
        manager.register("server2".to_string(), server2).unwrap();

        // Start all should fail (no services registered)
        let result = manager.start_all().await;
        assert!(result.is_err());

        // Stop all should still work (idempotent)
        manager.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn test_server_ids() {
        let mut manager = ServerManager::new();

        let options1 = GrpcServerOptions::new().with_port(0);
        manager.register("server-a".to_string(), Box::new(GrpcProtocolServer::new(options1))).unwrap();

        let options2 = GrpcServerOptions::new().with_port(0);
        manager.register("server-b".to_string(), Box::new(GrpcProtocolServer::new(options2))).unwrap();

        let ids: Vec<&String> = manager.server_ids().collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().any(|id| id.as_str() == "server-a"));
        assert!(ids.iter().any(|id| id.as_str() == "server-b"));
    }

    #[tokio::test]
    async fn test_stop_all_idempotent() {
        let mut manager = ServerManager::new();

        let options = GrpcServerOptions::new().with_port(0);
        manager.register("server".to_string(), Box::new(GrpcProtocolServer::new(options))).unwrap();

        // Stop without starting (should be no-op)
        manager.stop_all().await.unwrap();
        
        // Stop again (should still be no-op)
        manager.stop_all().await.unwrap();
    }
}

