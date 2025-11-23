//! # HSU Log Collection
//!
//! Log capture and aggregation for the HSU framework.
//!
//! This crate provides:
//! - Process output capture (stdout/stderr)
//! - Log aggregation and forwarding
//! - Structured logging enhancement
//! - Multiple output targets
//!
//! This corresponds to the Go package `pkg/logcollection`.

pub mod service;
pub mod types;
pub mod output;

// Re-export main types
pub use service::{LogCollectionService, ProcessLogConfig, SystemLogConfig};
pub use types::{LogEntry, LogLevel, StreamType, LogMetadata, RawLogEntry};
pub use output::{OutputWriter, FileOutputWriter};
