//! Service Connector - Protocol Selection and Connection Establishment
//!
//! # Architecture
//!
//! This module implements the `ServiceConnector` which handles:
//! 1. Protocol selection (Auto, Direct, gRPC, HTTP)
//! 2. Connection establishment
//! 3. Visitor dispatch for typed gateway creation
//!
//! This is analogous to Go's `ServiceConnector` and replaces the old
//! `ServiceGatewayFactoryImpl` pattern.
//!
//! # Rust Learning Note
//!
//! ## Visitor Pattern Integration
//!
//! The `ServiceConnector` uses the visitor pattern to achieve type-safe
//! protocol dispatch:
//!
//! ```text
//! 1. Caller creates a visitor (knows what type it wants)
//! 2. Caller calls connector.connect(..., visitor)
//! 3. Connector creates appropriate connection (gRPC, Direct, etc.)
//! 4. Connection calls visitor.protocol_is_X(...) with protocol-specific data
//! 5. Visitor stores typed result
//! 6. Caller extracts typed result
//! ```
//!
//! **This is "double dispatch"** - two method calls to resolve both:
//! - Which protocol (resolved by connector)
//! - What type (resolved by visitor)
//!
//! ## Comparison with Go
//!
//! **Go version:**
//! ```go
//! type ServiceConnector interface {
//!     Connect(ctx context.Context, moduleID, serviceID ModuleID, protocol Protocol,
//!             visitor ClientConnectionVisitor) error
//!     EnableDirectClosure(moduleID ModuleID, serviceIDs []ServiceID)
//! }
//! ```
//!
//! **Rust version:**
//! ```rust
//! #[async_trait]
//! pub trait ServiceConnector: Send + Sync {
//!     async fn connect(&self, module_id: &ModuleID, service_id: &ServiceID,
//!                      protocol: Protocol, visitor: &mut dyn ClientConnectionVisitor) -> Result<()>;
//!     fn enable_direct_closure(&mut self, module_id: ModuleID, service_ids: Vec<ServiceID>);
//! }
//! ```
//!
//! Nearly identical! The main difference is Rust's `&mut` for the visitor (ownership).

use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use async_trait::async_trait;
use hsu_common::{ModuleID, ServiceID, Protocol, Result, Error};
use hsu_module_proto::{ClientConnection, ClientConnectionVisitor, GrpcProtocolGateway, GrpcOptions};
use tracing::{debug, trace, error};

use crate::registry_client::ServiceRegistryClient;

/// Service connector handles protocol selection and connection establishment.
///
/// This trait is the core abstraction for connecting to services across
/// different protocols. It uses the visitor pattern to enable type-safe
/// protocol dispatch.
///
/// # Example Usage
///
/// ```rust,ignore
/// // Create a visitor that knows what type it wants
/// let mut visitor = MyGatewayVisitor::new(factory_funcs);
///
/// // Connect using the connector
/// connector.connect(&module_id, &service_id, Protocol::Auto, &mut visitor).await?;
///
/// // Extract typed result
/// let gateway: Arc<dyn MyService> = visitor.gateway()?;
/// ```
#[async_trait]
pub trait ServiceConnector: Send + Sync {
    /// Connect to a service using the specified protocol.
    ///
    /// The visitor pattern is used to handle protocol-specific connections:
    /// 1. ServiceConnector selects and creates the connection
    /// 2. Connection calls visitor.protocol_is_X(...)
    /// 3. Visitor stores typed result
    ///
    /// # Arguments
    ///
    /// * `module_id` - Target module ID
    /// * `service_id` - Target service ID
    /// * `protocol` - Desired protocol (Auto, Direct, gRPC, HTTP)
    /// * `visitor` - Visitor to handle the connection
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Service not found
    /// - Protocol not available
    /// - Connection failed
    async fn connect(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        protocol: Protocol,
        visitor: &mut dyn ClientConnectionVisitor,
    ) -> Result<()>;
    
