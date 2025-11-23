//! Module Trait - New Architecture
//!
//! This is the NEW module trait for the refactored architecture.
//! It only includes the essential methods needed for module lifecycle.
//!
//! # Design Philosophy
//!
//! Unlike the old `Module` trait from `hsu_module_management`, this new trait:
//! - Does NOT have `set_service_gateway_factory()` - gateways come from service provider
//! - Does NOT have `service_handlers_map()` - handlers managed separately via registrar
//! - Only has lifecycle methods: `start()` and `stop()`
//! - Only has identification: `id()`
//!
//! # Comparison with Old Trait
//!
//! **Old trait** (`hsu_module_management::Module`):
//! ```rust,ignore
//! trait Module {
//!     fn id(&self) -> &ModuleID;
//!     fn set_service_gateway_factory(&mut self, factory: Arc<dyn ServiceGatewayFactory>);
//!     async fn start(&mut self) -> Result<()>;
//!     async fn stop(&mut self) -> Result<()>;
//!     fn service_handlers_map(&self) -> Option<ServiceHandlersMap>;
//! }
//! ```
//!
//! **New trait** (this file):
//! ```rust
//! trait Module {
//!     fn id(&self) -> &ModuleID;
//!     async fn start(&mut self) -> Result<()>;
//!     async fn stop(&mut self) -> Result<()>;
//! }
//! ```
//!
//! Much simpler! The service provider pattern handles everything else.

use async_trait::async_trait;
use hsu_common::{ModuleID, Result};

/// Module trait for the new architecture.
///
/// This is a minimal trait that only includes essential lifecycle methods.
///
/// # Lifecycle
///
/// 1. Module is created by factory function
/// 2. Framework calls `start()` to initialize
/// 3. Module runs (handles requests, makes calls, etc.)
/// 4. Framework calls `stop()` to shut down
///
/// # Service Access
///
/// Modules access services through their service provider, which is injected
/// during creation (not through `set_service_gateway_factory` like the old design).
///
/// # Handler Registration
///
/// Server modules provide handlers through the `HandlersRegistrar`, which is
/// created by the service provider (not through `service_handlers_map`).
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns the module's unique identifier.
    fn id(&self) -> &ModuleID;

    /// Starts the module.
    ///
    /// This is called by the framework after the module is created.
    /// Modules should initialize resources and begin operation here.
    async fn start(&mut self) -> Result<()>;

    /// Stops the module.
    ///
    /// This is called by the framework during shutdown.
    /// Modules should clean up resources and stop operations here.
    async fn stop(&mut self) -> Result<()>;
}

