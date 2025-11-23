//! Module Descriptor - Type-Safe Module Registration
//!
//! # Architecture
//!
//! This module provides the `ModuleDescriptor` pattern for type-safe,
//! self-registering modules that match the Go implementation.
//!
//! ## The Pattern
//!
//! Modules register themselves at startup using generic descriptors:
//!
//! ```text
//! Module Crate (echo-client)
//!     ↓
//! mod init {
//!     use once_cell::sync::Lazy;
//!     
//!     static INIT: Lazy<()> = Lazy::new(|| {
//!         register_module(
//!             ModuleID::from("echo-client"),
//!             ModuleDescriptor {
//!                 service_provider_factory: new_service_provider,
//!                 module_factory: new_module,
//!                 handlers_registrar_factory: None,
//!                 direct_closure_enable: Some(enable_direct_closure),
//!             },
//!         );
//!     });
//!     
//!     pub fn init() {
//!         Lazy::force(&INIT);
//!     }
//! }
//! ```
//!
//! Then in main:
//! ```text
//! fn main() {
//!     echo_client::init();  // Register module
//!     
//!     let runtime = ModuleRuntime::new(...);
//!     runtime.create_module("echo-client")?;  // Type-safe!
//! }
//! ```
//!
//! ## Comparison with Go
//!
//! **Go version:**
//! ```go
//! type ModuleDescriptor[SP any, SG any, SH any] struct {
//!     ServiceProviderFactoryFunc   func(serviceConnector, logger) ServiceProviderHandle
//!     ModuleFactoryFunc            func(serviceProvider SP, logger) (Module, SH)
//!     HandlersRegistrarFactoryFunc func(protocolServers, logger) (HandlersRegistrar[SH], error)
//!     DirectClosureEnableFunc      func(DirectClosureEnableOptions[SG, SH])
//! }
//!
//! func RegisterModule[SP any, SG any, SH any](
//!     moduleID ModuleID,
//!     descriptor ModuleDescriptor[SP, SG, SH],
//! ) { /* ... */ }
//! ```
//!
//! **Rust version (this file):**
//! ```rust
//! pub struct ModuleDescriptor<SP, SG, SH>
//! where
//!     SP: Send + Sync + 'static,
//!     SG: Send + Sync + 'static,
//!     SH: Send + Sync + 'static,
//! {
//!     pub service_provider_factory: TypedServiceProviderFactoryFunc<SP>,
//!     pub module_factory: TypedModuleFactoryFunc<SP, SH>,
//!     pub handlers_registrar_factory: Option<TypedHandlersRegistrarFactoryFunc<SH>>,
//!     pub direct_closure_enable: Option<TypedDirectClosureEnableFunc<SG, SH>>,
//! }
//!
//! pub fn register_module<SP, SG, SH>(
//!     module_id: ModuleID,
//!     descriptor: ModuleDescriptor<SP, SG, SH>,
//! ) { /* ... */ }
//! ```
//!
//! Nearly identical! Main differences:
//! - Rust: Explicit `Send + Sync + 'static` bounds (Go infers)
//! - Rust: `Option<T>` for optional functions (Go uses nil)

use std::sync::Arc;
use std::any::Any;
use std::collections::HashMap;
use hsu_common::{ModuleID, Result};
use hsu_module_api::ServiceConnector;
use crate::{Module, ProtocolServer};

/// Options for creating a service provider.
///
/// # Rust Learning Note
///
/// This struct is passed to the service provider factory function.
/// It's the same across all module types (not generic).
#[derive(Clone)]
pub struct CreateServiceProviderOptions {
    pub service_connector: Arc<dyn ServiceConnector>,
}

/// Handle returned by service provider factory.
///
/// Contains both the service provider and its associated service gateways.
///
/// # Rust Learning Note
///
/// Uses `Box<dyn Any>` for type erasure - the descriptor stores typed values,
/// but the registry needs to store different types in the same map.
pub struct ServiceProviderHandle {
    /// The service provider (type-erased).
    pub service_provider: Box<dyn Any + Send + Sync>,
    
