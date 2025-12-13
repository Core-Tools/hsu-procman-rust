//! Operation runner for centralized heavy work execution.
//!
//! This module provides `OpRunner` which manages a pool of worker tasks
//! that execute process control operations off the actor thread. Workers
//! handle panics gracefully and always report completion back to the actor.

use super::types::{OpKind, Result};
use futures::future::FutureExt;
use hsu_common::ProcessError;
use hsu_managed_process::{ProcessControl, ProcessDiagnostics};
use hsu_process_file::ProcessFileManager;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, warn};

/// Default timeout for operations (30 seconds)
const DEFAULT_OP_TIMEOUT: Duration = Duration::from_secs(30);

/// Capacity of the OpRunner job submission channel.
///
/// This constant is used both for the channel size and for user-facing `QueueFull` errors
/// so they remain consistent.
pub(super) const JOB_QUEUE_CAPACITY: usize = 64;

/// A job to be executed by the operation runner
pub(super) struct Job {
    /// Process ID this operation is for
    pub process_id: String,
    /// The kind of operation to perform
    pub op: OpKind,
    /// The process control handle (taken from ManagedProcessInstance)
    pub control: Box<dyn ProcessControl>,
    /// Batch ID for coordinated operations (StartAll, ShutdownAll)
    pub batch_id: Option<u64>,
}

/// Result of an operation execution (sent back to actor)
pub(super) struct OpCompleted {
    /// Process ID
    pub process_id: String,
    /// The operation that was performed
    pub op: OpKind,
    /// The process control handle (to be returned to instance)
    /// None if the task panicked and the control handle was lost
    pub control: Option<Box<dyn ProcessControl>>,
    /// Result of the operation
    pub result: Result<()>,
    /// Batch ID if this was part of a coordinated operation
    pub batch_id: Option<u64>,
    /// Number of restarts processed (for HeartbeatPoll)
    pub restarts_processed: usize,
    /// Optional diagnostics payload (for QueryDiagnostics)
    pub diagnostics: Option<ProcessDiagnostics>,
}

/// Operation runner that manages worker tasks for heavy operations.
///
/// The runner spawns a fixed number of worker tasks that pull jobs from
/// a channel, execute them with timeout and panic handling, and send
/// completion back to the actor.
pub(super) struct OpRunner {
    /// Channel to submit jobs (public for actor to clone)
    pub job_tx: mpsc::Sender<Job>,
}

impl OpRunner {
    /// Create a new operation runner with the specified concurrency.
    ///
    /// # Arguments
    /// * `concurrency` - Number of worker tasks to spawn
    /// * `completed_tx` - Channel to send OpCompleted messages back to actor
    /// * `pid_file_manager` - Shared PID file manager
    pub fn new(
        concurrency: usize,
        completed_tx: mpsc::Sender<OpCompleted>,
        pid_file_manager: Arc<ProcessFileManager>,
    ) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>(JOB_QUEUE_CAPACITY);
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let dispatcher_completed_tx = completed_tx.clone();
        let dispatcher_pid = Arc::clone(&pid_file_manager);
        let max_in_flight = concurrency;

        tokio::spawn(async move {
            Self::dispatcher_loop(
                job_rx,
                semaphore,
                dispatcher_completed_tx,
                dispatcher_pid,
                max_in_flight,
            )
            .await;
        });

        debug!(
            "OpRunner started with {} workers (semaphore limited)",
            concurrency
        );

