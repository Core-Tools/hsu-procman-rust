//! Simplified Process Manager - Actor-based orchestration using ProcessControl trait
//!
//! This module focuses on high-level orchestration with an actor-style design:
//! - Single event loop owns all internal state
//! - Message enum represents commands to the manager
//! - Handle struct provides public API by sending commands over a channel
//! - No Arc<RwLock<HashMap<...>>> or Arc<Mutex<ProcessManagerState>>
//!
//! ## Actor Lifecycle
//!
//! The actor continues running after `Shutdown` to allow state queries.
//! It terminates only when all `ProcessManager` handles are dropped (channel closed).
//!
//! After shutdown:
//! - `get_manager_state()` returns `Stopped`
//! - `start()` / `start_process()` / `restart_process()` return errors
//! - `stop_process()` and query methods still work
//!
//! ## Heartbeat Policy
//!
//! The actor runs a periodic heartbeat (every 2 seconds) that processes pending
//! automatic restarts for all processes. The actor uses a *biased* `tokio::select!`
//! ordering to guarantee completion latency and keep commands responsive.
//!
//! ### Biased Select Trade-off
//!
//! The `biased` keyword in `select!` gives strict priority in declaration order:
//! `OpCompleted` messages first (to clear in-flight ops and resolve oneshots/batches),
//! then commands, then heartbeat ticks. Under sustained load, heartbeats may be
//! delayed or skipped entirely.
//!
//! **Why this is acceptable:**
//! 1. Heartbeats are fire-and-forget (no callers waiting on them)
//! 2. `MissedTickBehavior::Skip` prevents heartbeat backlog accumulation
//! 3. Delayed automatic restarts during heavy load is acceptable - users can
//!    explicitly restart if needed
//! 4. Command responsiveness is critical for interactive use and API latency
//! 5. This does not cause correctness issues, only delays background work
//!
//! **Alternative considered:** Processing N commands then forcing a heartbeat tick
//! adds complexity without significant benefit. The current design is simpler and
//! provides better worst-case latency for user-facing commands.
//!
//! ## Operation Runner (Centralized Heavy Work)
//!
//! Heavy operations (start/stop/restart/heartbeat) are executed by an OpRunner
//! which manages a pool of worker tasks. This keeps the actor responsive:
//! 1. Actor validates state and takes process control handle
//! 2. Submits Job to OpRunner which executes it in a worker task
//! 3. Worker sends OpCompleted back to actor with result and control handle
//! 4. Actor restores control handle and updates state
//!
//! ## Per-Process Operation Queueing
//!
//! When an operation is requested on a process that is already busy:
//! - The operation is queued (FIFO) instead of being rejected
//! - When the current operation completes, the next queued operation starts
//! - This ensures sequential per-process execution while maintaining cross-process concurrency
//!
//! All process control logic is delegated to ProcessControl implementations.
//!
//! ## Backpressure Strategy
//!
//! The system uses layered queues with bounded capacities to prevent unbounded memory growth:
//!
//! | Layer | Capacity | Saturation Behavior |
//! |-------|----------|---------------------|
//! | Command channel | 32 | Caller's `send()` awaits until space available |
//! | Job channel (OpRunner) | 64 | `try_send()` fails → `QueueFull` error returned |
//! | Per-process pending ops | 32 | `QueueFull` error returned to caller |
//! | Global pending ops | 1024 | `QueueFull` error returned to caller |
//!
//! **Design rationale:**
//! - Command channel blocking provides natural backpressure to callers
//! - Per-process limits prevent a single runaway process from consuming all resources
//! - Global limit protects against system-wide overload with many processes
//! - `QueueFull` errors are explicit failures, not silent drops
//!
//! **Idempotency:** `StartAll` is idempotent - calling it when already `Running` or
//! `Starting` returns `Ok(())` immediately without creating a new batch. This prevents
//! duplicate work under concurrent calls.

// Internal modules (not exposed)
mod actor;
mod commands;
mod handle;
mod ops;

// Types module (mixed visibility)
mod types;

#[cfg(test)]
mod tests;

// Re-export public types only
pub use handle::ProcessManager;
pub use types::{ProcessDiagnostics, ProcessInfo, ProcessManagerState, State};

// Private imports for this module (used by ProcessManager::new)
use actor::ProcessManagerActor;
use commands::ManagerCommand;
use hsu_common::ProcessError;
use hsu_process_file::{ProcessFileManager, ServiceContext};
use tokio::sync::mpsc;
use tracing::info;
use ops::OpCompleted;
use types::Result;

impl ProcessManager {
    /// Create a new process manager with the given configuration
    pub async fn new(config: crate::config::ProcessManagerConfig) -> Result<Self> {
        info!(
            "Creating process manager with {} processes",
            config.managed_processes.len()
        );

        // Create ProcessFileManager with optional base_directory override
        let pid_file_manager = if let Some(ref base_dir) = config.process_manager.base_directory {
            info!(
                "Using base directory override for PID/log files: {}",
                base_dir
            );
            ProcessFileManager::with_base_directory(
                base_dir,
                ServiceContext::Session,
                "hsu-procman",
            )
        } else {
            ProcessFileManager::with_defaults()
        };

        // Create log collection service if enabled
        let log_collection_service = if let Some(ref log_config) = config.log_collection {
            if log_config.enabled {
                info!("Log collection enabled, initializing service...");
                let service = ProcessManagerActor::create_log_collection_service(
                    log_config,
                    &pid_file_manager,
                )?;

                // Start the service
                service.start().await.map_err(|e| {
                    ProcessError::spawn_failed(
                        "process-manager",
                        format!("Failed to start log collection service: {}", e),
                    )
                })?;
                info!("Log collection service started successfully");

                Some(service)
            } else {
                info!("Log collection disabled in configuration");
                None
            }
        } else {
            None
        };

        // Create the command channel
        let (cmd_tx, cmd_rx) = mpsc::channel::<ManagerCommand>(32);

        // Create the completion channel (OpRunner -> actor)
        let (completed_tx, completed_rx) = mpsc::channel::<OpCompleted>(256);

        // Create the actor with initial state
        let mut actor = ProcessManagerActor::new(
            config,
            pid_file_manager,
            log_collection_service,
            completed_tx,
        );

        // Initialize processes from configuration
        actor.initialize_processes().await?;

        // Attempt to reattach to existing processes
        actor.attempt_reattachment().await?;

        // Spawn the actor's event loop
        tokio::spawn(actor.run(cmd_rx, completed_rx));

        Ok(ProcessManager { cmd_tx })
    }
}
