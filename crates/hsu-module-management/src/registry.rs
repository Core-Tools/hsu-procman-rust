//! Global Module Registry - Type-Erased Module Storage
//!
//! # Architecture
//!
//! This module provides a global registry for module factories.
//! Modules register themselves at startup, and the runtime creates them on demand.
//!
//! ## The Flow
//!
//! ```text
//! Startup (before main)
//! ├── echo_client::init()
//! │   └── register_module("echo-client", descriptor)
//! │       └── Store type-erased factories in global registry
//! └── echo_server::init()
//!     └── register_module("echo-server", descriptor)
//!
//! Runtime
//! ├── create_service_provider("echo-client", options)
//! │   └── Lookup factory, call it, return handle
//! └── create_module("echo-client", options)
//!     ├── Lookup factory
//!     ├── Downcast service provider to expected type
//!     ├── Create module + handlers
//!     ├── Register handlers (if server)
//!     ├── Enable direct closure (if applicable)
//!     └── Return module + protocol map
//! ```
//!
//! ## Comparison with Go
//!
//! **Go version:**
//! ```go
//! var (
//!     globalModuleFactoryRegistry          = make(map[moduletypes.ModuleID]ModuleFactoryFunc)
//!     globalServiceProviderFactoryRegistry = make(map[moduletypes.ModuleID]ServiceProviderFactoryFunc)
//!     globalRegistryLock                   sync.RWMutex
//! )
//!
//! func RegisterModule[SP any, SG any, SH any](
//!     moduleID moduletypes.ModuleID,
//!     descriptor ModuleDescriptor[SP, SG, SH],
//! ) { /* ... */ }
//! ```
//!
//! **Rust version (this file):**
//! ```rust
//! lazy_static! {
//!     static ref MODULE_REGISTRY: RwLock<HashMap<ModuleID, ModuleFactoryFn>> = 
//!         RwLock::new(HashMap::new());
//!     static ref SERVICE_PROVIDER_REGISTRY: RwLock<HashMap<ModuleID, ServiceProviderFactoryFn>> = 
//!         RwLock::new(HashMap::new());
//! }
//!
//! pub fn register_module<SP, SG, SH>(
//!     module_id: ModuleID,
//!     descriptor: ModuleDescriptor<SP, SG, SH>,
//! ) { /* ... */ }
//! ```
//!
//! **Key Difference:**  
//! Go uses `sync.RWMutex`, Rust uses `lazy_static! + RwLock`.  
//! Same pattern, different syntax!

use std::sync::{Arc, RwLock};
use std::collections::HashMap;
use std::any::Any;
use hsu_common::{ModuleID, Result, Error};
use lazy_static::lazy_static;
use tracing::{debug, trace, error};

use crate::module_descriptor::{
    ModuleDescriptor, CreateModuleOptions, CreateServiceProviderOptions,
    ServiceProviderHandle, ProtocolToServicesMap, DirectClosureEnableOptions,
};
use crate::Module;

/// Type-erased module factory function.
///
/// # Rust Learning Note
///
/// This is stored in the global registry. It erases the generic types (SP, SG, SH)
/// so we can store different module types in the same HashMap.
pub type ModuleFactoryFn = fn(CreateModuleOptions) -> Result<(Box<dyn Module>, ProtocolToServicesMap)>;

/// Type-erased service provider factory function.
pub type ServiceProviderFactoryFn = fn(CreateServiceProviderOptions) -> ServiceProviderHandle;

lazy_static! {
    /// Global registry of module factories.
    ///
    /// # Rust Learning Note
    ///
    /// `lazy_static!` provides global, lazily-initialized statics.
    /// `RwLock` allows multiple concurrent readers or one writer.
    static ref MODULE_REGISTRY: RwLock<HashMap<ModuleID, ModuleFactoryFn>> = 
        RwLock::new(HashMap::new());
    
    /// Global registry of service provider factories.
    static ref SERVICE_PROVIDER_REGISTRY: RwLock<HashMap<ModuleID, ServiceProviderFactoryFn>> = 
        RwLock::new(HashMap::new());
}