    /// Enable direct closure for a module's services.
    ///
    /// When direct closure is enabled, `Protocol::Auto` will prefer
    /// direct (in-process) calls over remote calls.
    ///
    /// # Arguments
    ///
    /// * `module_id` - Module whose services are available locally
    /// * `service_ids` - List of service IDs available for direct calls
    ///
    /// # Rust Learning Note
    ///
    /// Takes `&self` (not `&mut self`) because implementations use
    /// interior mutability (`RwLock`) to allow mutation through shared
    /// references. This is necessary for `Arc<dyn ServiceConnector>`.
    fn enable_direct_closure(
        &self,
        module_id: ModuleID,
        service_ids: Vec<ServiceID>,
    );
}

/// Implementation of ServiceConnector.
///
/// # Rust Learning Note
///
/// ## Interior Mutability with RwLock
///
/// We use `RwLock` for `direct_closure_map` because:
/// - The connector needs to be `&self` (shared) for `connect()`
/// - But `enable_direct_closure()` needs to mutate the map
///
/// ```rust
/// pub struct ServiceConnectorImpl {
///     direct_closure_map: RwLock<HashMap<...>>,  // ← Interior mutability!
///     registry_client: Arc<ServiceRegistryClient>,
/// }
/// ```
///
/// **RwLock allows:**
/// - Multiple concurrent readers
/// - Single writer (blocks readers)
///
/// **Alternative approaches:**
/// 1. `Mutex<HashMap>` - simpler, but blocks all access during mutation
/// 2. `Arc<ServiceConnectorImpl>` + `&mut self` - requires Arc everywhere
/// 3. `RefCell<HashMap>` - not thread-safe (won't work with Send + Sync)
///
/// **We chose RwLock because:**
/// - Reads (checking direct closure) are frequent
/// - Writes (enabling direct closure) are rare (only at startup)
/// - Thread-safe
pub struct ServiceConnectorImpl {
    /// Map of modules available for direct (local) calls.
    ///
    /// RwLock provides interior mutability - we can mutate through &self.
    direct_closure_map: RwLock<HashMap<ModuleID, Vec<ServiceID>>>,
    
    /// Service registry client for discovering remote services.
    registry_client: Arc<ServiceRegistryClient>,
}

impl ServiceConnectorImpl {
    /// Creates a new service connector.
    ///
    /// # Arguments
    ///
    /// * `registry_client` - Client for service discovery
    pub fn new(registry_client: Arc<ServiceRegistryClient>) -> Self {
        Self {
            direct_closure_map: RwLock::new(HashMap::new()),
            registry_client,
        }
    }
    
    /// Checks if a service is available for direct (local) calls.
    fn has_direct_closure(&self, module_id: &ModuleID, service_id: &ServiceID) -> bool {
        self.direct_closure_map
            .read()
            .unwrap()
            .get(module_id)
            .map(|ids| ids.contains(service_id))
            .unwrap_or(false)
    }
    
    /// Connect to a remote service via gRPC.
    ///
    /// # Rust Learning Note
    ///
    /// ## Error Handling Flow
    ///
    /// Notice the `?` operator chaining:
    /// ```rust
    /// let apis = self.registry_client.discover(module_id).await?;
    /// let api = apis.iter().find(...).ok_or_else(...)?;
    /// let address = api.address.clone().ok_or_else(...)?;
    /// ```
    ///
    /// Each `?` can early-return an error. This is equivalent to Go's:
    /// ```go
    /// apis, err := client.Discover(moduleID)
    /// if err != nil { return err }
    ///
    /// api := findAPI(apis, serviceID)
    /// if api == nil { return errors.New("not found") }
    ///
    /// address := api.Address
    /// if address == "" { return errors.New("no address") }
    /// ```
    ///
    /// **Rust's `?` is more concise but equivalent!**
    async fn connect_remote(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        visitor: &mut dyn ClientConnectionVisitor,
    ) -> Result<()> {
        // 1. Discover service from registry
        trace!("Discovering service {}/{} from registry", module_id, service_id);
        let apis = self.registry_client.discover(module_id).await?;
        
        // 2. Find the specific service
        let api = apis
            .iter()
            .find(|api| api.service_id == service_id.as_str())
            .ok_or_else(|| Error::Validation {
                message: format!("Service {}/{} not found in registry", module_id, service_id),
            })?;
        
        // 3. Get address
        let address = api.address
            .clone()
            .ok_or_else(|| Error::Protocol("No address for service".to_string()))?;
        
        debug!("Discovered service {}/{} at: {}", module_id, service_id, address);
        
        // 4. Create gRPC connection
        // TODO: Support configurable options
        let options = GrpcOptions::default();
        let connection = GrpcProtocolGateway::connect(address, options).await?;
        
        debug!("✅ gRPC connection established for {}/{}", module_id, service_id);
        
        // 5. Accept visitor (double dispatch!)
        // The connection knows it's gRPC, so it will call visitor.protocol_is_grpc(...)
        connection.accept_visitor(visitor).await
    }
}

