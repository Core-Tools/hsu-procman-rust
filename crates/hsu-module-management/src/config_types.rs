//! Configuration types for module management runtime.
//!
//! # Rust Learning Note
//!
//! This module defines **configuration structures** for the runtime.
//!
//! ## Design Pattern: Builder + Configuration Structs
//!
//! Instead of passing many parameters to functions, we use configuration structs:
//! ```rust
//! // Bad: Too many parameters
//! runtime.configure(url, port, protocol, handlers, options, ...);
//!
//! // Good: Configuration struct
//! let config = RuntimeConfig {
//!     server_options: vec![...],
//!     handlers_configs: vec![...],
//!     ...
//! };
//! runtime.configure(config);
//! ```
//!
//! ## Golang Comparison
//!
//! **Golang:**
//! ```go
//! type RuntimeOptions struct {
//!     ServerOptions         ServerOptionsList
//!     ModuleHandlersConfigs ModuleHandlersConfigList
//!     ClientOptions         ClientOptionsMap
//!     // ...
//! }
//! ```
//!
//! **Rust:**
//! ```rust
//! pub struct RuntimeConfig {
//!     pub server_options: Vec<ServerOptions>,
//!     pub handlers_configs: Vec<HandlersConfig>,
//!     // ...
//! }
//! ```
//!
//! Very similar, but Rust uses concrete types (Vec) instead of type aliases.

use hsu_common::{Protocol, ModuleID, ServiceID};
use std::collections::HashMap;

/// Options for creating a protocol server.
///
/// # Rust Learning Note
///
/// ## Server Configuration
///
/// This struct tells the runtime:
/// - What protocol to use (gRPC, HTTP, etc.)
/// - What port to listen on (0 = dynamic)
/// - Additional protocol-specific options
///
/// Example:
/// ```rust
/// let options = ServerOptions {
///     server_id: "grpc-main".to_string(),
///     protocol: Protocol::Grpc,
///     port: 50051,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Unique identifier for this server instance.
    pub server_id: String,
    
    /// Protocol this server implements.
    pub protocol: Protocol,
    
    /// Port to listen on (0 = dynamic allocation).
    pub port: u16,
}

impl ServerOptions {
    /// Creates new server options.
    pub fn new(server_id: impl Into<String>, protocol: Protocol, port: u16) -> Self {
        Self {
            server_id: server_id.into(),
            protocol,
            port,
        }
    }
    
    /// Builder-style method to set server ID.
    pub fn with_server_id(mut self, server_id: impl Into<String>) -> Self {
        self.server_id = server_id.into();
        self
    }
    
    /// Builder-style method to set protocol.
    pub fn with_protocol(mut self, protocol: Protocol) -> Self {
        self.protocol = protocol;
        self
    }
    
    /// Builder-style method to set port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

/// Configuration for registering module handlers with a protocol server.
///
/// # Rust Learning Note
///
/// ## Handler Registration
///
/// This struct tells the runtime:
/// - Which module's handlers to register
/// - Which protocol server to register them with
/// - How to perform the registration
///
/// Example:
/// ```rust
/// let config = HandlersConfig {
///     module_id: ModuleID::from("echo"),
///     server_id: "grpc-main".to_string(),
///     protocol: Protocol::Grpc,
/// };
/// ```
///
/// The runtime will:
/// 1. Find the module by `module_id`
/// 2. Get its service handlers
/// 3. Find the server by `server_id`
/// 4. Register handlers with that server
#[derive(Debug, Clone)]
pub struct HandlersConfig {
    /// ID of the module providing the handlers.
    pub module_id: ModuleID,
    
    /// ID of the server to register handlers with.
    pub server_id: String,
    
    /// Protocol being used (must match server's protocol).
    pub protocol: Protocol,
}

impl HandlersConfig {
    /// Creates new handlers configuration.
    pub fn new(
        module_id: ModuleID,
        server_id: impl Into<String>,
        protocol: Protocol,
    ) -> Self {
        Self {
            module_id,
            server_id: server_id.into(),
            protocol,
        }
    }
}

/// Configuration for creating service gateways.
///
/// # Rust Learning Note
///
/// ## Gateway Configuration
///
/// This specifies how to create gateways for calling remote services:
/// - Which service to call
/// - What protocol to use
/// - User-provided factory for creating the gateway
/// - Any protocol-specific options
///
/// Example:
/// ```rust,ignore
/// let config = GatewayConfig::new(ServiceID::from("echo-service"), Protocol::Grpc)
///     .with_factory(Arc::new(EchoGrpcGatewayFactory));
/// ```
///
/// ## Factory Pattern
///
/// The factory is a trait object that implements `ProtocolGatewayFactory`.
/// The framework calls `factory.create_gateway(address)` to create the
/// service-specific gateway implementation.
///
/// **This matches the Go pattern:**
/// ```go
/// GatewayFactoryFunc: grpcapi.NewGRPCGateway1
/// ```
#[derive(Clone)]
pub struct GatewayConfig {
    /// ID of the service this gateway calls.
    pub service_id: ServiceID,
    