/// Registers a module with the global registry.
///
/// # Type Parameters
///
/// * `SP` - Service Provider type
/// * `SG` - Service Gateways type
/// * `SH` - Service Handlers type
///
/// # Example
///
/// ```rust,ignore
/// use hsu_module_management::{register_module, ModuleDescriptor};
///
/// fn init() {
///     register_module(
///         ModuleID::from("echo-client"),
///         ModuleDescriptor {
///             service_provider_factory: new_service_provider,
///             module_factory: new_module,
///             handlers_registrar_factory: None,
///             direct_closure_enable: None,
///         },
///     );
/// }
/// ```
///
/// # Rust Learning Note
///
/// This function is generic at compile time, but stores type-erased
/// functions in the registry. The type casting happens here!
pub fn register_module<SP, SG, SH>(
    module_id: ModuleID,
    descriptor: ModuleDescriptor<SP, SG, SH>,
) where
    SP: Clone + Send + Sync + 'static,
    SG: Clone + Send + Sync + 'static,
    SH: Clone + Send + Sync + 'static,
{
    debug!("[Registry] Registering module: {}", module_id);
    
    // Create type-erased service provider factory
    let sp_factory = move |options: CreateServiceProviderOptions| -> ServiceProviderHandle {
        trace!("[Registry] Creating service provider for: {}", module_id);
        (descriptor.service_provider_factory)(options.service_connector)
    };
    
    // Create type-erased module factory with type casting
    let module_factory = move |options: CreateModuleOptions| -> Result<(Box<dyn Module>, ProtocolToServicesMap)> {
        trace!("[Registry] Creating module: {}", module_id);
        
        // ✅ Type cast happens HERE (once, centralized!)
        let typed_sp = options.service_provider
            .downcast_ref::<SP>()
            .ok_or_else(|| {
                error!(
                    "[Registry] Type mismatch for module '{}': expected {}, got different type",
                    module_id,
                    std::any::type_name::<SP>()
                );
                Error::Validation {
                    message: format!(
                        "Type mismatch for module '{}': expected provider of type {}, got different type",
                        module_id,
                        std::any::type_name::<SP>()
                    ),
                }
            })?;
        
        // ✅ Now type-safe!
        trace!("[Registry] Service provider type cast successful");
        let (module, handlers) = (descriptor.module_factory)(typed_sp.clone());
        
        // Register handlers if needed (server modules)
        let mut protocol_map = HashMap::new();
        if let Some(registrar_factory) = descriptor.handlers_registrar_factory {
            trace!("[Registry] Creating handlers registrar for: {}", module_id);
            let registrar = registrar_factory(options.protocol_servers)?;
            protocol_map = registrar.register_handlers(handlers.clone())?;
            debug!("[Registry] Handlers registered for {}: {:?}", module_id, protocol_map.keys());
        }
        
        // Enable direct closure if needed
        if let Some(enable_fn) = descriptor.direct_closure_enable {
            trace!("[Registry] Enabling direct closure for: {}", module_id);
            for sg in options.service_gateways {
                let typed_sg = sg.downcast_ref::<SG>()
                    .ok_or_else(|| {
                        error!(
                            "[Registry] Type mismatch for service gateways in module '{}'",
                            module_id
                        );
                        Error::Validation {
                            message: format!(
                                "Type mismatch for module '{}': expected service gateways of type {}",
                                module_id,
                                std::any::type_name::<SG>()
                            ),
                        }
                    })?;
                
                let opts = DirectClosureEnableOptions {
                    service_connector: options.service_connector.clone(),
                    service_gateways: typed_sg.clone(),
                    service_handlers: handlers.clone(),
                };
                enable_fn(opts);
                debug!("[Registry] Direct closure enabled for: {}", module_id);
            }
        }
        
        debug!("[Registry] ✅ Module created successfully: {}", module_id);
        Ok((module, protocol_map))
    };
    
    // Store in global registry
    MODULE_REGISTRY.write().unwrap().insert(module_id.clone(), module_factory);
    SERVICE_PROVIDER_REGISTRY.write().unwrap().insert(module_id.clone(), sp_factory);
    
    debug!("[Registry] ✅ Module registered: {}", module_id);
}