        OpRunner { job_tx }
    }

    /// Dispatcher loop: receives jobs, enforces concurrency via semaphore, tracks tasks in JoinSet.
    async fn dispatcher_loop(
        mut job_rx: mpsc::Receiver<Job>,
        semaphore: Arc<Semaphore>,
        completed_tx: mpsc::Sender<OpCompleted>,
        pid_file_manager: Arc<ProcessFileManager>,
        max_in_flight: usize,
    ) {
        let mut join_set: JoinSet<()> = JoinSet::new();

        loop {
            tokio::select! {
                maybe_job = job_rx.recv() => {
                    match maybe_job {
                        Some(job) => {
                            let permit = match semaphore.clone().acquire_owned().await {
                                Ok(p) => p,
                                Err(e) => {
                                    error!("OpRunner semaphore closed unexpectedly: {}", e);
                                    // This is fatal for the specific job: it cannot be executed.
                                    // Do NOT drop the job silently; synthesize an OpCompleted so
                                    // the actor can clear in-flight state, complete batches, and
                                    // return the control handle.
                                    let process_id = job.process_id.clone();
                                    let op = job.op.clone();
                                    let batch_id = job.batch_id;
                                    let completed = OpCompleted {
                                        process_id: process_id.clone(),
                                        op,
                                        control: Some(job.control),
                                        result: Err(ProcessError::spawn_failed(
                                            process_id,
                                            format!("operation runner semaphore closed: {}", e),
                                        )),
                                        batch_id,
                                        restarts_processed: 0,
                                        diagnostics: None,
                                    };
                                    if completed_tx.send(completed).await.is_err() {
                                        error!(
                                            "Failed to send OpCompleted after semaphore failure: completion channel closed"
                                        );
                                    }
                                    continue;
                                }
                            };

                            let completed_tx = completed_tx.clone();
                            let pid_file_manager = Arc::clone(&pid_file_manager);

                            join_set.spawn(async move {
                                // Permit dropped when scope ends
                                let _permit = permit;

                                // Execute job with panic handling - always produces OpCompleted
                                let completed = Self::execute_job_with_panic_recovery(job, &pid_file_manager).await;

                                if completed_tx.send(completed).await.is_err() {
                                    error!("Failed to send OpCompleted: completion channel closed");
                                }
                            });

                            // Bound JoinSet growth: finished tasks remain in the JoinSet until joined.
                            // Under high throughput, permits may be released faster than we poll
                            // `join_next()`, causing unbounded memory growth. Drain completed tasks
                            // opportunistically back down to the concurrency bound.
                            while join_set.len() > max_in_flight {
                                let _ = join_set.join_next().await;
                            }
                        }
                        None => {
                            debug!("OpRunner dispatcher: channel closed, draining tasks");
                            break;
                        }
                    }
                }
                // Only poll join_set when it has tasks, otherwise it returns Ready(None) immediately
                // causing a busy-wait spin loop
                join_res = join_set.join_next(), if !join_set.is_empty() => {
                    if let Some(Err(e)) = join_res {
                        // This should not happen since we handle panics inside the task
                        // But log it just in case
                        if e.is_panic() {
                            error!("OpRunner worker task panicked unexpectedly: {}", e);
                        } else {
                            warn!("OpRunner worker task cancelled: {}", e);
                        }
                    }
                }
            }
        }

        // Drain remaining tasks
        while let Some(res) = join_set.join_next().await {
            if let Err(e) = res {
                if e.is_panic() {
                    error!("OpRunner worker task panicked during drain: {}", e);
                } else {
                    warn!("OpRunner worker task cancelled during drain: {}", e);
                }
            }
        }
    }

    /// Execute a job with panic recovery - always returns OpCompleted
    async fn execute_job_with_panic_recovery(
        job: Job,
        pid_file_manager: &Arc<ProcessFileManager>,
    ) -> OpCompleted {
        let process_id = job.process_id.clone();
        let op = job.op.clone();
        let batch_id = job.batch_id;

        // Wrap execution in catch_unwind to handle panics
        let result: std::result::Result<OpCompleted, Box<dyn std::any::Any + Send>> =
            AssertUnwindSafe(Self::execute_job_safe(job, pid_file_manager))
                .catch_unwind()
                .await;

        match result {
            Ok(completed) => completed,
            Err(panic_info) => {
                // Task panicked - synthesize OpCompleted with control = None
                let panic_msg: String = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    (*s).to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };

                error!(
                    "Job panicked for process {} during {}: {}",
                    process_id,
                    op.name(),
                    panic_msg
                );

                OpCompleted {
                    process_id: process_id.clone(),
                    op,
                    control: None, // Control handle is lost due to panic
                    result: Err(ProcessError::task_panic(process_id, panic_msg)),
                    batch_id,
                    restarts_processed: 0,
                    diagnostics: None,
                }
            }
        }
    }

    /// Execute a job with timeout.
    async fn execute_job_safe(
        mut job: Job,
        pid_file_manager: &Arc<ProcessFileManager>,
    ) -> OpCompleted {
        let process_id = job.process_id.clone();
        let op = job.op.clone();
        let batch_id = job.batch_id;

        // Execute with timeout
        let execution_result = timeout(
            DEFAULT_OP_TIMEOUT,
            Self::execute_job_inner(&mut job, pid_file_manager),
        )
        .await;

        let (result, restarts_processed, diagnostics) = match execution_result {
            Ok((res, restarts, diagnostics)) => (res, restarts, diagnostics),
            Err(_) => {
                warn!("Operation {:?} timed out for process {}", op, process_id);
                (
                    Err(ProcessError::Timeout {
                        id: process_id.clone(),
                        operation: op.name().to_string(),
                    }),
                    0,
                    None,
                )
            }
        };

        OpCompleted {
            process_id,
            op,
            control: Some(job.control),
            result,
            batch_id,
            restarts_processed,
            diagnostics,
        }
    }

    /// Inner execution of a job (called within timeout).
    async fn execute_job_inner(
        job: &mut Job,
        pid_file_manager: &Arc<ProcessFileManager>,
    ) -> (Result<()>, usize, Option<ProcessDiagnostics>) {
        let mut restarts_processed = 0;
        let mut diagnostics: Option<ProcessDiagnostics> = None;

        let result = match &job.op {
            OpKind::Start => {
                info!("Starting process: {}", job.process_id);
                match job.control.start().await {
                    Ok(()) => {
                        // Write PID file for managed processes
                        if let Some(pid) = job.control.get_pid() {
                            if let Err(e) =
                                pid_file_manager.write_pid_file(&job.process_id, pid).await
                            {
                                warn!("Failed to write PID file for {}: {}", job.process_id, e);
                            }
                        }
                        info!("Process started successfully: {}", job.process_id);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to start process {}: {}", job.process_id, e);
                        Err(e)
                    }
                }
            }
            OpKind::Stop => {
                info!("Stopping process: {}", job.process_id);
                match job.control.stop().await {
                    Ok(()) => {
                        // Delete PID file
                        if let Err(e) = pid_file_manager.delete_pid_file(&job.process_id).await {
                            warn!("Failed to delete PID file for {}: {}", job.process_id, e);
                        }
                        info!("Process stopped successfully: {}", job.process_id);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to stop process {}: {}", job.process_id, e);
                        Err(e)
                    }
                }
            }
            OpKind::Restart { force } => {
                let force = *force;
                info!("Restarting process: {} (force: {})", job.process_id, force);
                match job.control.restart(force).await {
                    Ok(()) => {
                        // Update PID file
                        if let Some(pid) = job.control.get_pid() {
                            if let Err(e) =
                                pid_file_manager.write_pid_file(&job.process_id, pid).await
                            {
                                warn!("Failed to update PID file for {}: {}", job.process_id, e);
                            }
                        }
                        info!("Process restarted successfully: {}", job.process_id);
                        Ok(())
                    }
                    Err(e) => {
                        error!("Failed to restart process {}: {}", job.process_id, e);
                        Err(e)
                    }
                }
            }
            OpKind::HeartbeatPoll => {
                // Process pending restarts via trait method
                match job.control.process_pending_restarts().await {
                    Ok(count) => {
                        if count > 0 {
                            info!(
                                "Processed {} automatic restart(s) for {}",
                                count, job.process_id
                            );
                        }
                        restarts_processed = count;
                        Ok(())
                    }
                    Err(e) => {
                        error!(
                            "Error processing restart requests for {}: {}",
                            job.process_id, e
                        );
                        Err(e)
                    }
                }
            }
            OpKind::QueryDiagnostics => {
                debug!("Querying diagnostics for {}", job.process_id);
                diagnostics = Some(job.control.get_diagnostics());
                Ok(())
            }
            #[cfg(test)]
            OpKind::TestSleep(duration) => {
                tokio::time::sleep(*duration).await;
                Ok(())
            }
        };

        (result, restarts_processed, diagnostics)
    }

    /// Test-only helper: run the dispatcher loop with a closed semaphore so that
    /// permit acquisition fails deterministically. Ensures the "semaphore closed"
    /// path still emits an `OpCompleted` for the job.
    #[cfg(test)]
    pub(super) async fn test_run_job_with_closed_semaphore(
        job: Job,
        completed_tx: mpsc::Sender<OpCompleted>,
    ) {
        let (job_tx, job_rx) = mpsc::channel::<Job>(1);
        let semaphore = Arc::new(Semaphore::new(1));
        semaphore.close();

        // This PID file manager is unused in the semaphore-failure path, but required by the
        // dispatcher loop signature.
        let pid_file_manager = Arc::new(ProcessFileManager::with_defaults());

        // Enqueue exactly one job and then close the job channel so the dispatcher exits.
        let _ = job_tx.send(job).await;
        drop(job_tx);

        Self::dispatcher_loop(job_rx, semaphore, completed_tx, pid_file_manager, 1).await;
    }
}
