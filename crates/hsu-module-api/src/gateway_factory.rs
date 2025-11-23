//! Gateway factory implementation with auto-protocol selection.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates the **Factory Pattern** in Rust.
//! 
//! ## Factory Pattern
//! 
//! **Purpose:** Create objects without specifying exact class.
//! 
//! **Go Version:**
//! ```go
//! type GatewayFactory interface {
//!     NewGateway(moduleID, serviceID string, protocol Protocol) (Gateway, error)
//! }
//! 
//! func (f *factoryImpl) NewGateway(...) (Gateway, error) {
//!     switch protocol {
//!     case ProtocolDirect:
//!         return &DirectGateway{...}, nil
//!     case ProtocolGRPC:
//!         return &GrpcGateway{...}, nil
//!     }
//! }
//! ```
//! 
//! **Rust Version:**
//! ```rust
//! #[async_trait]
//! trait ServiceGatewayFactory {
//!     async fn new_service_gateway(...) -> Result<ServiceGateway>;
//! }
//! 
//! // Returns enum - no interface{} needed!
//! match protocol {
//!     Protocol::Direct => ServiceGateway::Direct(...),
//!     Protocol::Grpc => ServiceGateway::Grpc(...),
//! }
//! ```

use async_trait::async_trait;
use hsu_common::{ModuleID, ServiceID, Protocol, Result, Error};
use hsu_module_management::{
    ServiceGatewayFactory, ServiceGateway,
};
use hsu_module_proto::DirectProtocol;
use std::sync::Arc;
use std::collections::HashMap;
use tracing::{debug, trace};

use crate::registry_client::ServiceRegistryClient;

/// Gateway factory implementation with auto-protocol selection.
/// 
/// # Rust Learning Note
/// 
/// ## Factory State
/// 
/// ```rust
/// pub struct ServiceGatewayFactoryImpl {
///     local_modules: Arc<HashMap<ModuleID, Arc<DirectProtocol>>>,
///     registry_client: Arc<ServiceRegistryClient>,
/// }
/// ```
/// 
/// **Key design decisions:**
/// 
/// 1. **local_modules**: Map of locally available modules
///    - Used for direct (in-process) calls
///     - Wrapped in Arc for cheap cloning
/// 
/// 2. **registry_client**: For discovering remote modules
///    - Shared across all gateway creations
///    - Also wrapped in Arc
/// 
/// ## Why Arc Everywhere?
/// 
/// The factory is shared by many modules:
/// ```
/// ModuleRuntime
/// ├── Module A ──┐
/// ├── Module B ──┼─→ GatewayFactory (Arc)
/// └── Module C ──┘        ↓
///                    Cheap clones!
/// ```
/// 
/// **Cost of sharing:** Just ~2-3 cycles per clone!
pub struct ServiceGatewayFactoryImpl {
    /// Map of locally available modules (for direct protocol).
    local_modules: Arc<HashMap<ModuleID, Arc<DirectProtocol>>>,
    
    /// Service registry client (for discovering remote modules).
    registry_client: Arc<ServiceRegistryClient>,
    
    /// User-provided gateway configurations with factories.
    /// 
    /// This is the key to matching Go's functionality!
    /// Maps: ModuleID -> Vec<GatewayConfig>
    /// where GatewayConfig contains the user's factory.
    gateway_configs: Arc<HashMap<ModuleID, Vec<hsu_module_management::GatewayConfig>>>,
}

impl ServiceGatewayFactoryImpl {
    /// Creates a new gateway factory.
    pub fn new(
        local_modules: HashMap<ModuleID, Arc<DirectProtocol>>,
        registry_client: Arc<ServiceRegistryClient>,
        gateway_configs: HashMap<ModuleID, Vec<hsu_module_management::GatewayConfig>>,
    ) -> Self {
        Self {
            local_modules: Arc::new(local_modules),
            registry_client,
            gateway_configs: Arc::new(gateway_configs),
        }
    }

