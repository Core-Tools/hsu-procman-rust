//! # HSU Process Management
//!
//! Main process management orchestration for the HSU framework.
//!
//! This crate provides:
//! - ProcessManager - main orchestrator
//! - Configuration management  
//! - Process lifecycle management
//! - Management APIs (HTTP/gRPC)
//!
//! This corresponds to the Go package `pkg/processmanagement`.

pub mod config;
pub mod lifecycle;
pub mod attachment;
pub mod api;
pub mod process_control_impl;

#[cfg(test)]
mod manager_tests;

// -----------------------------------------------------------------------------
// ProcessManager implementation selection (procman-v1 vs procman-v2)
// -----------------------------------------------------------------------------

#[cfg(all(feature = "procman-v1", feature = "procman-v2"))]
compile_error!(
    "Exactly one ProcessManager implementation must be enabled: enable either feature \
     `procman-v1` or `procman-v2` (not both)."
);

#[cfg(not(any(feature = "procman-v1", feature = "procman-v2")))]
compile_error!(
    "No ProcessManager implementation selected: enable feature `procman-v2` (default) \
     or explicitly enable `procman-v1`."
);

#[cfg(all(feature = "procman-v1", not(feature = "procman-v2")))]
#[path = "manager.rs"]
pub mod manager;

#[cfg(all(feature = "procman-v2", not(feature = "procman-v1")))]
#[path = "manager/mod.rs"]
pub mod manager;

// Re-export main types
pub use config::{
    ProcessManagerConfig, ProcessConfig, ProcessManagementType,
    ProcessManagerOptions, ProcessManagementConfig,
    StandardManagedProcessConfig, IntegratedManagedProcessConfig, UnmanagedProcessConfig,
    ManagedProcessControlConfig, ExecutionConfig,
    HealthCheckConfig, HealthCheckRunOptions,
    ResourceLimitsConfig, MemoryLimits, CpuLimits, ProcessLimits,
    ContextAwareRestartConfig, RestartConfig,
    RestartPolicyConfig, RestartStrategy,
    LogCollectionConfig,
};

#[cfg(any(
    all(feature = "procman-v1", not(feature = "procman-v2")),
    all(feature = "procman-v2", not(feature = "procman-v1"))
))]
pub use manager::{ProcessDiagnostics, ProcessInfo, ProcessManager, ProcessManagerState};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