    /// Map of service gateways by target module ID (type-erased).
    pub service_gateways_map: HashMap<ModuleID, Box<dyn Any + Send + Sync>>,
}

/// Options for creating a module.
///
/// # Rust Learning Note
///
/// This struct contains all dependencies needed to create a module.
/// The module factory will downcast `service_provider` to the expected type.
pub struct CreateModuleOptions {
    pub service_connector: Arc<dyn ServiceConnector>,
    pub service_provider: Box<dyn Any + Send + Sync>,
    pub service_gateways: Vec<Box<dyn Any + Send + Sync>>,
    pub protocol_servers: Vec<Arc<dyn ProtocolServer>>,
}

/// Map of protocols to service IDs.
///
/// Returned by `HandlersRegistrar::register_handlers()`.
pub type ProtocolToServicesMap = HashMap<crate::Protocol, Vec<crate::ServiceID>>;

/// Trait for registering service handlers with protocol servers.
///
/// # Rust Learning Note
///
/// This is a trait (not generic struct) because different modules
/// have different handler types (SH).
pub trait HandlersRegistrar<SH>: Send + Sync {
    /// Registers handlers with protocol servers.
    ///
    /// Returns a map of which services are available on which protocols.
    fn register_handlers(&self, handlers: SH) -> Result<ProtocolToServicesMap>;
}

/// Options for enabling direct closure.
///
/// # Rust Learning Note
///
/// Generic over both SG (service gateways) and SH (service handlers).
pub struct DirectClosureEnableOptions<SG, SH> {
    pub service_connector: Arc<dyn ServiceConnector>,
    pub service_gateways: SG,
    pub service_handlers: SH,
}

/// Factory function for creating a service provider.
///
/// # Type Parameters
///
/// * `SP` - Service Provider type (e.g., `Arc<dyn EchoServiceGateways>`)
pub type TypedServiceProviderFactoryFunc<SP> = 
    fn(Arc<dyn ServiceConnector>) -> ServiceProviderHandle;

/// Factory function for creating a module.
///
/// # Type Parameters
///
/// * `SP` - Service Provider type
/// * `SH` - Service Handlers type
pub type TypedModuleFactoryFunc<SP, SH> = 
    fn(SP) -> (Box<dyn Module>, SH);

/// Factory function for creating a handlers registrar.
///
/// # Type Parameters
///
/// * `SH` - Service Handlers type
pub type TypedHandlersRegistrarFactoryFunc<SH> = 
    fn(Vec<Arc<dyn ProtocolServer>>) -> Result<Box<dyn HandlersRegistrar<SH>>>;

/// Function for enabling direct closure.
///
/// # Type Parameters
///
/// * `SG` - Service Gateways type
/// * `SH` - Service Handlers type
pub type TypedDirectClosureEnableFunc<SG, SH> = 
    fn(DirectClosureEnableOptions<SG, SH>);