#[async_trait]
impl ServiceConnector for ServiceConnectorImpl {
    async fn connect(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        protocol: Protocol,
        visitor: &mut dyn ClientConnectionVisitor,
    ) -> Result<()> {
        let has_direct = self.has_direct_closure(module_id, service_id);
        
        match protocol {
            Protocol::Direct => {
                // Must use direct protocol
                if !has_direct {
                    return Err(Error::Validation {
                        message: format!(
                            "Service {}/{} not available for direct calls. Enable direct closure first.",
                            module_id, service_id
                        ),
                    });
                }
                
                debug!("Using direct protocol for {}/{}", module_id, service_id);
                visitor.protocol_is_direct().await
            }
            
            Protocol::Auto => {
                // Try direct first, fallback to remote
                if has_direct {
                    debug!("Auto: Trying direct protocol for {}/{}", module_id, service_id);
                    match visitor.protocol_is_direct().await {
                        Ok(()) => {
                            debug!("✅ Auto: Direct protocol succeeded for {}/{}", module_id, service_id);
                            return Ok(());
                        }
                        Err(e) => {
                            debug!("⚠️ Auto: Direct protocol failed for {}/{}, falling back to remote: {}",
                                   module_id, service_id, e);
                        }
                    }
                }
                
                // Fallback to remote (gRPC)
                debug!("Auto: Using gRPC protocol for {}/{}", module_id, service_id);
                self.connect_remote(module_id, service_id, visitor).await
            }
            
            Protocol::Grpc => {
                // Always use gRPC (even if local)
                debug!("Using gRPC protocol for {}/{}", module_id, service_id);
                self.connect_remote(module_id, service_id, visitor).await
            }
            
            Protocol::Http => {
                // HTTP not implemented yet
                error!("HTTP protocol not yet implemented for {}/{}", module_id, service_id);
                Err(Error::Protocol("HTTP protocol not yet implemented".to_string()))
            }
        }
    }
    
    fn enable_direct_closure(
        &self,
        module_id: ModuleID,
        service_ids: Vec<ServiceID>,
    ) {
        debug!("Enabling direct closure for module {}: {:?}", module_id, service_ids);
        self.direct_closure_map
            .write()
            .unwrap()
            .insert(module_id, service_ids);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connector_creation() {
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let connector = ServiceConnectorImpl::new(registry_client);
        
        // Should be created with empty direct closure map
        assert!(!connector.has_direct_closure(
            &ModuleID::from("test"),
            &ServiceID::from("service")
        ));
    }

    #[test]
    fn test_enable_direct_closure() {
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let connector = ServiceConnectorImpl::new(registry_client);
        
        let module_id = ModuleID::from("test-module");
        let service_ids = vec![
            ServiceID::from("service1"),
            ServiceID::from("service2"),
        ];
        
        connector.enable_direct_closure(module_id.clone(), service_ids);
        
        // Should now have direct closure enabled
        assert!(connector.has_direct_closure(&module_id, &ServiceID::from("service1")));
        assert!(connector.has_direct_closure(&module_id, &ServiceID::from("service2")));
        assert!(!connector.has_direct_closure(&module_id, &ServiceID::from("service3")));
    }
}

