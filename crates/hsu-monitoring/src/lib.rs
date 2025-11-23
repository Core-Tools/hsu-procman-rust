//! # HSU Monitoring
//!
//! Health checking and monitoring for the HSU framework.
//!
//! This crate provides:
//! - HTTP health checks
//! - gRPC health checks
//! - Resource monitoring
//! - Health status tracking
//!
//! This corresponds to the Go package `pkg/monitoring`.

pub mod http;
// TODO: Fix gRPC proto compilation issues
// pub mod grpc;
pub mod resource_monitoring;
pub mod health_monitor;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Health check error types.
#[derive(Error, Debug)]
pub enum HealthCheckError {
    #[error("Health check timeout: {id}")]
    Timeout { id: String },

    #[error("Health check connection failed: {id} - {reason}")]
    ConnectionFailed { id: String, reason: String },

    #[error("Health check invalid response: {id} - {response}")]
    InvalidResponse { id: String, response: String },

    #[error("Health check endpoint not configured: {id}")]
    NotConfigured { id: String },

    #[error("Health check service unavailable: {id}")]
    ServiceUnavailable { id: String },
}

/// Result type for health check operations.
pub type HealthCheckResult<T> = Result<T, HealthCheckError>;

/// Health check result data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckData {
    pub is_healthy: bool,
    pub checked_at: DateTime<Utc>,
    pub response_time_ms: Option<u64>,
    pub error_message: Option<String>,
}

/// Health status tracker for a process.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub is_healthy: bool,
    pub last_check: Option<DateTime<Utc>>,
    pub last_success: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub failure_reason: Option<String>,
}

impl HealthStatus {
    pub fn new() -> Self {
        Self {
            is_healthy: true,
            last_check: None,
            last_success: None,
            consecutive_failures: 0,
            consecutive_successes: 0,
            failure_reason: None,
        }
    }

    pub fn record_success(&mut self) {
        self.is_healthy = true;
        self.last_check = Some(Utc::now());
        self.last_success = Some(Utc::now());
        self.consecutive_successes += 1;
        self.consecutive_failures = 0;
        self.failure_reason = None;
    }

    pub fn record_failure(&mut self, reason: String, failure_threshold: u32) {
        self.last_check = Some(Utc::now());
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;
        self.failure_reason = Some(reason);

        if self.consecutive_failures >= failure_threshold {
            self.is_healthy = false;
        }
    }
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export main types
pub use http::*;
// pub use grpc::*;  // TODO: Enable when gRPC is fixed
pub use resource_monitoring::*;
pub use health_monitor::*;

