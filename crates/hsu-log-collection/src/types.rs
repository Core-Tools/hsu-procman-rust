//! Core types for log collection

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Log level for structured logging
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

/// Stream type (stdout or stderr)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    Stdout,
    Stderr,
}

impl std::fmt::Display for StreamType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamType::Stdout => write!(f, "stdout"),
            StreamType::Stderr => write!(f, "stderr"),
        }
    }
}

/// Metadata for a log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogMetadata {
    pub timestamp: DateTime<Utc>,
    pub process_id: String,
    pub stream: StreamType,
    pub line_num: i64,
}

/// Raw log entry from a process (unprocessed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawLogEntry {
    pub process_id: String,
    pub stream: StreamType,
    pub line: String,
    pub timestamp: DateTime<Utc>,
}

/// Processed log entry ready for output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: Option<String>,
    pub message: String,
    pub process_id: String,
    pub stream: StreamType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<HashMap<String, serde_json::Value>>,
    pub raw_line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enhanced: Option<HashMap<String, serde_json::Value>>,
}

/// Status information for a specific process's log collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLogStatus {
    pub process_id: String,
    pub active: bool,
    pub lines_processed: i64,
    pub bytes_processed: i64,
    pub last_activity: Option<DateTime<Utc>>,
    pub errors: Vec<String>,
}

/// Overall log collection system status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemLogStatus {
    pub active: bool,
    pub processes_active: usize,
    pub total_processes: usize,
    pub total_lines: i64,
    pub total_bytes: i64,
    pub start_time: DateTime<Utc>,
    pub last_activity: Option<DateTime<Utc>>,
    pub output_targets: Vec<String>,
}


