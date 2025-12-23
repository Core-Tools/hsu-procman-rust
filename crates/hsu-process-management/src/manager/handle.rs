//! ProcessManager handle - Public API for interacting with the process manager.
//!
//! This module contains the `ProcessManager` struct which is a lightweight handle
//! that can be cloned and shared across tasks. All methods send commands to the
//! internal actor and await responses.

use super::commands::ManagerCommand;
use super::types::{ProcessInfo, ProcessManagerState, Result};
use hsu_common::ProcessError;
use tokio::sync::{mpsc, oneshot};

/// Process manager handle that provides the public API by sending commands to the actor.
///
/// This is a lightweight handle that can be cloned and shared across tasks.
/// All methods send commands to the internal actor and await responses.
#[derive(Clone)]
pub struct ProcessManager {
    pub(super) cmd_tx: mpsc::Sender<ManagerCommand>,
}

impl ProcessManager {
    // -------------------------------------------------------------------------
    // Error Mapping Helpers
    // -------------------------------------------------------------------------

    /// Map a channel send error to a ProcessError.
    fn map_send_err(context: &str) -> ProcessError {
        ProcessError::spawn_failed(
            "process-manager",
            format!("{}: actor unavailable (channel closed)", context),
        )
    }

    /// Map a oneshot receive error to a ProcessError.
    fn map_recv_err(context: &str) -> ProcessError {
        ProcessError::spawn_failed(
            "process-manager",
            format!("{}: actor dropped response (internal error)", context),
        )
    }

    // -------------------------------------------------------------------------
    // Lifecycle Methods
    // -------------------------------------------------------------------------

    /// Start the process manager and all enabled processes (idempotent).
    ///
    /// Transitions state: Initializing -> Starting -> Running
    ///
    /// The method returns only after all processes have been started.
    /// If some processes fail to start, returns an error listing the failed
    /// process IDs, but other processes are still started (best effort).
    ///
    /// # Idempotency
    /// - If already `Running` or `Starting`: returns `Ok(())` immediately without
    ///   creating a new start batch.
    /// - If `Stopping`: returns an error (cannot start while shutdown in progress).
    /// - If `Stopped`: returns an error (create a new ProcessManager instance).
    ///
    /// # Errors
    /// - Returns error if manager is already stopped
    /// - Returns error if manager is currently stopping
    /// - Returns error if any process fails to start (with aggregated message)
    /// - Returns error if actor is unavailable
    pub async fn start(&mut self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::StartAll { resp: tx })
            .await
            .map_err(|_| Self::map_send_err("start"))?;
        rx.await.map_err(|_| Self::map_recv_err("start"))?
    }

    /// Shutdown the process manager and all processes.
    ///
    /// Transitions state: * -> Stopping -> Stopped
    ///
    /// The method returns only after all processes have been stopped.
    /// The actor continues running after shutdown to allow state queries.
    /// It terminates when all handles are dropped.
    ///
    /// # Errors
    /// - Returns error if any process fails to stop (with aggregated message)
    /// - Returns error if actor is unavailable
    pub async fn shutdown(&mut self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::Shutdown { resp: tx })
            .await
            .map_err(|_| Self::map_send_err("shutdown"))?;
        rx.await.map_err(|_| Self::map_recv_err("shutdown"))?
    }

    // -------------------------------------------------------------------------
    // Process Control Methods
    // -------------------------------------------------------------------------

    /// Start a specific process by ID.
    ///
    /// If the process is already busy with another operation, the start
    /// request is queued and will be executed after the current operation
    /// completes.
    ///
    /// # Errors
    /// - `ProcessError::NotFound` if process_id is not registered
    /// - Returns error if manager is stopped or actor is unavailable
    pub async fn start_process(&self, process_id: &str) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::StartProcess {
                id: process_id.to_string(),
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("start_process"))?;
        rx.await.map_err(|_| Self::map_recv_err("start_process"))?
    }

    /// Stop a specific process by ID.
    ///
    /// If the process is already busy with another operation, the stop
    /// request is queued and will be executed after the current operation
    /// completes.
    ///
    /// # Errors
    /// - `ProcessError::NotFound` if process_id is not registered
    /// - Returns error if actor is unavailable
    pub async fn stop_process(&self, process_id: &str) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::StopProcess {
                id: process_id.to_string(),
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("stop_process"))?;
        rx.await.map_err(|_| Self::map_recv_err("stop_process"))?
    }

    /// Restart a specific process by ID.
    ///
    /// If the process is already busy with another operation, the restart
    /// request is queued and will be executed after the current operation
    /// completes.
    ///
    /// # Arguments
    /// * `process_id` - The process identifier
    /// * `force` - If true, bypasses circuit breaker for immediate restart
    ///
    /// # Errors
    /// - `ProcessError::NotFound` if process_id is not registered
    /// - Returns error if manager is stopped or actor is unavailable
    pub async fn restart_process(&self, process_id: &str, force: bool) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::RestartProcess {
                id: process_id.to_string(),
                force,
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("restart_process"))?;
        rx.await
            .map_err(|_| Self::map_recv_err("restart_process"))?
    }

    // -------------------------------------------------------------------------
    // Query Methods
    // -------------------------------------------------------------------------

    /// Get information about a specific process.
    ///
    /// If the process is currently busy with an operation, returns cached
    /// information from before the operation started.
    ///
    /// # Errors
    /// - `ProcessError::NotFound` if process_id is not registered
    /// - Returns error if actor is unavailable
    pub async fn get_process_info(&self, process_id: &str) -> Result<ProcessInfo> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::GetProcessInfo {
                id: process_id.to_string(),
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("get_process_info"))?;
        rx.await
            .map_err(|_| Self::map_recv_err("get_process_info"))?
    }

    /// Get information about all processes.
    ///
    /// # Errors
    /// - Returns error if actor is unavailable
    /// - Returns error if any process info collection fails (fail-fast)
    pub async fn get_all_process_info(&self) -> Result<Vec<ProcessInfo>> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::GetAllProcessInfo { resp: tx })
            .await
            .map_err(|_| Self::map_send_err("get_all_process_info"))?;
        rx.await
            .map_err(|_| Self::map_recv_err("get_all_process_info"))?
    }

    /// Get the current manager state.
    ///
    /// Returns `ProcessManagerState::Error` if the actor is unavailable.
    pub async fn get_manager_state(&self) -> ProcessManagerState {
        let (tx, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(ManagerCommand::GetManagerState { resp: tx })
            .await
            .is_err()
        {
            return ProcessManagerState::Error("Actor unavailable (channel closed)".to_string());
        }
        rx.await.unwrap_or(ProcessManagerState::Error(
            "Actor dropped response".to_string(),
        ))
    }

    /// Test-only helper to simulate a long-running operation without touching real processes.
    #[cfg(test)]
    pub async fn test_sleep_op(
        &self,
        process_id: &str,
        duration: std::time::Duration,
    ) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::TestSleep {
                id: process_id.to_string(),
                duration,
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("test_sleep"))?;
        rx.await.map_err(|_| Self::map_recv_err("test_sleep"))?
    }

    /// Test-only helper to simulate control handle loss for a process.
    ///
    /// This marks the process as inoperable and drains all pending operations,
    /// returning the count of drained ops. Used to test the drain logic.
    #[cfg(test)]
    pub async fn test_lose_control(&self, process_id: &str) -> Result<usize> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::TestLoseControl {
                id: process_id.to_string(),
                resp: tx,
            })
            .await
            .map_err(|_| Self::map_send_err("test_lose_control"))?;
        rx.await
            .map_err(|_| Self::map_recv_err("test_lose_control"))?
    }
}
