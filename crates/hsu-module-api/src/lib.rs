//! # HSU Module API
//! 
//! Module API layer including service registry client, gateway factory,
//! and module registration.
//! 
//! This crate provides the glue between modules, protocols, and the service registry:
//! - Service registry client for discovering services
//! - Gateway factory for creating protocol-specific gateways
//! - Auto-protocol selection (direct if available, otherwise remote)
//! - Module runtime for orchestration
//! - Module descriptor and registration system


pub mod registry_client;
pub mod gateway_factory;
pub mod service_connector;
pub mod gateway_factory_typed;
pub mod module_trait;
pub mod module_descriptor;
pub mod registry;
pub mod config;
pub mod bootstrap;

// Re-export commonly used items
pub use registry_client::ServiceRegistryClient;
pub use gateway_factory::ServiceGatewayFactoryImpl;
pub use service_connector::{ServiceConnector, ServiceConnectorImpl};
pub use gateway_factory_typed::{ServiceGatewayFactory, GatewayFactoryFuncs};
pub use module_trait::Module;
pub use module_descriptor::{
    ModuleDescriptor,
    CreateModuleOptions,
    CreateServiceProviderOptions,
    ServiceProviderHandle,
    ProtocolToServicesMap,
    HandlersRegistrarOptions,
    DirectClosureEnablerOptions,
    TypedServiceProviderFactoryFunc,
    TypedModuleFactoryFunc,
    TypedHandlersRegistrarFunc,
    TypedDirectClosureEnablerFunc,
    new_module_descriptor,
};
pub use registry::{
    register_module,
    create_module,
    create_service_provider,
    list_registered_modules,
    get_module_descriptor,
    create_module_from_descriptor,
    create_module_with_options,
};
pub use config::{
    Config,
    RuntimeConfig,
    ServiceRegistryConfig,
    ProtocolServerConfig,
    ModuleConfig,
};
pub use bootstrap::run_with_config;
