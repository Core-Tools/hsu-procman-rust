//! # HSU Module Management
//! 
//! This crate provides the core module management functionality,
//! including module traits, lifecycle management, and the runtime
//! orchestration layer.

pub mod module_types;
pub mod lifecycle;
pub mod config_types;

// Re-export commonly used items
pub use module_types::{
    Module,
    ServiceHandler,
    ServiceGateway,
    ServiceHandlersMap,
    ServiceGatewayFactory,
    ProtocolGatewayFactory,  // New: User-provided factory trait
};
// Re-export from hsu-common
pub use hsu_common::{Protocol, ServiceID};
pub use lifecycle::Lifecycle;
pub use config_types::{
    ServerOptions,
    HandlersConfig,
    GatewayConfig,
    GatewayConfigMap,
};