    /// Auto-selects protocol based on availability.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Auto-Protocol Selection Logic
    /// 
    /// ```
    /// Protocol::Auto → Check local → Direct
    ///                              ↓
    ///                      Not local → Discover → gRPC
    /// 
    /// Protocol::Direct → Check local → Direct
    ///                                 ↓
    ///                         Not local → Error
    /// 
    /// Protocol::Grpc → Discover → gRPC
    /// ```
    /// 
    /// **Decision tree:**
    /// 1. If Protocol::Auto: Try direct first, fallback to gRPC
    /// 2. If Protocol::Direct: Must be local, error if not
    /// 3. If Protocol::Grpc: Always use gRPC (even if local)
    /// 
    /// **Why this design?**
    /// - Performance: Prefer direct when possible
    /// - Flexibility: Allow explicit protocol choice
    /// - Safety: Clear error if requirements can't be met
    async fn select_protocol(
        &self,
        module_id: &ModuleID,
        protocol: Protocol,
    ) -> Result<Protocol> {
        match protocol {
            Protocol::Auto => {
                // Try direct first
                if self.local_modules.contains_key(module_id) {
                    debug!("Auto-selecting Direct protocol for local module: {}", module_id);
                    Ok(Protocol::Direct)
                } else {
                    // Fallback to gRPC
                    debug!("Auto-selecting Grpc protocol for remote module: {}", module_id);
                    Ok(Protocol::Grpc)
                }
            }
            Protocol::Direct => {
                // Must be local
                if self.local_modules.contains_key(module_id) {
                    Ok(Protocol::Direct)
                } else {
                    Err(Error::Protocol(format!(
                        "Module {} not available locally for direct protocol",
                        module_id
                    )))
                }
            }
            // Other protocols pass through
            _ => Ok(protocol),
        }
    }

    /// Creates a direct gateway.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Direct Gateway Implementation
    /// 
    /// For direct (in-process) communication, we:
    /// 1. Get the DirectProtocol for the module (contains all service handlers)
    /// 2. Look up the specific service handler by service_id
    /// 3. Return it wrapped in ServiceGateway::Direct
    /// 
    /// **This is zero-cost** - just a HashMap lookup!
    /// 
    /// ## Why Direct Protocol Has Handlers
    /// 
    /// The DirectProtocol struct contains:
    /// ```rust
    /// handlers: Arc<HashMap<ServiceID, Arc<ServiceHandler>>>
    /// ```
    /// 
    /// This is the "service registry" for in-process calls.
    /// Each module's DirectProtocol knows about all services it provides.
    /// 
    /// ## Performance
    /// 
    /// Creating a direct gateway is incredibly cheap:
    /// - HashMap lookup: ~5 cycles
    /// - Arc clone: ~1 cycle
    /// - Enum wrap: 0 cycles (compile-time)
    /// 
    /// **Total: ~6 CPU cycles!**
    fn create_direct_gateway(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
    ) -> Result<ServiceGateway> {
        debug!("Creating direct gateway for {}/{}", module_id, service_id);
        
        // Get the DirectProtocol for this module
        let protocol = self.local_modules
            .get(module_id)
            .ok_or_else(|| Error::module_not_found(module_id.clone()))?;

        // Get the specific service handler from the protocol
        // This is a cheap operation - just a HashMap lookup + Arc clone
        let handler = protocol.get_handler(service_id)?;

        // Wrap in ServiceGateway::Direct
        // The application layer can later downcast the handler to the concrete service type
        debug!("✅ Direct gateway created successfully");
        Ok(ServiceGateway::Direct(handler))
    }

    /// Creates a gRPC gateway using user-provided factory.
    /// 
    /// # Rust Learning Note
    /// 
    /// This is where we call the user's factory - matching Go's pattern!
    /// 
    /// **Go equivalent:**
    /// ```go
    /// serviceGateway := targetGatewayFactoryFunc(connection.GatewaysClientConnection(), gf.logger)
    /// ```
    /// 
    /// **Rust version:**
    /// ```rust
    /// let gateway = factory.create_gateway(address).await?;
    /// ```
    async fn create_grpc_gateway(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
    ) -> Result<ServiceGateway> {
        debug!("Creating gRPC gateway for {}/{}", module_id, service_id);
        
        // 1. Find the user-provided gateway config for this service
        let gateway_config = self.gateway_configs
            .get(module_id)
            .and_then(|configs| {
                configs.iter().find(|cfg| &cfg.service_id == service_id)
            })
            .ok_or_else(|| Error::Validation {
                message: format!(
                    "No gateway config found for module {} service {}",
                    module_id, service_id
                )
            })?;
        
        // 2. Get the user-provided factory
        let factory = gateway_config.factory.as_ref()
            .ok_or_else(|| Error::Validation {
                message: format!(
                    "No factory provided for module {} service {} - use .with_factory()",
                    module_id, service_id
                )
            })?;
        
        // 3. Determine address (direct or from service discovery)
        let address = if let Some(direct_address) = &gateway_config.address {
            // User provided direct address (skips service discovery)
            debug!("Using direct address: {}", direct_address);
            direct_address.clone()
        } else {
            // Discover service from registry
            debug!("Discovering service from registry...");
            let apis = self.registry_client
                .discover(module_id)
                .await?;

            // Find the specific service
            let api = apis
                .iter()
                .find(|api| api.service_id == service_id.as_str())
                .ok_or_else(|| Error::service_not_found(module_id.clone(), service_id.clone()))?;

            // Get address
            let discovered_address = api.address
                .clone()
                .ok_or_else(|| Error::Protocol("No address for gRPC service".to_string()))?;
            
            debug!("Discovered service at: {}", discovered_address);
            discovered_address
        };

        // 4. Call user-provided factory to create the gateway!
        // This is the key moment - we delegate to user code.
        debug!("Calling user factory to create gateway...");
        let gateway = factory.create_gateway(address).await?;
        
        debug!("✅ Gateway created successfully via user factory");
        Ok(gateway)
    }
}

