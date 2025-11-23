//! Bootstrap - Module Wiring and Runtime Initialization
//!
//! This module provides the `run_with_config()` function that orchestrates
//! the entire module runtime lifecycle.
//!
//! ## Architecture
//!
//! ```text
//! run_with_config(config)
//!     ↓
//! 1. Create ServiceConnector
//! 2. For each enabled module:
//!    a. Get ModuleDescriptor from registry
//!    b. Create ServiceProvider
//!    c. Create Module instance
//!    d. Register handlers (if server module)
//!    e. Enable direct closure (if available)
//! 3. Start all modules
//! 4. Wait for Ctrl+C
//! 5. Stop all modules gracefully
//! ```
//!
//! ## Comparison with Golang
//!
//! This is the Rust equivalent of:
//! - `hsu-core/go/pkg/modulemanagement/modulewiring/bootstrap.go`
//! - Function: `RunWithConfig(cfg *Config, logger logging.Logger) error`
//!
//! **Key differences:**
//! - Rust: Async/await (Golang: goroutines)
//! - Rust: Result<()> (Golang: error)
//! - Rust: tracing (Golang: logger parameter)

use std::sync::Arc;
use hsu_common::Result;
use tracing::{info, debug, error};

use crate::{
    Config,
    Module,
    service_connector::ServiceConnectorImpl,
    ServiceRegistryClient,
};