/// Gets a module factory by module ID.
///
/// Returns an error if the module is not registered.
pub fn get_module_factory(module_id: &ModuleID) -> Result<ModuleFactoryFn> {
    let registry = MODULE_REGISTRY.read().unwrap();
    registry.get(module_id)
        .copied()
        .ok_or_else(|| Error::Validation {
            message: format!("Unknown module ID: {}", module_id),
        })
}

/// Creates a module using the registered factory.
///
/// # Example
///
/// ```rust,ignore
/// let options = CreateModuleOptions {
///     service_connector,
///     service_provider: Box::new(provider),
///     service_gateways: vec![],
///     protocol_servers: vec![],
/// };
///
/// let (module, protocol_map) = create_module(&ModuleID::from("echo-client"), options)?;
/// ```
pub fn create_module(
    module_id: &ModuleID,
    options: CreateModuleOptions,
) -> Result<(Box<dyn Module>, ProtocolToServicesMap)> {
    debug!("[Registry] Creating module: {}", module_id);
    let factory = get_module_factory(module_id)?;
    factory(options)
}

/// Gets a service provider factory by module ID.
///
/// Returns an error if the module is not registered.
pub fn get_service_provider_factory(module_id: &ModuleID) -> Result<ServiceProviderFactoryFn> {
    let registry = SERVICE_PROVIDER_REGISTRY.read().unwrap();
    registry.get(module_id)
        .copied()
        .ok_or_else(|| Error::Validation {
            message: format!("Unknown service provider factory: {}", module_id),
        })
}

/// Creates a service provider using the registered factory.
///
/// # Example
///
/// ```rust,ignore
/// let options = CreateServiceProviderOptions {
///     service_connector,
/// };
///
/// let handle = create_service_provider(&ModuleID::from("echo-client"), options)?;
/// ```
pub fn create_service_provider(
    module_id: &ModuleID,
    options: CreateServiceProviderOptions,
) -> Result<ServiceProviderHandle> {
    debug!("[Registry] Creating service provider for: {}", module_id);
    let factory = get_service_provider_factory(module_id)?;
    Ok(factory(options))
}

/// Lists all registered module IDs.
///
/// Useful for debugging and diagnostics.
pub fn list_registered_modules() -> Vec<ModuleID> {
    let registry = MODULE_REGISTRY.read().unwrap();
    registry.keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_descriptor::{ModuleDescriptor, ServiceProviderHandle};
    use hsu_module_api::ServiceConnector;
    use std::sync::Arc;

    #[test]
    fn test_register_and_create_module() {
        // Define simple types
        type SP = Arc<String>;
        type SG = ();
        type SH = ();
        
        let test_module_id = ModuleID::from("test-module");
        
        // Create descriptor
        let descriptor: ModuleDescriptor<SP, SG, SH> = ModuleDescriptor {
            service_provider_factory: |_connector| {
                let provider = Arc::new("test-provider".to_string());
                ServiceProviderHandle {
                    service_provider: Box::new(provider),
                    service_gateways_map: HashMap::new(),
                }
            },
            module_factory: |provider| {
                struct DummyModule {
                    provider_name: String,
                }
                impl Module for DummyModule {
                    fn module_id(&self) -> ModuleID {
                        ModuleID::from("test-module")
                    }
                    fn start(&mut self) -> Result<()> {
                        Ok(())
                    }
                    fn stop(&mut self) -> Result<()> {
                        Ok(())
                    }
                }
                let module = Box::new(DummyModule {
                    provider_name: (*provider).clone(),
                });
                (module, ())
            },
            handlers_registrar_factory: None,
            direct_closure_enable: None,
        };
        
        // Register module
        register_module(test_module_id.clone(), descriptor);
        
        // Verify it's registered
        assert!(get_module_factory(&test_module_id).is_ok());
        
        // Create service provider
        let sp_handle = create_service_provider(
            &test_module_id,
            CreateServiceProviderOptions {
                service_connector: Arc::new(service_connector_mock()),
            },
        ).unwrap();
        
        // Create module
        let (_module, _protocol_map) = create_module(
            &test_module_id,
            CreateModuleOptions {
                service_connector: Arc::new(service_connector_mock()),
                service_provider: sp_handle.service_provider,
                service_gateways: vec![],
                protocol_servers: vec![],
            },
        ).unwrap();
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

