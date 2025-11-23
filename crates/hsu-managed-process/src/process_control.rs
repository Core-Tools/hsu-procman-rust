//! ProcessControl trait - Interface for process lifecycle management
//!
//! This module defines the trait (interface) that separates high-level process
//! management orchestration from low-level process control implementation.
//!
//! **Design Philosophy:**
//! - Manager orchestrates multiple processes
//! - ProcessControl manages individual process lifecycle
//! - Implementation handles complex details (health, resources, restart)
//!
//! Mirrors Go's `pkg/managedprocess/processcontrol/ProcessControl` interface.

use async_trait::async_trait;
use hsu_common::ProcessResult;
use hsu_process_state::ProcessState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// ProcessControl trait defines the interface for controlling a process lifecycle
///
/// This trait separates the concerns of:
/// - High-level orchestration (ProcessManager)
/// - Low-level process control (implementations)
///
/// Implementations handle:
/// - Process start/stop/restart logic
/// - Health check integration
/// - Resource monitoring
/// - Circuit breaker logic
/// - State machine management
#[async_trait]
pub trait ProcessControl: Send + Sync {
    /// Start the process
    ///
    /// Spawns or attaches to the process, initializes monitoring tasks,
    /// and transitions to running state.
    async fn start(&mut self) -> ProcessResult<()>;

    /// Stop the process gracefully
    ///
    /// Attempts graceful shutdown with configured timeout,
    /// falls back to force kill if necessary.
    async fn stop(&mut self) -> ProcessResult<()>;

    /// Restart the process (stop then start)
    ///
    /// # Arguments
    /// * `force` - If true, bypasses circuit breaker for immediate restart
    ///            If false, uses circuit breaker safety mechanisms (recommended)
    async fn restart(&mut self, force: bool) -> ProcessResult<()>;

    /// Get the current process state
    fn get_state(&self) -> ProcessState;

    /// Get detailed process diagnostics including error information
    fn get_diagnostics(&self) -> ProcessDiagnostics;

    /// Get process PID if running
    fn get_pid(&self) -> Option<u32>;

    /// Check if process is healthy (from health checks)
    fn is_healthy(&self) -> bool;
    
    /// Process pending automatic restart requests (from health checks, resource violations, etc.)
    /// Returns the number of restart requests processed.
    /// 
    /// Default implementation does nothing (for process types that don't support automatic restart).
    async fn process_pending_restarts(&mut self) -> ProcessResult<usize> {
        Ok(0)
    }
}

/// Process diagnostics information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessDiagnostics {
    /// Current process state
    pub state: ProcessState,
    
    /// Last error that occurred (if any)
    pub last_error: Option<ProcessError>,
    
    /// Process ID (if running)
    pub process_id: Option<u32>,
    
    /// When the process started
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    
    /// Executable path
    pub executable_path: String,
    
    /// Whether executable exists on filesystem
    pub executable_exists: bool,
    
    /// Number of consecutive failures
    pub failure_count: u32,
    
    /// Last restart/start attempt time
    pub last_attempt_time: Option<chrono::DateTime<chrono::Utc>>,
    
    /// Health check status
    pub is_healthy: bool,
    
    /// CPU usage percentage (if available)
    pub cpu_usage: Option<f32>,
    
    /// Memory usage in MB (if available)
    pub memory_usage: Option<u64>,
}

/// Categorized process error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessError {
    /// Error category
    pub category: ErrorCategory,
    
    /// Human-readable description
    pub details: String,
    
    /// When the error occurred
    pub timestamp: chrono::DateTime<chrono::Utc>,
    
    /// Whether this error is potentially recoverable
    pub recoverable: bool,
}

/// Error category enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    ExecutableNotFound,
    PermissionDenied,
    ResourceLimit,
    NetworkIssue,
    Timeout,
    ProcessCrash,
    HealthCheckFailure,
    CircuitBreakerTripped,
    Unknown,
}

/// Restart trigger type - what caused the restart
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartTriggerType {
    HealthFailure,
    ResourceViolation,
    Manual,
    ProcessCrash,
}

/// Restart context provides information about restart trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartContext {
    /// What triggered the restart
    pub trigger_type: RestartTriggerType,
    
    /// Severity level
    pub severity: String,
    
    /// Process profile type (batch, web, database, etc.)
    pub process_profile_type: String,
    
    /// Specific violation type (memory, cpu, health, etc.)
    pub violation_type: Option<String>,
    
    /// Human-readable message
    pub message: String,
}

/// Process control configuration options
#[derive(Debug, Clone)]
pub struct ProcessControlConfig {
    /// Process ID for tracking
    pub process_id: String,
    
    /// Can attach to existing process
    pub can_attach: bool,
    
    /// Can terminate process
    pub can_terminate: bool,
    
    /// Can restart process
    pub can_restart: bool,
    
    /// Graceful shutdown timeout
    pub graceful_timeout: Duration,
    
    /// Process profile type for context-aware decisions
    pub process_profile_type: String,
    
    /// Log collection service (optional)
    pub log_collection_service: Option<Arc<hsu_log_collection::LogCollectionService>>,
    
    /// Log collection configuration (optional)
    pub log_config: Option<hsu_log_collection::ProcessLogConfig>,
}

impl Default for ProcessControlConfig {
    fn default() -> Self {
        Self {
            process_id: String::new(),
            can_attach: false,
            can_terminate: true,
            can_restart: true,
            graceful_timeout: Duration::from_secs(10),
            process_profile_type: "standard".to_string(),
            log_collection_service: None,
            log_config: None,
        }
    }
}

impl ProcessDiagnostics {
    /// Create new diagnostics with minimal information
    pub fn new(process_id: String, state: ProcessState) -> Self {
        Self {
            state,
            last_error: None,
            process_id: None,
            start_time: None,
            executable_path: process_id,
            executable_exists: false,
            failure_count: 0,
            last_attempt_time: None,
            is_healthy: true,
            cpu_usage: None,
            memory_usage: None,
        }
    }
}

impl ProcessError {
    /// Create a new process error
    pub fn new(category: ErrorCategory, details: String, recoverable: bool) -> Self {
        Self {
            category,
            details,
            timestamp: chrono::Utc::now(),
            recoverable,
        }
    }
}

