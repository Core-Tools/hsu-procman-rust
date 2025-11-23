//! # HSU Service Registry
//! 
//! Service registry for dynamic service discovery in the HSU framework.
//! 
//! This crate provides:
//! - In-memory registry storage (thread-safe with DashMap)
//! - HTTP API for publishing and discovering services
//! - Transport abstractions (TCP, Unix Domain Sockets, Named Pipes)
//! - Standalone server executable

pub mod types;
pub mod storage;
pub mod api;
pub mod transport;
pub mod server;

// Re-export commonly used items
pub use types::{ModuleInfo, RemoteModuleAPI};
pub use storage::Registry;
pub use server::RegistryServer;
