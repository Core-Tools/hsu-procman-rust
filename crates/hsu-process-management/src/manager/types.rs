//! Shared data types for the process manager module.
//!
//! This module contains:
//! - Public types exposed to external callers (ProcessManagerState, ProcessInfo, ProcessDiagnostics)
//! - Crate-internal types used for orchestration (ManagedProcessInstance, OpRequest)

use crate::config::ProcessConfig;
use hsu_managed_process::ProcessControl;
use hsu_monitoring::HealthStatus;
use hsu_process_state::ProcessState;
use hsu_resource_limits::ResourceUsage;
use std::collections::VecDeque;
use tokio::sync::oneshot;
use tokio::time::Duration;

// ============================================================================
// Public Types - Exposed to external callers
// ============================================================================

/// Process manager overall state
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessManagerState {
    Initializing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Error(String),
}

/// Process information structure for external queries
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub id: String,
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    pub restart_count: u32,
    pub cpu_usage: Option<f32>,
    pub memory_usage: Option<u64>,
    pub uptime: Option<Duration>,
    pub is_healthy: bool,
    pub last_health_check: Option<chrono::DateTime<chrono::Utc>>,
    pub consecutive_health_failures: u32,
}

/// Process diagnostics information
#[derive(Debug, Clone)]
pub struct ProcessDiagnostics {
    pub process_info: ProcessInfo,
    pub health_status: Option<HealthStatus>,
    pub resource_usage: ResourceUsage,
    pub last_error: Option<String>,
    pub logs_preview: Vec<String>,
}

// Re-export for compatibility
pub use ProcessManagerState as State;

// ============================================================================
// Crate-Internal Types - Used by actor and facade
// ============================================================================

/// Result type for internal operations
pub(super) type Result<T> = std::result::Result<T, hsu_common::ProcessError>;

/// Kind of operation being performed on a process
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OpKind {
    /// Start the process
    Start,
    /// Stop the process
    Stop,
    /// Restart the process (with force flag)
    Restart { force: bool },
    /// Heartbeat poll for pending restarts
    HeartbeatPoll,
    /// Query process diagnostics (potentially heavy)
    QueryDiagnostics,
    /// Test-only sleep to simulate long-running work
    #[cfg(test)]
    TestSleep(Duration),
}

impl OpKind {
    /// Get a human-readable name for the operation
    pub fn name(&self) -> &'static str {
        match self {
            OpKind::Start => "start",
            OpKind::Stop => "stop",
            OpKind::Restart { .. } => "restart",
            OpKind::HeartbeatPoll => "heartbeat_poll",
            OpKind::QueryDiagnostics => "query_diagnostics",
            #[cfg(test)]
            OpKind::TestSleep(_) => "test_sleep",
        }
    }
}

/// Responder for an in-flight operation
pub(crate) enum OpResponder {
    /// Standard unit result responder
    Unit(oneshot::Sender<Result<()>>),
    /// Process info responder (used by QueryDiagnostics)
    ProcessInfo(oneshot::Sender<Result<ProcessInfo>>),
}

/// A queued operation request waiting to be executed
pub(crate) struct OpRequest {
    /// The operation kind
    pub kind: OpKind,
    /// Optional responder for the caller
    pub resp: Option<OpResponder>,
    /// Batch ID if this is part of a coordinated operation
    pub batch_id: Option<u64>,
}

/// Currently in-flight operation for a process
pub(crate) struct InFlightOp {
    /// The operation kind
    pub kind: OpKind,
    /// Optional responder for the caller
    pub resp: Option<OpResponder>,
    /// Batch ID if this is part of a coordinated operation
    pub batch_id: Option<u64>,
}

/// Individual managed process instance with runtime state
///
/// This is a lightweight wrapper around ProcessControl trait,
/// focusing on orchestration rather than implementation details.
///
/// The `control` field is `Option` because it gets temporarily taken
/// when an operation is in progress (the OpRunner holds it).
pub(super) struct ManagedProcessInstance {
    /// Process configuration (retained for future features like config-based reattachment)
    #[allow(dead_code)]
    pub config: ProcessConfig,

    /// Process control implementation (None when operation is in-flight)
    pub control: Option<Box<dyn ProcessControl>>,

    /// Current in-flight operation (None when idle)
    pub in_flight: Option<InFlightOp>,

    /// Queue of pending operations waiting to be executed
    pub pending_ops: VecDeque<OpRequest>,

    /// Manager-level restart tracking
    pub restart_count: u32,
    pub last_restart_time: Option<chrono::DateTime<chrono::Utc>>,

    /// Last known PID (for diagnostics when control is in-flight)
    pub last_known_pid: Option<u32>,

    /// Last known state (for diagnostics when control is in-flight)
    pub last_known_state: ProcessState,
}

impl ManagedProcessInstance {
    /// Check if this instance is currently busy with an operation
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Get the current operation name if busy
    pub fn busy_op_name(&self) -> Option<&'static str> {
        self.in_flight.as_ref().map(|op| op.kind.name())
    }
}

// ============================================================================
// Batch Tracking Types
// ============================================================================

/// Kind of batch operation
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum BatchKind {
    StartAll,
    Shutdown,
}

/// Pending batch operation tracking
pub(super) struct PendingBatch {
    /// What kind of batch this is
    pub kind: BatchKind,
    /// Number of operations still pending
    pub pending_count: usize,
    /// IDs of processes that failed
    pub failed_process_ids: Vec<String>,
    /// Response channel to notify caller when batch completes (None for internal batches)
    pub resp: Option<oneshot::Sender<Result<()>>>,
}
