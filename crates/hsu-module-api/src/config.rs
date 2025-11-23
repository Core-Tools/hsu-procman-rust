//! Configuration Types for Module Runtime
//!
//! This module provides configuration structures matching the Golang implementation.
//!
//! ## Comparison with Golang
//!
//! **Golang (`hsu-core/go/pkg/modulemanagement/modulewiring/config.go`):**
//! ```go
//! type Config struct {
//!     Runtime RuntimeConfig
//!     Modules []ModuleConfig
//! }
//!
//! type RuntimeConfig struct {
//!     ServiceRegistry ServiceRegistryConfig
//!     Servers         []ProtocolServerConfig
//! }
//!
//! type ModuleConfig struct {
//!     ID      ModuleID
//!     Enabled bool
//!     Servers []Protocol
//! }
//! ```
//!
//! **Rust (this file):** Nearly identical structure!

use hsu_common::{ModuleID, Protocol};

/// Top-level configuration for the entire runtime.
///
/// This matches Golang's `Config` struct.
#[derive(Clone, Debug)]
pub struct Config {
    /// Runtime-level configuration (servers, registry, etc.)
    pub runtime: RuntimeConfig,
    
    /// Per-module configuration
    pub modules: Vec<ModuleConfig>,
}

/// Runtime-level configuration.
///
/// This matches Golang's `RuntimeConfig` struct.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Service registry configuration
    pub service_registry: ServiceRegistryConfig,
    
    /// Protocol servers to create
    pub servers: Vec<ProtocolServerConfig>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            service_registry: ServiceRegistryConfig::default(),
            servers: Vec::new(),
        }
    }
}

/// Service registry configuration.
///
/// This matches Golang's `ServiceRegistryConfig` struct.
#[derive(Clone, Debug)]
pub struct ServiceRegistryConfig {
    /// Registry URL (e.g., "http://localhost:8080")
    pub url: String,
}

impl Default for ServiceRegistryConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_string(),
        }
    }
}

/// Protocol server configuration.
///
/// This matches Golang's `ProtocolServerConfig` struct.
#[derive(Clone, Debug)]
pub struct ProtocolServerConfig {
    /// Protocol type (Grpc, Http, etc.)
    pub protocol: Protocol,
    
    /// Listen address (e.g., "0.0.0.0:50051")
    pub listen_address: String,
}

/// Per-module configuration.
///
/// This matches Golang's `ModuleConfig` struct.
#[derive(Clone, Debug)]
pub struct ModuleConfig {
    /// Module ID (e.g., "echo", "echo-client")
    pub id: ModuleID,
    
    /// Whether module is enabled
    pub enabled: bool,
    
    /// Which protocol servers this module should register handlers on
    pub servers: Vec<Protocol>,
}

