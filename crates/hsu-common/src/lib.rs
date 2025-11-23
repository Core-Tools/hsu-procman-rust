//! # HSU Common
//! 
//! Common types, traits, and utilities shared across the HSU framework.
//! 
//! This crate provides the foundational abstractions that all other HSU
//! crates build upon, including error types, logging interfaces, and
//! core domain types.

pub mod errors;
pub mod types;

// Re-export commonly used items
pub use errors::{Error, Result, ProcessError, ProcessResult};
pub use types::{ModuleID, ServiceID, Protocol};

