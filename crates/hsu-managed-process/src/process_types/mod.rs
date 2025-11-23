//! Process type implementations
//!
//! This module provides type-specific wrappers around ProcessControl implementations:
//! - StandardManagedProcess: Full lifecycle control (spawn, monitor, terminate, restart)
//! - IntegratedManagedProcess: Like StandardManaged but with built-in gRPC health checks
//! - UnmanagedProcess: Monitoring only (attach, monitor, detach)
//!
//! Each type provides compile-time guarantees about what operations are supported.

pub mod standard_managed;
pub mod integrated_managed;
pub mod unmanaged;

// Re-export main types
pub use standard_managed::StandardManagedProcess;
pub use integrated_managed::IntegratedManagedProcess;
pub use unmanaged::UnmanagedProcess;

