//! Internal command protocol for the process manager actor.
//!
//! This module contains the message types used for communication between
//! the ProcessManager handle and the ProcessManagerActor. These types
//! are NOT exposed outside the manager module.

use super::types::{ProcessInfo, ProcessManagerState, Result};
use tokio::sync::oneshot;

// ============================================================================
// Command Enum - Internal messages sent to the actor
// ============================================================================

/// Command messages for the ProcessManager actor.
///
/// This enum is internal to the manager module. External code interacts via `ProcessManager` methods.
pub(super) enum ManagerCommand {
    /// Start all enabled processes
    StartAll { resp: oneshot::Sender<Result<()>> },
    /// Shutdown all processes and stop the manager
    Shutdown { resp: oneshot::Sender<Result<()>> },
    /// Start a specific process by ID
    StartProcess {
        id: String,
        resp: oneshot::Sender<Result<()>>,
    },
    /// Stop a specific process by ID
    StopProcess {
        id: String,
        resp: oneshot::Sender<Result<()>>,
    },
    /// Restart a specific process by ID
    RestartProcess {
        id: String,
        force: bool,
        resp: oneshot::Sender<Result<()>>,
    },
    /// Get information about a specific process
    GetProcessInfo {
        id: String,
        resp: oneshot::Sender<Result<ProcessInfo>>,
    },
    /// Get information about all processes
    GetAllProcessInfo {
        resp: oneshot::Sender<Result<Vec<ProcessInfo>>>,
    },
    /// Get the current manager state
    GetManagerState {
        resp: oneshot::Sender<ProcessManagerState>,
    },
    /// Test-only: schedule a sleep operation to simulate long work
    #[cfg(test)]
    TestSleep {
        id: String,
        duration: std::time::Duration,
        resp: oneshot::Sender<Result<()>>,
    },
    /// Test-only: simulate control handle loss for a process
    /// Used to test that pending ops are properly drained when a process becomes inoperable
    #[cfg(test)]
    TestLoseControl {
        id: String,
        resp: oneshot::Sender<Result<usize>>, // Returns count of drained ops
    },
}