    /// Protocol to use for communication.
    pub protocol: Protocol,
    
    /// Optional direct address (skips service discovery).
    pub address: Option<String>,
    
    /// User-provided factory for creating the gateway.
    /// 
    /// This is the key addition that matches Go's `GatewayFactoryFunc`.
    /// The framework will call this factory to create the actual gateway.
    pub factory: Option<std::sync::Arc<dyn crate::module_types::ProtocolGatewayFactory>>,
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("service_id", &self.service_id)
            .field("protocol", &self.protocol)
            .field("address", &self.address)
            .field("factory", &if self.factory.is_some() { "Some(...)" } else { "None" })
            .finish()
    }
}

impl GatewayConfig {
    /// Creates new gateway configuration.
    pub fn new(service_id: ServiceID, protocol: Protocol) -> Self {
        Self {
            service_id,
            protocol,
            address: None,
            factory: None,
        }
    }
    
    /// Sets a direct address (skips service discovery).
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }
    
    /// Sets the user-provided factory for creating gateways.
    /// 
    /// # Rust Learning Note
    /// 
    /// This builder method allows users to provide their own gateway factory,
    /// matching Go's `GatewayFactoryFunc` pattern.
    /// 
    /// ## Example
    /// 
    /// ```rust,ignore
    /// use hsu_module_management::{GatewayConfig, ProtocolGatewayFactory};
    /// 
    /// let config = GatewayConfig::new(ServiceID::from("echo-service"), Protocol::Grpc)
    ///     .with_factory(Arc::new(EchoGrpcGatewayFactory));
    /// ```
    /// 
    /// The framework will call `factory.create_gateway(address)` when
    /// creating gateways for this service.
    pub fn with_factory(
        mut self, 
        factory: std::sync::Arc<dyn crate::module_types::ProtocolGatewayFactory>
    ) -> Self {
        self.factory = Some(factory);
        self
    }
}

/// Map of module ID to gateway configurations.
///
/// # Rust Learning Note
///
/// ## Type Alias for Clarity
///
/// ```rust
/// pub type GatewayConfigMap = HashMap<ModuleID, Vec<GatewayConfig>>;
/// ```
///
/// This is a **type alias** - it doesn't create a new type, just gives
/// a shorter name to an existing type.
///
/// Why use it?
/// - More readable: `GatewayConfigMap` vs `HashMap<ModuleID, Vec<GatewayConfig>>`
/// - Self-documenting: tells us the purpose
/// - Easier to refactor: change definition in one place
///
/// **Golang equivalent:** `type GatewayConfigMap map[ModuleID][]GatewayConfig`
pub type GatewayConfigMap = HashMap<ModuleID, Vec<GatewayConfig>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_options_creation() {
        let options = ServerOptions::new("test-server", Protocol::Grpc, 50051);
        
        assert_eq!(options.server_id, "test-server");
        assert_eq!(options.protocol, Protocol::Grpc);
        assert_eq!(options.port, 50051);
    }

    #[test]
    fn test_server_options_builder() {
        let options = ServerOptions::new("old-id", Protocol::Grpc, 8080)
            .with_server_id("new-id")
            .with_protocol(Protocol::Auto)
            .with_port(9090);
        
        assert_eq!(options.server_id, "new-id");
        assert_eq!(options.protocol, Protocol::Auto);
        assert_eq!(options.port, 9090);
    }

    #[test]
    fn test_handlers_config_creation() {
        let config = HandlersConfig::new(
            ModuleID::from("echo"),
            "grpc-server",
            Protocol::Grpc,
        );
        
        assert_eq!(config.module_id, ModuleID::from("echo"));
        assert_eq!(config.server_id, "grpc-server");
        assert_eq!(config.protocol, Protocol::Grpc);
    }

    #[test]
    fn test_gateway_config_creation() {
        let config = GatewayConfig::new(
            ServiceID::from("echo-service"),
            Protocol::Grpc,
        );
        
        assert_eq!(config.service_id, ServiceID::from("echo-service"));
        assert_eq!(config.protocol, Protocol::Grpc);
        assert!(config.address.is_none());
    }

    #[test]
    fn test_gateway_config_with_address() {
        let config = GatewayConfig::new(
            ServiceID::from("echo-service"),
            Protocol::Grpc,
        )
        .with_address("localhost:50051");
        
        assert_eq!(config.address, Some("localhost:50051".to_string()));
    }

    #[test]
    fn test_gateway_config_map() {
        let mut map: GatewayConfigMap = HashMap::new();
        
        let configs = vec![
            GatewayConfig::new(ServiceID::from("service1"), Protocol::Grpc),
            GatewayConfig::new(ServiceID::from("service2"), Protocol::Grpc),
        ];
        
        map.insert(ModuleID::from("module1"), configs);
        
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&ModuleID::from("module1")).unwrap().len(), 2);
    }
}

