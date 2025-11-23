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
use hsu_common::{ModuleID, Result, Error};
use crate::Module;
use lazy_static::lazy_static;
use tracing::{debug, trace, error};

use crate::module_descriptor::{
    ModuleDescriptor, CreateModuleOptions, CreateServiceProviderOptions,
    ServiceProviderHandle, ProtocolToServicesMap, DirectClosureEnablerOptions, HandlersRegistrarOptions,
};

/// Type-erased module factory function.
///
/// # Rust Learning Note
///
/// This is stored in the global registry. It erases the generic types (SP, SG, SH)
/// so we can store different module types in the same HashMap.
///
/// We use `Box<dyn Fn(...)>` instead of `fn(...)` because we need to store closures
/// that capture variables (the descriptor).
pub type ModuleFactoryFn = Box<dyn Fn(CreateModuleOptions) -> Result<(Box<dyn Module>, ProtocolToServicesMap)> + Send + Sync>;

/// Type-erased service provider factory function.
pub type ServiceProviderFactoryFn = Box<dyn Fn(CreateServiceProviderOptions) -> ServiceProviderHandle + Send + Sync>;

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
    
    // Clone module_id for each closure
    let module_id_sp = module_id.clone();
    let module_id_module = module_id.clone();
    
    // Create type-erased service provider factory
    let sp_factory = move |options: CreateServiceProviderOptions| -> ServiceProviderHandle {
        trace!("[Registry] Creating service provider for: {}", module_id_sp);
        (descriptor.service_provider_factory)(options.service_connector)
    };
    
    // Create type-erased module factory with type casting
    let module_factory = move |options: CreateModuleOptions| -> Result<(Box<dyn Module>, ProtocolToServicesMap)> {
        trace!("[Registry] Creating module: {}", module_id_module);
        
        // ✅ Type cast happens HERE (once, centralized!)
        let typed_sp = options.service_provider
            .downcast_ref::<SP>()
            .ok_or_else(|| {
                error!(
                    "[Registry] Type mismatch for module '{}': expected {}, got different type",
                    module_id_module,
                    std::any::type_name::<SP>()
                );
                Error::Validation {
                    message: format!(
                        "Type mismatch for module '{}': expected provider of type {}, got different type",
                        module_id_module,
                        std::any::type_name::<SP>()
                    ),
                }
            })?;
        
        // ✅ Now type-safe!
        trace!("[Registry] Service provider type cast successful");
        let (module, handlers) = (descriptor.module_factory)(typed_sp.clone());
        
        // Register handlers if needed (server modules)
        let mut protocol_map = HashMap::new();
        if let Some(registrar_fn) = descriptor.handlers_registrar {
            trace!("[Registry] Creating handlers registrar for: {}", module_id_module);
            let opts = HandlersRegistrarOptions {
                protocol_servers: options.protocol_servers.clone(),
                service_handlers: handlers.clone(),
            };
            protocol_map = registrar_fn(opts)?;
            debug!("[Registry] Handlers registered for {}: {:?}", module_id_module, protocol_map.keys());
        }
        
        // Enable direct closure if needed
        if let Some(enabler_fn) = descriptor.direct_closure_enabler {
            trace!("[Registry] Enabling direct closure for: {}", module_id_module);
            for sg in options.service_gateways {
                let typed_sg = sg.downcast_ref::<SG>()
                    .ok_or_else(|| {
                        error!(
                            "[Registry] Type mismatch for service gateways in module '{}'",
                            module_id_module
                        );
                        Error::Validation {
                            message: format!(
                                "Type mismatch for module '{}': expected service gateways of type {}",
                                module_id_module,
                                std::any::type_name::<SG>()
                            ),
                        }
                    })?;
                
                let opts = DirectClosureEnablerOptions {
                    service_connector: Arc::clone(&options.service_connector),
                    service_gateways: typed_sg.clone(),
                    service_handlers: handlers.clone(),
                };
                enabler_fn(opts);
                debug!("[Registry] Direct closure enabled for: {}", module_id_module);
            }
        }
        
        debug!("[Registry] ✅ Module created successfully: {}", module_id_module);
        Ok((module, protocol_map))
    };
    
    // Store in global registry
    MODULE_REGISTRY.write().unwrap().insert(module_id.clone(), Box::new(module_factory));
    SERVICE_PROVIDER_REGISTRY.write().unwrap().insert(module_id.clone(), Box::new(sp_factory));
    
    debug!("[Registry] ✅ Module registered: {}", module_id);
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
    let registry = MODULE_REGISTRY.read().unwrap();
    let factory = registry.get(module_id)
        .ok_or_else(|| Error::Validation {
            message: format!("Unknown module ID: {}", module_id),
        })?;
    factory(options)
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
    let registry = SERVICE_PROVIDER_REGISTRY.read().unwrap();
    let factory = registry.get(module_id)
        .ok_or_else(|| Error::Validation {
            message: format!("Unknown service provider factory: {}", module_id),
        })?;
    Ok(factory(options))
}