/// Module descriptor with generic type parameters.
///
/// # Type Parameters
///
/// * `SP` - Service Provider type (what the module needs)
/// * `SG` - Service Gateways type (what the module provides to others)
/// * `SH` - Service Handlers type (handlers for server-side)
///
/// # Example
///
/// ```rust,ignore
/// use hsu_module_management::{ModuleDescriptor, register_module};
///
/// // Client module descriptor
/// let descriptor = ModuleDescriptor {
///     service_provider_factory: |connector| {
///         let gateways = new_echo_service_gateways(
///             ModuleID::from("echo"),
///             connector,
///         );
///         ServiceProviderHandle {
///             service_provider: Box::new(gateways.clone()),
///             service_gateways_map: HashMap::new(),
///         }
///     },
///     module_factory: |provider| {
///         let module = Box::new(EchoClientModule::new(provider));
///         let handlers = (); // No handlers for client
///         (module, handlers)
///     },
///     handlers_registrar_factory: None,
///     direct_closure_enable: None,
/// };
///
/// register_module(ModuleID::from("echo-client"), descriptor);
/// ```
pub struct ModuleDescriptor<SP, SG, SH>
where
    SP: Send + Sync + 'static,
    SG: Send + Sync + 'static,
    SH: Send + Sync + 'static,
{
    /// Factory for creating the service provider.
    ///
    /// This is called first, before any modules are created.
    pub service_provider_factory: TypedServiceProviderFactoryFunc<SP>,
    
    /// Factory for creating the module.
    ///
    /// Takes the typed service provider and returns the module + handlers.
    pub module_factory: TypedModuleFactoryFunc<SP, SH>,
    
    /// Optional factory for creating a handlers registrar.
    ///
    /// Only needed for server modules.
    pub handlers_registrar_factory: Option<TypedHandlersRegistrarFactoryFunc<SH>>,
    
    /// Optional function for enabling direct closure.
    ///
    /// Allows local (in-process) service calls.
    pub direct_closure_enable: Option<TypedDirectClosureEnableFunc<SG, SH>>,
}

/// Creates a new module descriptor.
///
/// # Example
///
/// ```rust,ignore
/// let descriptor = new_module_descriptor(
///     new_service_provider,
///     new_module,
///     None,  // No handlers registrar
///     None,  // No direct closure
/// );
/// ```
pub fn new_module_descriptor<SP, SG, SH>(
    service_provider_factory: TypedServiceProviderFactoryFunc<SP>,
    module_factory: TypedModuleFactoryFunc<SP, SH>,
    handlers_registrar_factory: Option<TypedHandlersRegistrarFactoryFunc<SH>>,
    direct_closure_enable: Option<TypedDirectClosureEnableFunc<SG, SH>>,
) -> ModuleDescriptor<SP, SG, SH>
where
    SP: Send + Sync + 'static,
    SG: Send + Sync + 'static,
    SH: Send + Sync + 'static,
{
    ModuleDescriptor {
        service_provider_factory,
        module_factory,
        handlers_registrar_factory,
        direct_closure_enable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_descriptor_creation() {
        // Define simple types
        type SP = ();
        type SG = ();
        type SH = ();
        
        // Create descriptor
        let descriptor: ModuleDescriptor<SP, SG, SH> = ModuleDescriptor {
            service_provider_factory: |_| ServiceProviderHandle {
                service_provider: Box::new(()),
                service_gateways_map: HashMap::new(),
            },
            module_factory: |_| {
                struct DummyModule;
                impl Module for DummyModule {
                    fn module_id(&self) -> ModuleID {
                        ModuleID::from("test")
                    }
                    fn start(&mut self) -> Result<()> {
                        Ok(())
                    }
                    fn stop(&mut self) -> Result<()> {
                        Ok(())
                    }
                }
                (Box::new(DummyModule), ())
            },
            handlers_registrar_factory: None,
            direct_closure_enable: None,
        };
        
        // Use descriptor
        let _handle = (descriptor.service_provider_factory)(
            Arc::new(crate::service_connector_mock())
        );
    }
    
    // Mock service connector for testing
    fn service_connector_mock() -> impl ServiceConnector {
        struct MockConnector;
        
        #[async_trait::async_trait]
        impl ServiceConnector for MockConnector {
            async fn connect(
                &self,
                _module_id: &ModuleID,
                _service_id: &crate::ServiceID,
                _protocol: crate::Protocol,
                _visitor: &mut dyn hsu_module_proto::ClientConnectionVisitor,
            ) -> Result<()> {
                unimplemented!()
            }
            
            fn enable_direct_closure(&mut self, _module_id: ModuleID, _service_ids: Vec<crate::ServiceID>) {
                unimplemented!()
            }
        }
        
        MockConnector
    }
}

