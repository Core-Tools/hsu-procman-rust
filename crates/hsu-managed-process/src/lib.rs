//! # HSU Managed Process
//!
//! Process types and management for the HSU framework.
//!
//! This crate provides:
//! - Process control trait (interface) for lifecycle management
//! - Process type definitions (standard, integrated, unmanaged)
//! - Process metadata and options
//!
//! This corresponds to the Go package `pkg/managedprocess`.
//!
//! **Architecture:**
//! ```text
//! ProcessManager (orchestration)
//!       ↓ uses
//! ProcessControl trait (interface)
//!       ↓ implemented by
//! ProcessControlImpl (complex logic - in hsu-process-management)
//!       ↓ wrapped by
//! StandardManaged, IntegratedManaged, Unmanaged
//! ```

pub mod process_control;
pub mod process_types;

// Re-export main types from process_control
pub use process_control::{
    ProcessControl, ProcessControlConfig, ProcessDiagnostics, ProcessError,
    ErrorCategory, RestartTriggerType, RestartContext,
};

// Re-export process types
pub use process_types::{
    StandardManagedProcess,
    IntegratedManagedProcess,
    UnmanagedProcess,
};

use hsu_process_state::{ProcessState, ProcessStateMachine};
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Process management type enumeration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProcessManagementType {
    /// Standard managed process - full lifecycle control
    StandardManaged,
    /// Integrated managed process - with gRPC integration
    IntegratedManaged,
    /// Unmanaged process - monitoring only
    Unmanaged,
}

/// Process metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub tags: Vec<String>,
}

impl Default for ProcessMetadata {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            version: None,
            tags: Vec::new(),
        }
    }
}

/// Process control options.
#[derive(Debug, Clone)]
pub struct ProcessControlOptions {
    pub id: String,
    pub process_type: ProcessManagementType,
    pub metadata: ProcessMetadata,
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub environment: std::collections::HashMap<String, String>,
}

/// Managed process wrapper.
pub struct ManagedProcess {
    pub id: String,
    pub process_type: ProcessManagementType,
    pub metadata: ProcessMetadata,
    pub state_machine: ProcessStateMachine,
    pub pid: Option<u32>,
    pub start_time: Option<DateTime<Utc>>,
    pub restart_count: u32,
    pub last_restart_time: Option<DateTime<Utc>>,
}

impl ManagedProcess {
    pub fn new(id: String, process_type: ProcessManagementType) -> Self {
        Self {
            id: id.clone(),
            process_type,
            metadata: ProcessMetadata::default(),
            state_machine: ProcessStateMachine::new(&id),
            pid: None,
            start_time: None,
            restart_count: 0,
            last_restart_time: None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state_machine.current_state(), ProcessState::Running)
    }

    pub fn has_pid(&self) -> bool {
        self.pid.is_some()
    }

    pub fn current_state(&self) -> ProcessState {
        self.state_machine.current_state()
    }

    pub fn record_start(&mut self, pid: u32) {
        self.pid = Some(pid);
        self.start_time = Some(Utc::now());
    }

    pub fn record_restart(&mut self) {
        self.restart_count += 1;
        self.last_restart_time = Some(Utc::now());
    }

    pub fn clear_runtime_state(&mut self) {
        self.pid = None;
        self.start_time = None;
    }
}

