//! ProcessManagerActor - Internal actor that owns state
//!
//! This module contains the actor implementation which runs in a single task
//! and processes commands from the handle. The actor owns all process state
//! and is not directly accessible from outside the manager module.

use super::commands::ManagerCommand;
use super::ops::{Job, OpCompleted, OpRunner, JOB_QUEUE_CAPACITY};
use super::types::{
    BatchKind, InFlightOp, ManagedProcessInstance, OpKind, OpRequest, OpResponder, PendingBatch,
    ProcessInfo, ProcessManagerState, Result,
};
use crate::config::{ProcessConfig, ProcessManagerConfig};
use crate::process_control_impl::ProcessControlImpl;
use hsu_common::ProcessError;
use hsu_log_collection::LogCollectionService;
use hsu_managed_process::ProcessControlConfig;
use hsu_process_file::ProcessFileManager;
use hsu_process_state::ProcessState;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, MissedTickBehavior};
use tracing::{debug, error, info, warn};

/// Maximum number of in-flight heartbeat operations to prevent task explosion
const MAX_HEARTBEAT_IN_FLIGHT: usize = 64;

/// Number of concurrent workers for the operation runner
const OP_RUNNER_CONCURRENCY: usize = 8;

/// Maximum queued operations per process
const MAX_PENDING_OPS_PER_PROCESS: usize = 32;

/// Maximum queued operations globally
const MAX_TOTAL_PENDING_OPS: usize = 1024;

/// Timeout for draining shutdown operations after command channel closes.
/// NOTE: Drain is now purely state-based (command channel closed + no pending/in-flight work).
/// We intentionally avoid a timeout here to ensure no oneshot responders are left unresolved.

