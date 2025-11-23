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
pub mod manager;
pub mod lifecycle;
pub mod attachment;
pub mod api;
pub mod process_control_impl;

#[cfg(test)]
mod manager_tests;

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
pub use manager::{ProcessManager, ProcessManagerState, ProcessInfo, ProcessDiagnostics};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

