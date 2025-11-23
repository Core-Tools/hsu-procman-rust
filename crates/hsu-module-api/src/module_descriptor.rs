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
//!     pub service_provider_factory: TypedServiceProviderFactoryFunc,
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
use hsu_common::{ModuleID, Result, Protocol, ServiceID};
use hsu_module_proto::ProtocolServer;
use crate::{ServiceConnector, Module};

/// Options for creating a service provider.
///
/// # Rust Learning Note
///
/// This struct is passed to the service provider factory function.
/// It's the same across all module types (not generic).
#[derive(Clone)]
pub struct CreateServiceProviderOptions {
    pub service_connector: Arc<dyn ServiceConnector>,
    pub protocol_servers: Vec<Arc<dyn ProtocolServer>>,
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
pub type ProtocolToServicesMap = HashMap<Protocol, Vec<ServiceID>>;

/// Options for registering handlers.
///
/// # Rust Learning Note
///
/// Generic over both SG (service gateways) and SH (service handlers).
pub struct HandlersRegistrarOptions<SH> {
    pub protocol_servers: Vec<Arc<dyn ProtocolServer>>,
    pub service_handlers: SH,
}

/// Options for enabling direct closure.
///
/// # Rust Learning Note
///
/// Generic over both SG (service gateways) and SH (service handlers).
pub struct DirectClosureEnablerOptions<SG, SH> {
    pub service_connector: Arc<dyn ServiceConnector>,
    pub service_gateways: SG,
    pub service_handlers: SH,
}

/// Factory function for creating a service provider.
///
/// # Rust Learning Note
///
/// This returns `ServiceProviderHandle` which erases the type.
/// The type safety comes from using this in `ModuleDescriptor<SP, SG, SH>`.
/// SP serves as a type marker - it's not used in the function signature but ensures
/// that the descriptor is properly typed.
pub type TypedServiceProviderFactoryFunc = 
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
pub type TypedHandlersRegistrarFunc<SH> = 
    fn(HandlersRegistrarOptions<SH>) -> Result<ProtocolToServicesMap>;

/// Function for enabling direct closure.
///
/// # Type Parameters
///
/// * `SG` - Service Gateways type
/// * `SH` - Service Handlers type
pub type TypedDirectClosureEnablerFunc<SG, SH> = 
    fn(DirectClosureEnablerOptions<SG, SH>);

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
///     handlers_registrar: None,
///     direct_closure_enabler: None,
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
    pub service_provider_factory: TypedServiceProviderFactoryFunc,
    
    /// Phantom data to enforce type safety (SP is used as a marker for type checking).
    _phantom: std::marker::PhantomData<SP>,
    
    /// Factory for creating the module.
    ///
    /// Takes the typed service provider and returns the module + handlers.
    pub module_factory: TypedModuleFactoryFunc<SP, SH>,
    
    /// Optional function for registering handlers.
    ///
    /// Only needed for server modules.
    pub handlers_registrar: Option<TypedHandlersRegistrarFunc<SH>>,
    
    /// Optional function for enabling direct closure.
    ///
    /// Allows local (in-process) service calls.
    pub direct_closure_enabler: Option<TypedDirectClosureEnablerFunc<SG, SH>>,
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
    service_provider_factory: TypedServiceProviderFactoryFunc,
    module_factory: TypedModuleFactoryFunc<SP, SH>,
    handlers_registrar: Option<TypedHandlersRegistrarFunc<SH>>,
    direct_closure_enabler: Option<TypedDirectClosureEnablerFunc<SG, SH>>,
) -> ModuleDescriptor<SP, SG, SH>
where
    SP: Send + Sync + 'static,
    SG: Send + Sync + 'static,
    SH: Send + Sync + 'static,
{
    ModuleDescriptor {
        service_provider_factory,
        module_factory,
        handlers_registrar,
        direct_closure_enabler,
        _phantom: std::marker::PhantomData,
    }
}

// Tests removed - the descriptor pattern is validated through integration tests
// in bootstrap.rs and real-world usage in the example projects