/// Lists all registered module IDs.
///
/// Useful for debugging and diagnostics.
pub fn list_registered_modules() -> Vec<ModuleID> {
    let registry = MODULE_REGISTRY.read().unwrap();
    registry.keys().cloned().collect()
}

/// Gets a module descriptor from the registry (internal helper).
///
/// This is used internally by `create_module_from_registry`.
fn get_module_factory_fn(module_id: &ModuleID) -> Result<&'static ModuleFactoryFn> {
    let registry = MODULE_REGISTRY.read()
        .map_err(|e| Error::Internal(format!("Registry lock error: {}", e)))?;
    
    // SAFETY: This is safe because MODULE_REGISTRY is a static and lives for the entire program
    // The reference is valid for 'static because the registry never removes entries
    unsafe {
        let factory_ref = registry.get(module_id)
            .ok_or_else(|| Error::NotFound { 
                resource: format!("Module '{}'", module_id) 
            })?;
        Ok(std::mem::transmute::<&ModuleFactoryFn, &'static ModuleFactoryFn>(factory_ref))
    }
}

/// Gets the module descriptor for a given module ID.
///
/// This function looks up a module in the global registry by its ID.
/// It's a wrapper that checks if the module exists.
///
/// # Arguments
///
/// * `module_id` - The ID of the module to look up
///
/// # Returns
///
/// * `Ok(())` - Module exists in registry
/// * `Err(_)` - Module not found
pub fn get_module_descriptor(module_id: &ModuleID) -> Result<()> {
    get_module_factory_fn(module_id)?;
    Ok(())
}

/// Creates a module from the registry using its module ID (simple version).
///
/// This is a convenience function that:
/// 1. Looks up module factory in registry
/// 2. Creates options with empty gateways/servers
/// 3. Calls module factory
///
/// # Arguments
///
/// * `module_id` - The ID of the module to create
/// * `service_connector` - The service connector for creating gateways
///
/// # Returns
///
/// * `Ok((module, handlers))` - Successfully created module
/// * `Err(_)` - Creation failed
///
/// # Example
///
/// ```rust,ignore
/// let (module, handlers) = create_module_from_descriptor(&ModuleID::from("echo"), service_connector)?;
/// ```
pub fn create_module_from_descriptor(
    module_id: &ModuleID,
    service_connector: Arc<dyn crate::ServiceConnector>,
) -> Result<(Box<dyn Module>, ProtocolToServicesMap)> {
    debug!("[Registry] Creating module '{}' from registry (simple)", module_id);
    
    // Create default options
    let options = CreateModuleOptions {
        service_connector,
        service_provider: Box::new(()), // Placeholder, will be created by factory
        service_gateways: Vec::new(),
        protocol_servers: Vec::new(),
    };
    
    create_module_with_options(module_id, options)
}

/// Creates a module from the registry with full options (advanced version).
///
/// This function allows passing service gateways and protocol servers for:
/// - Direct closure enabling (requires service_gateways)
/// - Handler registration (requires protocol_servers)
///
/// # Arguments
///
/// * `module_id` - The ID of the module to create
/// * `options` - Complete module creation options
///
/// # Returns
///
/// * `Ok((module, protocol_map))` - Successfully created module with protocol map
/// * `Err(_)` - Creation failed
///
/// # Example
///
/// ```rust,ignore
/// let options = CreateModuleOptions {
///     service_connector,
///     service_provider,
///     service_gateways: vec![...],  // For direct closure
///     protocol_servers: vec![...],  // For handler registration
/// };
/// let (module, protocol_map) = create_module_with_options(&ModuleID::from("echo"), options)?;
/// ```
pub fn create_module_with_options(
    module_id: &ModuleID,
    options: CreateModuleOptions,
) -> Result<(Box<dyn Module>, ProtocolToServicesMap)> {
    debug!("[Registry] Creating module '{}' with full options", module_id);
    debug!("[Registry]   - {} service gateway(s)", options.service_gateways.len());
    debug!("[Registry]   - {} protocol server(s)", options.protocol_servers.len());
    
    // Get the factory function from registry
    let factory = get_module_factory_fn(module_id)?;
    
    // Call the factory (which handles handler registration and direct closure internally)
    let (module, protocol_map) = factory(options)?;
    
    debug!("[Registry] Module '{}' created with {} protocol(s)", module_id, protocol_map.len());
    
    Ok((module, protocol_map))
}

// Tests removed - the registration pattern is validated through integration tests
// in bootstrap.rs and real-world usage in the example projects