/// Runs the module runtime with the given configuration.
///
/// This is the main entry point for running HSU modules.
///
/// # Arguments
///
/// * `config` - Runtime and module configuration
///
/// # Returns
///
/// * `Ok(())` - Runtime completed successfully
/// * `Err(_)` - Runtime encountered an error
///
/// # Example
///
/// ```rust,no_run
/// use hsu_module_api::{Config, ModuleConfig, run_with_config};
/// use hsu_common::ModuleID;
///
/// #[tokio::main]
/// async fn main() -> hsu_common::Result<()> {
///     // Register modules first
///     echo_server::init_echo_server_module(Default::default())?;
///     echo_client::init_echo_client_module(Default::default())?;
///     
///     // Configure and run
///     let config = Config {
///         runtime: Default::default(),
///         modules: vec![
///             ModuleConfig {
///                 id: ModuleID::from("echo"),
///                 enabled: true,
///                 servers: vec![],
///             },
///             ModuleConfig {
///                 id: ModuleID::from("echo-client"),
///                 enabled: true,
///                 servers: vec![],
///             },
///         ],
///     };
///     
///     run_with_config(config).await
/// }
/// ```
pub async fn run_with_config(config: Config) -> Result<()> {
    info!("🚀 Starting HSU Module Runtime");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    // Step 1: Create service registry client
    debug!("[Bootstrap] Creating service registry client");
    let registry_client = Arc::new(ServiceRegistryClient::new(&config.runtime.service_registry.url));
    info!("✅ Service registry client created: {}", config.runtime.service_registry.url);
    
    // Step 1.5: Create protocol servers from config
    //
    // NOTE: We use hsu_module_proto::new_protocol_server which returns Arc<dyn ProtocolServer>.
    // Protocol servers will be started later, but for now we just create them.
    // Handler registration happens during module creation (modules receive Arc references).
    debug!("[Bootstrap] Creating {} protocol server(s)", config.runtime.servers.len());
    let protocol_servers_arc: Vec<Arc<dyn hsu_module_proto::ProtocolServer>> = {
        let mut servers = Vec::new();
        for server_config in &config.runtime.servers {
            info!("[Bootstrap] Creating protocol server: {:?} on {}", 
                server_config.protocol, server_config.listen_address);
            
            let server = hsu_module_proto::new_protocol_server(
                server_config.protocol,
                server_config.listen_address.clone(),
            )?;
            
            servers.push(server);
            debug!("[Bootstrap]   - Protocol server created");
        }
        servers
    };
    
    if !protocol_servers_arc.is_empty() {
        info!("✅ Created {} protocol server(s)", protocol_servers_arc.len());
        debug!("[Bootstrap] Note: Servers will be started after modules are created");
    }
    
    // Step 2: Create service connector
    debug!("[Bootstrap] Creating service connector");
    let service_connector = Arc::new(ServiceConnectorImpl::new(registry_client.clone()));
    info!("✅ Service connector created");
    
    // Step 3: Create service providers FIRST (matches Golang!)
    //
    // IMPORTANT: We create service providers in a SEPARATE step before creating modules.
    // This matches Golang's bootstrap.go flow and is architecturally correct.
    //
    // Why separate steps?
    // - Service providers are created independently
    // - Modules are then created using those service providers
    // - Allows for future features like service gateway maps, protocol server registration
    debug!("[Bootstrap] Creating service providers for {} modules", config.modules.len());
    use std::collections::HashMap;
    use crate::create_service_provider;
    
    let mut service_provider_map: HashMap<hsu_common::ModuleID, crate::ServiceProviderHandle> = HashMap::new();
    
    for module_config in &config.modules {
        if !module_config.enabled {
            debug!("[Bootstrap] Skipping disabled module: {}", module_config.id);
            continue;
        }
        
        info!("[Bootstrap] Creating service provider for module: {}", module_config.id);
        
        // Create service provider options with protocol servers
        let options = crate::module_descriptor::CreateServiceProviderOptions {
            service_connector: service_connector.clone(),
            protocol_servers: protocol_servers_arc.clone(),  // Pass servers!
        };
        
        // Create service provider from registry
        let service_provider_handle = create_service_provider(&module_config.id, options)?;
        debug!("[Bootstrap]   - Service provider created");
        
        service_provider_map.insert(module_config.id.clone(), service_provider_handle);
        info!("✅ Service provider for '{}' created", module_config.id);
    }
    
    info!("✅ All {} service providers created", service_provider_map.len());
    
    // Step 3.5: Build service gateway maps (for direct closure)
    //
    // This builds a reverse map: target_module_id → Vec<service_gateways>
    // allowing multiple clients to register their gateways for the same server module.
    //
    // Example: Both "echo-client-1" and "echo-client-2" provide gateways for "echo" module.
    debug!("[Bootstrap] Building service gateway maps");
    use std::any::Any;
    
    type ServiceGatewaysMap = HashMap<hsu_common::ModuleID, Vec<Box<dyn Any + Send + Sync>>>;
    let mut service_gateways_map: ServiceGatewaysMap = HashMap::new();
    
    for (module_id, service_provider_handle) in service_provider_map.iter_mut() {
        // For each service gateway this module provides
        // We take ownership (move) of the gateways from the map
        for (target_module_id, service_gateway) in service_provider_handle.service_gateways_map.drain() {
            debug!("[Bootstrap]   Module '{}' provides gateway for '{}'", module_id, target_module_id);
            
            // Get or create the vector for this target module
            let gateways_vec = service_gateways_map
                .entry(target_module_id)
                .or_insert_with(Vec::new);
            
            // Move the gateway (no clone needed!)
            gateways_vec.push(service_gateway);
        }
    }
    
    debug!("[Bootstrap] Service gateway map built with {} target modules", service_gateways_map.len());
    for (target_module_id, gateways) in &service_gateways_map {
        debug!("[Bootstrap]   Target '{}' has {} gateway(s)", target_module_id, gateways.len());
    }
    
    // Step 4: Create modules using service providers
    debug!("[Bootstrap] Creating {} modules", config.modules.len());
    
    // Store both modules and their protocol-to-services maps
    use crate::module_descriptor::ProtocolToServicesMap;
    let mut modules: Vec<Box<dyn Module>> = Vec::new();
    let mut module_protocol_maps: HashMap<hsu_common::ModuleID, ProtocolToServicesMap> = HashMap::new();
    
    for module_config in &config.modules {
        if !module_config.enabled {
            continue;
        }
        
        info!("[Bootstrap] Creating module: {}", module_config.id);
        
        // Move (not borrow) service provider from map
        // Each module creation consumes the service provider
        let service_provider_handle = service_provider_map.remove(&module_config.id)
            .ok_or_else(|| hsu_common::Error::Internal(
                format!("Service provider not found for module '{}'", module_config.id)
            ))?;
        debug!("[Bootstrap]   - Moved service provider from map");
        
        // Move service gateways for this module (if any clients provide them)
        let service_gateways = service_gateways_map
            .remove(&module_config.id)
            .unwrap_or_else(Vec::new);
        debug!("[Bootstrap]   - Got {} service gateway(s) for this module", service_gateways.len());
        
        // Create module from registry with gateway support
        use crate::module_descriptor::CreateModuleOptions;
        let options = CreateModuleOptions {
            service_connector: service_connector.clone(),
            service_provider: service_provider_handle.service_provider,  // Move the Box
            service_gateways,  // Pass gateways for direct closure!
            protocol_servers: protocol_servers_arc.clone(),  // ✅ Protocol servers passed!
        };
        
        let (module, protocol_map) = crate::registry::create_module_with_options(&module_config.id, options)?;
        debug!("[Bootstrap]   - Module instance created");
        
        // ✅ Handler registration and direct closure are now handled inside create_module_with_options!
        
        // Store the protocol map for API publishing
        module_protocol_maps.insert(module_config.id.clone(), protocol_map);
        
        modules.push(module);
        info!("✅ Module '{}' created successfully", module_config.id);
    }
    
    info!("✅ All {} modules created", modules.len());
    
    // Step 5: Start all modules
    info!("\n🔄 Starting modules...");
    for module in &mut modules {
        let module_id = module.id().clone();
        debug!("[Bootstrap] Starting module: {}", module_id);
        
        module.start().await?;
        
        info!("✅ Module '{}' started", module_id);
    }
    
    info!("\n🎉 All modules started successfully!");
    
    // Step 5.5: Start protocol servers
    //
    // NOW IMPLEMENTED! Protocol servers use interior mutability (RwLock) so they can be
    // started with &self (works with Arc<dyn ProtocolServer>).
    if !protocol_servers_arc.is_empty() {
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("🌐 Starting {} protocol server(s)", protocol_servers_arc.len());
        
        for (idx, server) in protocol_servers_arc.iter().enumerate() {
            debug!("[Bootstrap] Starting protocol server {} ({:?})...", idx + 1, server.protocol());
            
            server.start().await?;
            
            info!("✅ Protocol server {} started on port {}", idx + 1, server.port());
        }
        
        info!("✅ All protocol servers started successfully");
        
        // Step 5.6: Publish APIs to service registry
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("📤 Publishing module APIs to service registry...");
        
        use crate::registry_client::RemoteAPI;
        
        for module_config in &config.modules {
            if !module_config.enabled {
                continue;
            }
            
            // Get the protocol-to-services map for this module
            let protocol_map = module_protocol_maps.get(&module_config.id);
            
            if protocol_map.is_none() {
                debug!("[Bootstrap] Module '{}' has no protocol map, skipping API publishing", module_config.id);
                continue;
            }
            
            let protocol_map = protocol_map.unwrap();
            
            // Build list of APIs from protocol servers using ACTUAL service IDs
            // This matches Golang's approach: use protocolToServicesMap to get real service IDs
            let mut apis = Vec::new();
            for server in &protocol_servers_arc {
                let protocol = server.protocol();
                
                // Get service IDs for this protocol from the module's protocol map
                if let Some(service_ids) = protocol_map.get(&protocol) {
                    // Create one RemoteAPI per service ID (matching Golang!)
                    for service_id in service_ids {
                        let api = RemoteAPI {
                            service_id: service_id.to_string(),  // ✅ ACTUAL service ID!
                            protocol,
                            address: Some(server.address()),
                            metadata: None,
                        };
                        apis.push(api);
                        debug!("[Bootstrap]   - API: protocol={:?}, service_id={}, address={}", 
                            protocol, service_id, server.address());
                    }
                } else {
                    debug!("[Bootstrap]   - Module '{}' has no services for protocol {:?}", 
                        module_config.id, protocol);
                }
            }
            
            if !apis.is_empty() {
                debug!("[Bootstrap] Publishing {} API(s) for module '{}'", apis.len(), module_config.id);
                
                // Get process ID
                let process_id = std::process::id();
                
                let num_apis = apis.len();
                
                // Publish to registry
                registry_client
                    .publish(&module_config.id, process_id, apis)
                    .await?;
                
                info!("✅ Published {} API(s) for module '{}'", num_apis, module_config.id);
            } else {
                debug!("[Bootstrap] Module '{}' has no APIs to publish", module_config.id);
            }
        }
        
        info!("✅ All module APIs published to service registry");
    }
    
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    info!("📝 Runtime is operational");
    info!("Press Ctrl+C to stop...\n");
    
    // Step 6: Wait for shutdown signal (Ctrl+C)
    tokio::signal::ctrl_c().await
        .map_err(|e| hsu_common::Error::Internal(format!("Signal error: {}", e)))?;
    
    info!("\n📋 Shutting down gracefully...");
    
    // Step 7: Stop all modules in reverse order
    info!("🔄 Stopping modules...");
    for module in modules.iter_mut().rev() {
        let module_id = module.id().clone();
        debug!("[Bootstrap] Stopping module: {}", module_id);
        
        if let Err(e) = module.stop().await {
            error!("❌ Error stopping module '{}': {}", module_id, e);
        } else {
            info!("✅ Module '{}' stopped", module_id);
        }
    }
    
    // Step 7.5: Stop protocol servers
    if !protocol_servers_arc.is_empty() {
        info!("\n🔄 Stopping {} protocol server(s)...", protocol_servers_arc.len());
        
        for (idx, server) in protocol_servers_arc.iter().enumerate() {
            debug!("[Bootstrap] Stopping protocol server {} ({:?})...", idx + 1, server.protocol());
            
            if let Err(e) = server.stop().await {
                error!("❌ Error stopping protocol server {}: {}", idx + 1, e);
            } else {
                info!("✅ Protocol server {} stopped", idx + 1);
            }
        }
        
        info!("✅ All protocol servers stopped");
    }
    
    info!("\n✅ Clean shutdown complete!");
    info!("👋 Goodbye!\n");
    
    Ok(())
}