/// Batch ID generator
static BATCH_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_batch_id() -> u64 {
    BATCH_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Internal actor struct that owns the process manager state.
/// This struct runs in a single task and processes commands from the handle.
pub(super) struct ProcessManagerActor {
    pub config: ProcessManagerConfig,
    pub processes: HashMap<String, ManagedProcessInstance>,
    pub state: ProcessManagerState,
    /// Process file manager for PID files (wrapped in Arc for sharing)
    pub pid_file_manager: Arc<ProcessFileManager>,
    pub log_collection_service: Option<Arc<LogCollectionService>>,
    /// Operation runner for heavy work
    op_runner: OpRunner,
    /// Pending batch operations (StartAll, Shutdown)
    pending_batches: HashMap<u64, PendingBatch>,
    /// Total queued operations across all processes
    total_pending_ops: usize,
    /// Set once we observe the completion channel has closed.
    /// Used to make `handle_completion_channel_closed()` idempotent.
    completion_closed: bool,
    /// Set once we detect a mismatch between `total_pending_ops` and the actual per-process
    /// queue lengths. Used to avoid log spam while still surfacing invariant drift.
    drain_counter_mismatch_logged: bool,
}

impl ProcessManagerActor {
    /// Create a new actor with the given configuration and command channel.
    pub(super) fn new(
        config: ProcessManagerConfig,
        pid_file_manager: ProcessFileManager,
        log_collection_service: Option<Arc<LogCollectionService>>,
        completed_tx: mpsc::Sender<OpCompleted>,
    ) -> Self {
        let pid_file_manager = Arc::new(pid_file_manager);
        let op_runner =
            OpRunner::new(OP_RUNNER_CONCURRENCY, completed_tx, Arc::clone(&pid_file_manager));

        ProcessManagerActor {
            config,
            processes: HashMap::new(),
            state: ProcessManagerState::Initializing,
            pid_file_manager,
            log_collection_service,
            op_runner,
            pending_batches: HashMap::new(),
            total_pending_ops: 0,
            completion_closed: false,
            drain_counter_mismatch_logged: false,
        }
    }

    /// Main event loop for the actor.
    ///
    /// This loop listens to:
    /// - External commands (handle -> actor)
    /// - Operation completions (OpRunner -> actor)
    /// - Heartbeat ticks
    ///
    /// ## Termination
    ///
    /// The actor terminates when:
    /// - The command channel is closed (all external handles dropped), AND
    /// - No pending operations remain (no queued ops, no in-flight ops, no pending batches)
    ///
    /// Completion messages are still processed after command channel closure to ensure
    /// in-flight work is drained and no oneshot responders remain unresolved.
    pub(super) async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<ManagerCommand>,
        mut completed_rx: mpsc::Receiver<OpCompleted>,
    ) {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
        // Don't try to catch up on missed heartbeats - just skip them
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // Drain mode state: set when command channel closes.
        // In drain mode we stop accepting commands, but continue processing completions.
        let mut cmd_closed = false;

        loop {
            // Termination condition: command channel closed AND no pending/in-flight work.
            if cmd_closed && self.is_fully_drained() {
                info!("Command channel closed and all work drained; actor exiting");
                break;
            }

            // We intentionally bias the select to guarantee completion processing latency:
            // - Completions must not be starved (they clear in-flight ops and resolve oneshots/batches)
            // - Commands should not be starved by heartbeats
            // - Heartbeat is best-effort background work and may be delayed under load
            tokio::select! {
                biased;

                // Always process completions while the completion channel is alive (even after the
                // command channel closes). If the completion channel closes, we force-drain once
                // and then stop polling it to avoid a biased-select busy loop.
                maybe_completed = completed_rx.recv(), if !self.completion_closed => {
                    if let Some(completed) = maybe_completed {
                        self.handle_op_completed(completed);
                    } else {
                        // Completion channel closed unexpectedly. If there is any pending/in-flight work,
                        // it can never complete. Treat this as an unrecoverable failure and force-drain
                        // all pending work so no oneshot responders hang.
                        error!("Completion channel closed unexpectedly; forcing drain/fail of all pending work");
                        self.handle_completion_channel_closed();
                    }
                }

                // Only poll command receiver while it is open; once closed, do not poll to avoid
                // busy-looping (recv() would immediately return None).
                maybe_cmd = cmd_rx.recv(), if !cmd_closed => {
                    match maybe_cmd {
                        None => {
                            info!("Command channel closed; entering drain mode");
                            cmd_closed = true;

                            // Initiate shutdown if not already stopping/stopped (idempotent).
                            // This ensures child processes are stopped even when all handles are dropped.
                            if !matches!(self.state, ProcessManagerState::Stopping | ProcessManagerState::Stopped) {
                                self.initiate_shutdown(None);
                            }
                        }
                        Some(cmd) => self.handle_command(cmd),
                    }
                }

                _ = heartbeat.tick() => {
                    // Heartbeats are disabled in drain mode: we are shutting down and should not
                    // schedule additional work beyond what's already pending/in-flight.
                    if !cmd_closed && matches!(self.state, ProcessManagerState::Running) {
                        self.schedule_heartbeat_ops();
                    }
                }
            }
        }

        info!("ProcessManager actor terminated");
    }

    /// Handle unexpected completion channel closure.
    ///
    /// If the completion channel closes, no `OpCompleted` messages can arrive to clear
    /// `in_flight`, advance per-process queues, or complete batch counters. This is fatal.
    ///
    /// This method is idempotent.
    fn handle_completion_channel_closed(&mut self) {
        if self.completion_closed {
            return;
        }
        self.completion_closed = true;

        // Transition manager state to terminal.
        // Prefer `Stopped` to match current semantics (terminal state).
        self.state = ProcessManagerState::Stopped;

        // Fail all pending batches so batch-level oneshots never hang.
        let batch_err = ProcessError::completion_channel_closed("process-manager");
        for (_batch_id, batch) in self.pending_batches.drain() {
            if let Some(resp) = batch.resp {
                let _ = resp.send(Err(batch_err.clone()));
            }
        }

        // Fail all in-flight and pending per-process operations.
        // NOTE: We do not attempt to recover control handles; any in-flight job will never
        // return its control handle without completions.
        let process_ids: Vec<String> = self.processes.keys().cloned().collect();
        for process_id in process_ids {
            let per_err = ProcessError::completion_channel_closed(process_id.clone());
            if let Some(instance) = self.processes.get_mut(&process_id) {
                // Clear busy/in-flight and resolve responder if present.
                if let Some(in_flight) = instance.in_flight.take() {
                    if let Some(resp) = in_flight.resp {
                        Self::send_op_resp(resp, Err(per_err.clone()));
                    }
                }

                // Mark process inoperable (no control handle available).
                instance.control = None;
                instance.last_known_state = ProcessState::Failed;
                instance.last_known_pid = None;
            }

            // Drain any queued operations and resolve their responders with error.
            self.fail_and_drain_pending_ops(&process_id, per_err);
        }

        // Ensure global counters are consistent for termination.
        self.total_pending_ops = 0;
    }

    /// Handle a single command (non-blocking)
    fn handle_command(&mut self, cmd: ManagerCommand) {
        use ManagerCommand::*;

        // If the completion channel has closed, the actor can no longer execute operations
        // or observe completions to clear in-flight state. Reject all mutating commands
        // deterministically with CompletionChannelClosed to avoid misleading errors.
        //
        // Queries are still served from cached state to keep the actor responsive.
        if self.completion_closed {
            match cmd {
                StartAll { resp } => {
                    let _ = resp.send(Err(ProcessError::completion_channel_closed(
                        "process-manager",
                    )));
                }
                Shutdown { resp } => {
                    let _ = resp.send(Err(ProcessError::completion_channel_closed(
                        "process-manager",
                    )));
                }
                StartProcess { id, resp }
                | StopProcess { id, resp }
                | RestartProcess {
                    id,
                    force: _,
                    resp,
                } => {
                    let _ = resp.send(Err(ProcessError::completion_channel_closed(id)));
                }
                GetProcessInfo { id, resp } => {
                    self.handle_get_process_info(id, resp);
                }
                GetAllProcessInfo { resp } => {
                    let res = self.handle_get_all_process_info();
                    let _ = resp.send(res);
                }
                GetManagerState { resp } => {
                    let _ = resp.send(self.state.clone());
                }
                #[cfg(test)]
                ManagerCommand::TestSleep {
                    id,
                    duration: _,
                    resp,
                } => {
                    let _ = resp.send(Err(ProcessError::completion_channel_closed(id)));
                }
                #[cfg(test)]
                ManagerCommand::TestLoseControl { id, resp } => {
                    let _ = resp.send(Err(ProcessError::completion_channel_closed(id)));
                }
            }
            return;
        }

        match cmd {
            StartAll { resp } => {
                // Stopped is terminal: cannot start a stopped manager
                if matches!(self.state, ProcessManagerState::Stopped) {
                    debug!("StartAll rejected: manager is stopped (terminal state)");
                    let _ = resp.send(Err(ProcessError::operation_not_allowed(
                        "process-manager",
                        "start_all",
                        "Stopped",
                    )));
                } else {
                    self.handle_start_all(resp);
                }
            }
            Shutdown { resp } => {
                self.initiate_shutdown(Some(resp));
            }
            StartProcess { id, resp } => {
                if matches!(self.state, ProcessManagerState::Stopped) {
                    let _ = resp.send(Err(ProcessError::operation_not_allowed(
                        id,
                        "start_process",
                        "Stopped",
                    )));
                } else {
                    self.schedule_op(id, OpKind::Start, Some(OpResponder::Unit(resp)), None);
                }
            }
            StopProcess { id, resp } => {
                // StopProcess is allowed even after shutdown (idempotent)
                self.schedule_op(id, OpKind::Stop, Some(OpResponder::Unit(resp)), None);
            }
            RestartProcess { id, force, resp } => {
                if matches!(self.state, ProcessManagerState::Stopped) {
                    let _ = resp.send(Err(ProcessError::operation_not_allowed(
                        id,
                        "restart_process",
                        "Stopped",
                    )));
                } else {
                    self.schedule_op(
                        id,
                        OpKind::Restart { force },
                        Some(OpResponder::Unit(resp)),
                        None,
                    );
                }
            }
            GetProcessInfo { id, resp } => {
                self.handle_get_process_info(id, resp);
            }
            GetAllProcessInfo { resp } => {
                let res = self.handle_get_all_process_info();
                let _ = resp.send(res);
            }
            GetManagerState { resp } => {
                let _ = resp.send(self.state.clone());
            }
            #[cfg(test)]
            ManagerCommand::TestSleep { id, duration, resp } => {
                self.schedule_op(
                    id,
                    OpKind::TestSleep(Duration::from(duration)),
                    Some(OpResponder::Unit(resp)),
                    None,
                );
            }
            #[cfg(test)]
            ManagerCommand::TestLoseControl { id, resp } => {
                self.handle_test_lose_control(id, resp);
            }
        }
    }

    /// Schedule an operation on a process.
    ///
    /// If the process is idle, starts the operation immediately.
    /// If the process is busy, queues the operation for later execution.
    ///
    /// # Batch Invariant (must not hang)
    ///
    /// If `batch_id.is_some()`, this operation contributes to a batch's `pending_count`.
    /// Therefore, this method must **never** silently drop the operation in any saturation
    /// path. If the operation cannot be queued or started, it must be accounted for by
    /// calling `batch_op_completed(batch_id, Some(process_id))` before returning.
    fn schedule_op(
        &mut self,
        process_id: String,
        op: OpKind,
        resp: Option<OpResponder>,
        batch_id: Option<u64>,
    ) {
        // Find instance
        let instance = match self.processes.get_mut(&process_id) {
            Some(inst) => inst,
            None => {
                let err = ProcessError::NotFound {
                    id: process_id.clone(),
                };
                if let Some(resp) = resp {
                    Self::send_op_resp(resp, Err(err));
                }
                // If part of a batch, count it as failed
                if let Some(batch_id) = batch_id {
                    self.batch_op_completed(batch_id, Some(process_id));
                }
                return;
            }
        };

        // Enforce queue limits
        if instance.pending_ops.len() >= MAX_PENDING_OPS_PER_PROCESS
            || self.total_pending_ops >= MAX_TOTAL_PENDING_OPS
        {
            let limit = if instance.pending_ops.len() >= MAX_PENDING_OPS_PER_PROCESS {
                MAX_PENDING_OPS_PER_PROCESS
            } else {
                MAX_TOTAL_PENDING_OPS
            };
            debug!(
                "Queue limit reached for process {} (local {}, global {}), op {}",
                process_id,
                instance.pending_ops.len(),
                self.total_pending_ops,
                op.name()
            );
            // If this is a user-visible responder, return error.
            if let Some(resp) = resp {
                let err = ProcessError::queue_full(&process_id, limit);
                Self::send_op_resp(resp, Err(err));
                if let Some(batch_id) = batch_id {
                    self.batch_op_completed(batch_id, Some(process_id));
                }
                return;
            }
            // No responder (heartbeat or internal fire-and-forget).
            // IMPORTANT: Batch operations must never be silently dropped.
            if let Some(batch_id) = batch_id {
                self.batch_op_completed(batch_id, Some(process_id));
                return;
            }

            // Non-batch fire-and-forget (e.g. heartbeat): allowed to silently drop.
            return;
        }

        // If busy, queue the operation
        if instance.is_busy() {
            debug!(
                "Process '{}' busy with {}, queueing {} operation",
                process_id,
                instance.busy_op_name().unwrap_or("operation"),
                op.name()
            );
            instance.pending_ops.push_back(OpRequest {
                kind: op,
                resp,
                batch_id,
            });
            self.total_pending_ops += 1;
            return;
        }

        // Start the operation immediately
        self.start_op_now(&process_id, op, resp, batch_id);
    }

    /// Start an operation immediately (process must be idle).
    fn start_op_now(
        &mut self,
        process_id: &str,
        op: OpKind,
        resp: Option<OpResponder>,
        batch_id: Option<u64>,
    ) {
        let instance = match self.processes.get_mut(process_id) {
            Some(inst) => inst,
            None => return, // Shouldn't happen if called correctly
        };

        // Check if control is available
        let control = match instance.control.take() {
            Some(c) => c,
            None => {
                let err = ProcessError::spawn_failed(
                    "process-manager",
                    format!(
                        "Process '{}' has no control handle (internal error)",
                        process_id
                    ),
                );
                if let Some(resp) = resp {
                    Self::send_op_resp(resp, Err(err));
                }
                if let Some(batch_id) = batch_id {
                    self.batch_op_completed(batch_id, Some(process_id.to_string()));
                }
                return;
            }
        };

        // Mark as in-flight
        instance.in_flight = Some(InFlightOp {
            kind: op.clone(),
            resp,
            batch_id,
        });

        // Create job and submit
        let job = Job {
            process_id: process_id.to_string(),
            op,
            control,
            batch_id,
        };

        // Submit to runner using try_send (non-blocking, fails if channel full)
        match self.op_runner.job_tx.try_send(job) {
            Ok(()) => {
                // Job submitted successfully
            }
            Err(mpsc::error::TrySendError::Full(job)) => {
                // Channel full - restore control and notify responder
                error!("OpRunner job channel full, cannot submit job for {}", process_id);
                if let Some(inst) = self.processes.get_mut(process_id) {
                    inst.control = Some(job.control);
                    // Notify responder before dropping in_flight
                    if let Some(in_flight) = inst.in_flight.take() {
                        if let Some(resp) = in_flight.resp {
                            Self::send_op_resp(
                                resp,
                                Err(ProcessError::queue_full(process_id, JOB_QUEUE_CAPACITY)),
                            );
                        }
                    }
                }
                // Fail the batch if applicable
                if let Some(batch_id) = batch_id {
                    self.batch_op_completed(batch_id, Some(process_id.to_string()));
                }
            }
            Err(mpsc::error::TrySendError::Closed(job)) => {
                // Channel closed - restore control and notify responder
                error!("OpRunner job channel closed");
                if let Some(inst) = self.processes.get_mut(process_id) {
                    inst.control = Some(job.control);
                    // Notify responder before dropping in_flight
                    if let Some(in_flight) = inst.in_flight.take() {
                        if let Some(resp) = in_flight.resp {
                            Self::send_op_resp(
                                resp,
                                Err(ProcessError::spawn_failed(
                                    process_id,
                                    "operation runner shutdown".to_string(),
                                )),
                            );
                        }
                    }
                }
                if let Some(batch_id) = batch_id {
                    self.batch_op_completed(batch_id, Some(process_id.to_string()));
                }
            }
        }
    }

    fn send_op_resp(resp: OpResponder, result: Result<()>) {
        match resp {
            OpResponder::Unit(tx) => {
                let _ = tx.send(result);
            }
            OpResponder::ProcessInfo(tx) => match result {
                Ok(()) => {
                    let _ = tx.send(Err(ProcessError::spawn_failed(
                        "process-manager",
                        "missing diagnostics payload".to_string(),
                    )));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            },
        }
    }

    /// Drain all pending operations for a process and respond with an error to each.
    ///
    /// Called when a process becomes "inoperable" (control handle lost, typically due
    /// to worker panic). Ensures no pending operations remain stuck indefinitely.
    ///
    /// This method:
    /// - Pops all ops from the process's `pending_ops` queue
    /// - Sends `Err(err)` to each responder
    /// - Decrements `total_pending_ops` for each drained op
    /// - Notifies any associated batches of failure
    ///
    /// Returns the number of operations drained.
    fn fail_and_drain_pending_ops(&mut self, process_id: &str, err: ProcessError) -> usize {
        let instance = match self.processes.get_mut(process_id) {
            Some(inst) => inst,
            None => return 0,
        };

        let drain_count = instance.pending_ops.len();
        if drain_count == 0 {
            return 0;
        }

        warn!(
            "Draining {} pending op(s) for inoperable process '{}': {}",
            drain_count, process_id, err
        );

        // Collect all pending ops
        let pending: Vec<OpRequest> = instance.pending_ops.drain(..).collect();

        // Update global counter
        self.total_pending_ops = self.total_pending_ops.saturating_sub(pending.len());

        // Respond to each with error and handle batch tracking
        for op_request in pending {
            if let Some(resp) = op_request.resp {
                Self::send_op_resp(resp, Err(err.clone()));
            }
            // If this op was part of a batch, mark it as failed
            if let Some(batch_id) = op_request.batch_id {
                self.batch_op_completed(batch_id, Some(process_id.to_string()));
            }
        }

        drain_count
    }

    /// Test-only: Simulate losing the control handle for a process.
    ///
    /// This marks the process as inoperable (control = None, state = Failed)
    /// and drains all pending operations. If an operation is in-flight, it
    /// also fails that operation's responder. Returns the count of drained
    /// pending ops (not counting in-flight).
    #[cfg(test)]
    fn handle_test_lose_control(
        &mut self,
        process_id: String,
        resp: oneshot::Sender<Result<usize>>,
    ) {
        // Check if process exists
        let instance = match self.processes.get_mut(&process_id) {
            Some(inst) => inst,
            None => {
                let _ = resp.send(Err(ProcessError::NotFound { id: process_id }));
                return;
            }
        };

        // Drop the control handle to simulate loss
        instance.control = None;
        instance.last_known_state = ProcessState::Failed;

        // If there's an in-flight operation, fail its responder
        if let Some(in_flight) = instance.in_flight.take() {
            let in_flight_err = ProcessError::spawn_failed(
                &process_id,
                "operation failed due to simulated control loss".to_string(),
            );
            if let Some(in_flight_resp) = in_flight.resp {
                Self::send_op_resp(in_flight_resp, Err(in_flight_err));
            }
            // Handle batch if applicable
            if let Some(batch_id) = in_flight.batch_id {
                self.batch_op_completed(batch_id, Some(process_id.clone()));
            }
        }

        // Drain pending ops
        let drain_err = ProcessError::spawn_failed(
            &process_id,
            "process is inoperable (control handle lost - simulated for test)".to_string(),
        );
        let drained = self.fail_and_drain_pending_ops(&process_id, drain_err);

        let _ = resp.send(Ok(drained));
    }

    /// Handle operation completion
    fn handle_op_completed(&mut self, completed: OpCompleted) {
        let OpCompleted {
            process_id,
            op,
            control,
            result,
            batch_id,
            restarts_processed,
            diagnostics,
        } = completed;

        // Track whether control handle was lost (for draining pending ops)
        let control_lost = control.is_none();

        // Find instance and restore control
        let maybe_in_flight = if let Some(instance) = self.processes.get_mut(&process_id) {
            // Restore control handle if present (None means task panicked)
            if let Some(ctrl) = control {
                instance.control = Some(ctrl);

                // Update tracking based on operation (only if we have control)
                match &op {
                    OpKind::Start if result.is_ok() => {
                        if let Some(ref ctrl) = instance.control {
                            instance.last_known_pid = ctrl.get_pid();
                            instance.last_known_state = ctrl.get_state();
                        }
                    }
                    OpKind::Stop if result.is_ok() => {
                        instance.last_known_pid = None;
                        instance.last_known_state = ProcessState::Stopped;
                    }
                    OpKind::Restart { .. } if result.is_ok() => {
                        instance.restart_count += 1;
                        instance.last_restart_time = Some(chrono::Utc::now());
                        if let Some(ref ctrl) = instance.control {
                            instance.last_known_pid = ctrl.get_pid();
                            instance.last_known_state = ctrl.get_state();
                        }
                    }
                    OpKind::HeartbeatPoll if result.is_ok() && restarts_processed > 0 => {
                        instance.restart_count += restarts_processed as u32;
                        instance.last_restart_time = Some(chrono::Utc::now());
                    }
                    OpKind::QueryDiagnostics => {
                        if let Some(ref diag) = diagnostics {
                            instance.last_known_pid = diag.process_id;
                            instance.last_known_state = diag.state;
                        }
                    }
                    #[cfg(test)]
                    OpKind::TestSleep(_) => {}
                    _ => {}
                }

                debug!(
                    "Operation {} completed for process {}: {:?}",
                    op.name(),
                    process_id,
                    result.is_ok()
                );
            } else {
                // Control handle was lost (task panicked) - process is now inoperable
                error!(
                    "Control handle lost for process {} during {} operation (task panicked). \
                     Process is now inoperable until manager restart.",
                    process_id,
                    op.name()
                );
                instance.last_known_state = ProcessState::Failed;
            }

            // Take in_flight info
            instance.in_flight.take()
        } else {
            // Instance was removed (shouldn't happen normally)
            warn!(
                "OpCompleted for unknown process {}, control handle dropped",
                process_id
            );
            None
        };

        // Track if there was an error for batch handling
        let had_error = result.is_err();
        let failed_id = if had_error {
            Some(process_id.clone())
        } else {
            None
        };

        // Send reply if present (from in_flight, not from OpCompleted)
        if let Some(in_flight) = maybe_in_flight {
            if let Some(resp) = in_flight.resp {
                match resp {
                    OpResponder::Unit(tx) => {
                        let _ = tx.send(result);
                    }
                    OpResponder::ProcessInfo(tx) => {
                        match result {
                            Ok(()) => {
                                if let Some(diag) = diagnostics {
                                    let mut info = Self::process_info_from_diag(&process_id, &diag);
                                    if let Some(inst) = self.processes.get(&process_id) {
                                        info.restart_count = inst.restart_count;
                                    }
                                    let _ = tx.send(Ok(info));
                                } else {
                                    let _ = tx.send(Err(ProcessError::spawn_failed(
                                        "process-manager",
                                        "missing diagnostics payload".to_string(),
                                    )));
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e));
                            }
                        }
                    }
                }
            }

            // Handle batch completion using batch_id from in_flight
            if let Some(batch_id) = in_flight.batch_id {
                self.batch_op_completed(batch_id, failed_id);
            }
        } else if let Some(batch_id) = batch_id {
            // Fallback: use batch_id from OpCompleted
            self.batch_op_completed(batch_id, failed_id);
        }

        // If control was lost, the process is inoperable - drain all pending ops
        // to prevent them from being stuck forever
        if control_lost {
            let drain_err = ProcessError::spawn_failed(
                &process_id,
                "process is inoperable (control handle lost due to worker panic)".to_string(),
            );
            self.fail_and_drain_pending_ops(&process_id, drain_err);
            // Don't try to start next op - there's no control handle
            return;
        }

        // Start next queued operation for this process
        self.start_next_queued_op(&process_id);
    }

    /// Start the next queued operation for a process (if any)
    fn start_next_queued_op(&mut self, process_id: &str) {
        // First check if process is operable (has control handle)
        // If not, drain all pending ops to prevent them from being stuck
        {
            let instance = match self.processes.get(process_id) {
                Some(inst) => inst,
                None => return,
            };

            // If still busy (shouldn't happen), don't start another
            if instance.is_busy() {
                return;
            }

            // If no control handle, process is inoperable - drain pending ops
            if instance.control.is_none() && !instance.pending_ops.is_empty() {
                // End the immutable borrow before calling drain helper
                // (the borrow ends when the scope closes below)
            } else {
                // Process is operable or has no pending ops - proceed normally
                return self.start_next_queued_op_inner(process_id);
            }
        }

        // If we reach here, control is None and there are pending ops - drain them
        let drain_err = ProcessError::spawn_failed(
            process_id,
            "process is inoperable (no control handle available)".to_string(),
        );
        self.fail_and_drain_pending_ops(process_id, drain_err);
    }

    /// Inner implementation of start_next_queued_op for when process is operable.
    fn start_next_queued_op_inner(&mut self, process_id: &str) {

        let next_op = {
            let instance = match self.processes.get_mut(process_id) {
                Some(inst) => inst,
                None => return,
            };

            let op = instance.pending_ops.pop_front();
            if op.is_some() && self.total_pending_ops > 0 {
                self.total_pending_ops -= 1;
            }
            op
        };

        if let Some(op_request) = next_op {
            debug!(
                "Starting queued {} operation for process {}",
                op_request.kind.name(),
                process_id
            );
            self.start_op_now(
                process_id,
                op_request.kind,
                op_request.resp,
                op_request.batch_id,
            );
        }
    }

    /// Record completion of a batch operation
    fn batch_op_completed(&mut self, batch_id: u64, failed_process_id: Option<String>) {
        if let Some(batch) = self.pending_batches.get_mut(&batch_id) {
            // Detect underflow/double-complete: this should never be called with pending_count == 0
            // for a batch that still exists.
            match batch.pending_count.checked_sub(1) {
                Some(next) => batch.pending_count = next,
                None => {
                    error!(
                        "Batch {} pending_count underflow (double-complete?): kind={:?}, failed_process_id={:?}",
                        batch_id, batch.kind, failed_process_id
                    );
                    batch.pending_count = 0;
                }
            }
            if let Some(pid) = failed_process_id {
                batch.failed_process_ids.push(pid);
            }

            if batch.pending_count == 0 {
                // Batch complete, send response and handle state transitions
                let batch = self.pending_batches.remove(&batch_id).unwrap();
                let result = if batch.failed_process_ids.is_empty() {
                    Ok(())
                } else {
                    Err(ProcessError::spawn_failed(
                        "process-manager",
                        format!(
                            "Failed to {} process(es): {}",
                            match batch.kind {
                                BatchKind::StartAll => "start",
                                BatchKind::Shutdown => "stop",
                            },
                            batch.failed_process_ids.join(", ")
                        ),
                    ))
                };

                // Handle state transitions based on batch kind
                match batch.kind {
                    BatchKind::StartAll => {
                        // Transition from Starting to Running
                        if matches!(self.state, ProcessManagerState::Starting) {
                            self.state = ProcessManagerState::Running;
                            info!("Process manager started successfully");
                        }
                    }
                    BatchKind::Shutdown => {
                        // Transition from Stopping to Stopped
                        if matches!(self.state, ProcessManagerState::Stopping) {
                            self.state = ProcessManagerState::Stopped;
                            info!("Process manager shut down successfully");
                        }
                    }
                }

                // Send result to responder if present (None for internal batches)
                if let Some(resp) = batch.resp {
                    let _ = resp.send(result);
                }
            }
        } else {
            // This indicates a logic error (e.g., a completion arriving for a batch that was already
            // completed/failed and removed). Keep it at debug to avoid log spam in release.
            debug!("batch_op_completed called for unknown batch_id {}", batch_id);
        }
    }

    /// Returns true when the actor has no pending or in-flight work remaining.
    ///
    /// This is used to decide when it is safe to terminate after the command channel closes.
    fn is_fully_drained(&mut self) -> bool {
        // IMPORTANT: Termination must be based on authoritative state (per-process queues/in-flight
        // ops and pending batches). `total_pending_ops` is a performance hint and may drift if a
        // bug ever misses an increment/decrement. A drift must not prevent shutdown/termination.
        if !self.pending_batches.is_empty() {
            return false;
        }
        let all_clear = self
            .processes
            .values()
            .all(|inst| !inst.is_busy() && inst.pending_ops.is_empty());
        if !all_clear {
            return false;
        }

        // If the fast counter disagrees, warn once (helps catch bugs without risking hangs).
        let actual_pending: usize = self.processes.values().map(|p| p.pending_ops.len()).sum();
        if self.total_pending_ops != actual_pending && !self.drain_counter_mismatch_logged {
            self.drain_counter_mismatch_logged = true;
            warn!(
                "total_pending_ops counter mismatch during drain: counter={}, actual={}",
                self.total_pending_ops, actual_pending
            );
        }

        true
    }

    /// Test-only hook to simulate counter drift without relying on internal queue manipulation.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn test_set_total_pending_ops(&mut self, value: usize) {
        self.total_pending_ops = value;
    }

    /// Handle StartAll command - start all enabled processes (non-blocking, idempotent).
    ///
    /// # State Policy
    /// - `Running` or `Starting`: Idempotent - immediately responds `Ok(())` without
    ///   creating a new batch. The manager is already started or starting.
    /// - `Stopping`: Returns error - cannot start while shutdown is in progress.
    /// - `Stopped`: **Not handled here** - caller (`handle_command`) rejects with
    ///   `OperationNotAllowed` before this method is reached. Stopped is terminal.
    /// - `Initializing` or `Error`: Proceeds with the StartAll batch as normal.
    fn handle_start_all(&mut self, resp: oneshot::Sender<Result<()>>) {
        // Idempotent: if already running or starting, send Ok and return
        if matches!(
            self.state,
            ProcessManagerState::Running | ProcessManagerState::Starting
        ) {
            debug!(
                "StartAll called while already {:?}, responding Ok (idempotent)",
                self.state
            );
            let _ = resp.send(Ok(()));
            return;
        }

        // Cannot start while stopping
        if matches!(self.state, ProcessManagerState::Stopping) {
            debug!("StartAll rejected: manager is stopping");
            let _ = resp.send(Err(ProcessError::operation_not_allowed(
                "process-manager",
                "start_all",
                "Stopping",
            )));
            return;
        }

        info!("Starting process manager");

        self.state = ProcessManagerState::Starting;

        // Get all process IDs
        let process_ids: Vec<String> = self.processes.keys().cloned().collect();

        if process_ids.is_empty() {
            // No processes to start - transition immediately to Running
            self.state = ProcessManagerState::Running;
            let _ = resp.send(Ok(()));
            info!("Process manager started successfully (no processes to start)");
            return;
        }

        // Create batch tracking - state transitions happen in batch_op_completed
        let batch_id = next_batch_id();
        let batch = PendingBatch {
            kind: BatchKind::StartAll,
            pending_count: process_ids.len(),
            failed_process_ids: Vec::new(),
            resp: Some(resp),
        };
        self.pending_batches.insert(batch_id, batch);

        // Schedule all starts (they may queue if process is somehow busy)
        for process_id in process_ids {
            self.schedule_op(process_id, OpKind::Start, None, Some(batch_id));
        }

        // NOTE: State remains Starting until batch completes
        info!("StartAll initiated, waiting for processes to start...");
    }

    /// Initiate shutdown of all processes (non-blocking, idempotent).
    ///
    /// If already stopping or stopped, immediately sends `Ok(())` to the
    /// responder (if provided) and returns. Shutdown is fully idempotent:
    /// duplicate calls always succeed.
    fn initiate_shutdown(&mut self, resp: Option<oneshot::Sender<Result<()>>>) {
        // Idempotent: if already stopping/stopped, send Ok and return
        if matches!(
            self.state,
            ProcessManagerState::Stopping | ProcessManagerState::Stopped
        ) {
            debug!(
                "Shutdown already in progress or completed (state: {:?}), responding Ok to duplicate request",
                self.state
            );
            if let Some(resp) = resp {
                let _ = resp.send(Ok(()));
            }
            return;
        }

        info!("Shutting down process manager");

        self.state = ProcessManagerState::Stopping;

        // Get all process IDs
        let process_ids: Vec<String> = self.processes.keys().cloned().collect();

        if process_ids.is_empty() {
            self.state = ProcessManagerState::Stopped;
            if let Some(resp) = resp {
                let _ = resp.send(Ok(()));
            }
            info!("Process manager shut down successfully (no processes to stop)");
            return;
        }

        // Always create batch tracking (even without responder) to ensure state transitions
        let batch_id = next_batch_id();
        let batch = PendingBatch {
            kind: BatchKind::Shutdown,
            pending_count: process_ids.len(),
            failed_process_ids: Vec::new(),
            resp, // None for internal shutdown, Some for explicit Shutdown command
        };
        self.pending_batches.insert(batch_id, batch);

        // Schedule all stops (they will queue behind any in-progress operations)
        for process_id in process_ids {
            self.schedule_op(process_id, OpKind::Stop, None, Some(batch_id));
        }

        info!("Shutdown initiated, waiting for processes to stop...");
    }

    /// Schedule heartbeat operations for idle processes
    fn schedule_heartbeat_ops(&mut self) {
        // Count current in-flight operations
        let in_flight_count = self
            .processes
            .values()
            .filter(|inst| inst.is_busy())
            .count();

        if in_flight_count >= MAX_HEARTBEAT_IN_FLIGHT {
            debug!(
                "Skipping heartbeat: {} operations already in-flight",
                in_flight_count
            );
            return;
        }

        // Collect process IDs that are not busy and have no pending ops
        if self.total_pending_ops >= MAX_TOTAL_PENDING_OPS {
            debug!("Skipping heartbeat: global pending queue is full");
            return;
        }

        let available_processes: Vec<String> = self
            .processes
            .iter()
            .filter(|(_, inst)| !inst.is_busy() && inst.pending_ops.is_empty())
            .take(MAX_HEARTBEAT_IN_FLIGHT - in_flight_count)
            .map(|(id, _)| id.clone())
            .collect();

        // Schedule heartbeat poll for each available process
        for process_id in available_processes {
            self.schedule_op(process_id, OpKind::HeartbeatPoll, None, None);
        }
    }

    /// Build ProcessInfo from diagnostics payload
    fn process_info_from_diag(
        process_id: &str,
        diag: &hsu_managed_process::ProcessDiagnostics,
    ) -> ProcessInfo {
        ProcessInfo {
            id: process_id.to_string(),
            state: diag.state,
            pid: diag.process_id,
            start_time: diag.start_time,
            restart_count: 0, // caller should override if needed
            cpu_usage: diag.cpu_usage,
            memory_usage: diag.memory_usage,
            uptime: diag.start_time.map(|st| {
                let now = chrono::Utc::now();
                let duration = now.signed_duration_since(st);
                Duration::from_secs(duration.num_seconds().max(0) as u64)
            }),
            is_healthy: diag.is_healthy,
            last_health_check: None,
            consecutive_health_failures: diag.failure_count,
        }
    }

    /// Handle GetProcessInfo command.
    /// If control is available and process idle, offload diagnostics collection to OpRunner.
    /// If busy or control unavailable, return cached info immediately to keep actor non-blocking.
    fn handle_get_process_info(
        &mut self,
        process_id: String,
        resp: oneshot::Sender<Result<ProcessInfo>>,
    ) {
        let instance = match self.processes.get(&process_id) {
            Some(inst) => inst,
            None => {
                let _ = resp.send(Err(ProcessError::NotFound { id: process_id }));
                return;
            }
        };

        // If busy or no control, return cached/minimal info
        if instance.is_busy() || instance.control.is_none() {
            let info = ProcessInfo {
                id: process_id.clone(),
                state: instance.last_known_state,
                pid: instance.last_known_pid,
                start_time: None,
                restart_count: instance.restart_count,
                cpu_usage: None,
                memory_usage: None,
                uptime: None,
                is_healthy: true,
                last_health_check: None,
                consecutive_health_failures: 0,
            };
            let _ = resp.send(Ok(info));
            return;
        }

        // Otherwise schedule async diagnostics via OpRunner
        self.schedule_op(
            process_id,
            OpKind::QueryDiagnostics,
            Some(OpResponder::ProcessInfo(resp)),
            None,
        );
    }

    /// Handle GetAllProcessInfo command.
    fn handle_get_all_process_info(&self) -> Result<Vec<ProcessInfo>> {
        let mut infos = Vec::with_capacity(self.processes.len());

        for (id, instance) in self.processes.iter() {
            // Always use cached data to keep this call non-blocking
            // (control.get_state()/get_pid() could potentially block)
            let info = ProcessInfo {
                id: id.to_string(),
                state: instance.last_known_state,
                pid: instance.last_known_pid,
                start_time: None,
                restart_count: instance.restart_count,
                cpu_usage: None,
                memory_usage: None,
                uptime: None,
                is_healthy: true,
                last_health_check: None,
                consecutive_health_failures: 0,
            };
            infos.push(info);
        }

        Ok(infos)
    }

    /// Initialize all processes from configuration
    pub(super) async fn initialize_processes(&mut self) -> Result<()> {
        for process_config in &self.config.managed_processes {
            if !process_config.enabled {
                debug!("Skipping disabled process: {}", process_config.id);
                continue;
            }

            // Extract graceful timeout based on process type
            let graceful_timeout = match &process_config.process_type {
                crate::config::ProcessManagementType::StandardManaged => process_config
                    .management
                    .standard_managed
                    .as_ref()
                    .and_then(|sm| sm.control.graceful_timeout)
                    .unwrap_or(Duration::from_secs(10)),
                crate::config::ProcessManagementType::IntegratedManaged => process_config
                    .management
                    .integrated_managed
                    .as_ref()
                    .and_then(|im| im.control.graceful_timeout)
                    .unwrap_or(Duration::from_secs(10)),
                crate::config::ProcessManagementType::Unmanaged => process_config
                    .management
                    .unmanaged
                    .as_ref()
                    .and_then(|u| u.control.as_ref())
                    .map(|c| c.graceful_timeout)
                    .unwrap_or(Duration::from_secs(10)),
            };

            // Create ProcessControl implementation
            let control_config = ProcessControlConfig {
                process_id: process_config.id.clone(),
                can_attach: matches!(
                    process_config.process_type,
                    crate::config::ProcessManagementType::Unmanaged
                ),
                can_terminate: !matches!(
                    process_config.process_type,
                    crate::config::ProcessManagementType::Unmanaged
                ),
                can_restart: !matches!(
                    process_config.process_type,
                    crate::config::ProcessManagementType::Unmanaged
                ),
                graceful_timeout,
                process_profile_type: process_config.profile_type.clone(),
                log_collection_service: self.log_collection_service.clone(),
                log_config: None,
            };

            let process_control = Box::new(ProcessControlImpl::new(
                process_config.clone(),
                control_config,
            ));

            // Register process for log collection if enabled
            if let Some(ref log_service) = self.log_collection_service {
                self.register_process_for_logging(&process_config.id, process_config, log_service);
            }

            let managed_process = ManagedProcessInstance {
                config: process_config.clone(),
                control: Some(process_control),
                in_flight: None,
                pending_ops: VecDeque::new(),
                restart_count: 0,
                last_restart_time: None,
                last_known_pid: None,
                last_known_state: ProcessState::Stopped,
            };

            self.processes
                .insert(process_config.id.clone(), managed_process);
            info!("Initialized process: {}", process_config.id);
        }

        Ok(())
    }

    /// Attempt to reattach to existing processes from previous manager run.
    ///
    /// # Init-Path Await Allowance
    ///
    /// This method contains `.await` calls for file I/O (reading/deleting PID files).
    /// This is explicitly allowed because it runs during initialization in `ProcessManager::new()`,
    /// **before** the actor event loop (`run()`) starts. The actor is not yet processing
    /// commands, so there is no risk of blocking the event loop or starving other work.
    ///
    /// **Invariant**: The actor's `run()` method must never `.await` on file I/O or other
    /// potentially slow operations. All heavy work during normal operation must be delegated
    /// to the `OpRunner` which executes jobs in separate worker tasks.
    pub(super) async fn attempt_reattachment(&mut self) -> Result<()> {
        info!("Scanning for existing processes to reattach");

        let mut reattached_count = 0;
        let mut cleaned_count = 0;

        let process_ids: Vec<String> = self.processes.keys().cloned().collect();

        for process_id in process_ids {
            match self.pid_file_manager.read_pid_file(&process_id).await {
                Ok(pid) => match hsu_process::process_exists(pid) {
                    Ok(true) => {
                        info!("Found existing process: {} (PID: {})", process_id, pid);
                        reattached_count += 1;
                    }
                    Ok(false) => {
                        info!(
                            "Process {} (PID: {}) no longer exists, cleaning up",
                            process_id, pid
                        );
                        if let Err(e) = self.pid_file_manager.delete_pid_file(&process_id).await {
                            warn!("Failed to delete stale PID file for {}: {}", process_id, e);
                        } else {
                            cleaned_count += 1;
                        }
                    }
                    Err(e) => {
                        warn!("Failed to check if process {} exists: {}", process_id, e);
                    }
                },
                Err(_) => {
                    debug!("No PID file found for process: {}", process_id);
                }
            }
        }

        info!(
            "Reattachment scan complete: {} reattached, {} cleaned up",
            reattached_count, cleaned_count
        );
        Ok(())
    }

    /// Create and configure log collection service
    pub(super) fn create_log_collection_service(
        log_config: &crate::config::LogCollectionConfig,
        pid_file_manager: &ProcessFileManager,
    ) -> Result<Arc<LogCollectionService>> {
        info!("Creating log collection service");

        let mut service_config = hsu_log_collection::SystemLogConfig::default();

        if let Some(ref global_agg) = log_config.global_aggregation {
            if global_agg.enabled {
                info!(
                    "Global aggregation enabled with {} targets",
                    global_agg.targets.len()
                );

                for target in &global_agg.targets {
                    match target.target_type.as_str() {
                        "file" => {
                            if let Some(ref path) = target.path {
                                let resolved_path = pid_file_manager.generate_log_file_path(path);
                                info!(
                                    "Global aggregation file target: {}",
                                    resolved_path.display()
                                );

                                if let Some(parent) = resolved_path.parent() {
                                    std::fs::create_dir_all(parent).map_err(|e| {
                                        ProcessError::spawn_failed(
                                            "process-manager",
                                            format!("Failed to create log directory: {}", e),
                                        )
                                    })?;
                                }

                                service_config.output_file = Some(resolved_path);
                            }
                        }
                        "process_manager_stdout" => {
                            info!(
                                "Global aggregation stdout target (forwarding to PROCMAN stdout)"
                            );
                        }
                        _ => {
                            warn!("Unknown log target type: {}", target.target_type);
                        }
                    }
                }
            }
        }

        let service = Arc::new(LogCollectionService::new(service_config));
        info!("Log collection service created successfully");

        Ok(service)
    }

    /// Register a process with the log collection service
    fn register_process_for_logging(
        &self,
        process_id: &str,
        process_config: &ProcessConfig,
        log_service: &Arc<LogCollectionService>,
    ) {
        let log_config = self.get_process_log_config(process_config);

        if !log_config.enabled {
            debug!("Log collection disabled for process: {}", process_id);
            return;
        }

        info!("Registering process {} for log collection", process_id);

        let process_log_config = hsu_log_collection::ProcessLogConfig {
            enabled: log_config.enabled,
            capture_stdout: log_config.capture_stdout,
            capture_stderr: log_config.capture_stderr,
        };

        if let Err(e) = log_service.register_process(process_id.to_string(), process_log_config) {
            warn!(
                "Failed to register process {} for logging: {}",
                process_id, e
            );
            return;
        }

        debug!(
            "Process {} successfully registered for log collection",
            process_id
        );
    }

    /// Get the effective log collection configuration for a process
    fn get_process_log_config(
        &self,
        process_config: &ProcessConfig,
    ) -> crate::config::ProcessLogCollectionConfig {
        let process_override = match &process_config.process_type {
            crate::config::ProcessManagementType::StandardManaged => process_config
                .management
                .standard_managed
                .as_ref()
                .and_then(|sm| sm.control.log_collection.as_ref()),
            crate::config::ProcessManagementType::IntegratedManaged => process_config
                .management
                .integrated_managed
                .as_ref()
                .and_then(|im| im.control.log_collection.as_ref()),
            _ => None,
        };

        if let Some(override_config) = process_override {
            return override_config.clone();
        }

        if let Some(ref log_config) = self.config.log_collection {
            if let Some(ref default_config) = log_config.default {
                return crate::config::ProcessLogCollectionConfig {
                    enabled: default_config.enabled,
                    capture_stdout: default_config.capture_stdout,
                    capture_stderr: default_config.capture_stderr,
                    processing: None,
                    outputs: default_config.outputs.clone(),
                };
            }
        }

        crate::config::ProcessLogCollectionConfig {
            enabled: true,
            capture_stdout: true,
            capture_stderr: true,
            processing: None,
            outputs: None,
        }
    }
}