#[async_trait]
impl ServiceGatewayFactory for ServiceGatewayFactoryImpl {
    /// Creates a new service gateway with auto-protocol selection.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## The Magic Moment!
    /// 
    /// This is where everything comes together:
    /// 
    /// ```rust
    /// // User calls:
    /// let gateway = factory.new_service_gateway(
    ///     &module_id,
    ///     &service_id,
    ///     Protocol::Auto,  // ← Auto-select!
    /// ).await?;
    /// 
    /// // Factory:
    /// // 1. Checks if module is local
    /// // 2. If yes: Returns ServiceGateway::Direct (zero-cost!)
    /// // 3. If no: Discovers from registry, returns ServiceGateway::Grpc
    /// 
    /// // User doesn't care which - both implement the same interface!
    /// let response = gateway.call(...).await?;
    /// ```
    /// 
    /// **This is the power of abstraction:**
    /// - User: Simple API
    /// - Factory: Smart selection
    /// - Runtime: Optimal performance
    async fn new_service_gateway(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        protocol: Protocol,
    ) -> Result<ServiceGateway> {
        trace!(
            "Creating gateway: module={}, service={}, protocol={:?}",
            module_id, service_id, protocol
        );

        // Select actual protocol
        let selected_protocol = self.select_protocol(module_id, protocol).await?;

        // Create appropriate gateway
        match selected_protocol {
            Protocol::Direct => {
                debug!("Creating direct gateway for {}/{}", module_id, service_id);
                self.create_direct_gateway(module_id, service_id)
            }
            Protocol::Grpc => {
                debug!("Creating gRPC gateway for {}/{}", module_id, service_id);
                self.create_grpc_gateway(module_id, service_id).await
            }
            Protocol::Http => {
                // HTTP not implemented yet
                Err(Error::Protocol("HTTP protocol not yet implemented".to_string()))
            }
            Protocol::Auto => {
                // Should have been resolved by select_protocol
                unreachable!("Protocol::Auto should be resolved")
            }
        }
    }
}

// NOTE: No dummy services needed!
// The framework is now completely domain-agnostic.
// Domain-specific services belong in the application layer (Layer 2 & Layer 5).

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factory_creation() {
        let local_modules = HashMap::new();
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let gateway_configs = HashMap::new();
        
        let factory = ServiceGatewayFactoryImpl::new(local_modules, registry_client, gateway_configs);
        
        // Factory should be created successfully
        assert_eq!(factory.local_modules.len(), 0);
    }

    #[tokio::test]
    async fn test_auto_protocol_selection_local() {
        let mut local_modules = HashMap::new();
        let module_id = ModuleID::from("test-module");
        
        // Add a local module
        let protocol = Arc::new(DirectProtocol::new(HashMap::new()));
        local_modules.insert(module_id.clone(), protocol);
        
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let gateway_configs = HashMap::new();
        let factory = ServiceGatewayFactoryImpl::new(local_modules, registry_client, gateway_configs);
        
        // Should select Direct for local module
        let selected = factory.select_protocol(&module_id, Protocol::Auto).await.unwrap();
        assert_eq!(selected, Protocol::Direct);
    }

    #[tokio::test]
    async fn test_auto_protocol_selection_remote() {
        let local_modules = HashMap::new();
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let gateway_configs = HashMap::new();
        let factory = ServiceGatewayFactoryImpl::new(local_modules, registry_client, gateway_configs);
        
        let module_id = ModuleID::from("remote-module");
        
        // Should select Grpc for remote module
        let selected = factory.select_protocol(&module_id, Protocol::Auto).await.unwrap();
        assert_eq!(selected, Protocol::Grpc);
    }

    #[tokio::test]
    async fn test_direct_protocol_requires_local() {
        let local_modules = HashMap::new();
        let registry_client = Arc::new(ServiceRegistryClient::new("http://localhost:8080"));
        let gateway_configs = HashMap::new();
        let factory = ServiceGatewayFactoryImpl::new(local_modules, registry_client, gateway_configs);
        
        let module_id = ModuleID::from("remote-module");
        
        // Should fail - module not local
        let result = factory.select_protocol(&module_id, Protocol::Direct).await;
        assert!(result.is_err());
    }
}

