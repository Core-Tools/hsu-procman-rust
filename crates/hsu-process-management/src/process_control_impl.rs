//! ProcessControlImpl - Implementation of ProcessControl trait
//!
//! This module contains the core implementation of process lifecycle control,
//! encapsulating complex logic for:
//! - Process spawning and termination
//! - Health check integration
//! - Resource monitoring
//! - Restart policies and circuit breaker
//! - State machine management
//!
//! This implementation mirrors Go's `pkg/managedprocess/processcontrolimpl/process_control_impl.go`

use hsu_managed_process::{
    ProcessControl, ProcessControlConfig, ProcessDiagnostics, ProcessError,
    ErrorCategory,
};
use async_trait::async_trait;
use hsu_common::{ProcessError as CommonProcessError, ProcessResult};
use hsu_process_state::{ProcessState, ProcessStateMachine};
use hsu_process;
use crate::config::ProcessConfig;
use crate::lifecycle::ProcessLifecycleManager;
use hsu_monitoring::{HealthStatus, HealthMonitor, HealthCheckConfig, HealthCheckType};

/// Restart request from callbacks (health checks, resource violations, etc.)
#[derive(Debug, Clone)]
pub struct RestartRequest {
    pub reason: String,
    pub trigger_type: RestartTriggerType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Type of restart trigger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartTriggerType {
    /// Health check failure
    HealthFailure,
    /// Resource violation
    ResourceViolation,
    /// Manual/external request
    Manual,
}
use hsu_resource_limits::{
    ResourceMonitor, ResourceUsage,
    check_resource_limits_with_policy, ResourcePolicy, ResourceViolation,
};
use hsu_log_collection::{LogCollectionService, StreamType};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::{RwLock, Mutex};
use tokio::time::{timeout, interval, Duration};
use tracing::{debug, error, info, warn};

const FORCE_KILL_TIMEOUT: Duration = Duration::from_secs(3);
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Process control implementation with full lifecycle management
pub struct ProcessControlImpl {
    /// Process configuration
    config: ProcessConfig,
    
    /// Control configuration
    control_config: ProcessControlConfig,
    
    /// State machine for process lifecycle
    state_machine: ProcessStateMachine,
    
    /// Lifecycle manager for restart policies (wrapped for shared mutable access)
    lifecycle_manager: Arc<RwLock<ProcessLifecycleManager>>,
    
    /// Child process handle (if spawned)
    child: Option<Child>,
    
    /// Current process ID (if running)
    pid: Option<u32>,
    
    /// Process start time
    start_time: Option<chrono::DateTime<chrono::Utc>>,
    
    /// Restart counter
    restart_count: u32,
    
    /// Last restart time
    last_restart_time: Option<chrono::DateTime<chrono::Utc>>,
    
    /// Health status tracking
    health_status: Arc<RwLock<HealthStatus>>,
    
    /// Health monitor (replaces health_check_task)
    health_monitor: Option<HealthMonitor>,
    
    /// Health monitor background task
    health_monitor_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Resource usage tracking
    resource_usage: Arc<RwLock<ResourceUsage>>,
    
    /// Resource monitor
    resource_monitor: Arc<ResourceMonitor>,
    
    /// Resource monitoring background task
    resource_monitor_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Channel for sending resource violation events
    violation_tx: Option<tokio::sync::mpsc::UnboundedSender<(ResourcePolicy, ResourceViolation)>>,
    
    /// Resource violation handler task
    violation_handler_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Process exit monitor task (prevents zombie processes on Unix/Linux)
    exit_monitor_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Restart handler task (processes restart requests from callbacks)
    restart_handler_task: Option<tokio::task::JoinHandle<()>>,
    
    /// Last exit status (for diagnostics)
    /// Note: Currently tracked by exit_monitor_task, may be used for diagnostics in future
    #[allow(dead_code)]
    last_exit_status: Option<std::process::ExitStatus>,
    
    /// Log collection service (optional)
    log_collection_service: Option<Arc<LogCollectionService>>,
    
    /// Restart request channel (for health checks and resource violations to trigger restarts)
    restart_tx: tokio::sync::mpsc::UnboundedSender<RestartRequest>,
    restart_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<RestartRequest>>>,
    
    /// Last error (for diagnostics)
    last_error: Option<ProcessError>,
}

impl ProcessControlImpl {
    /// Create a new process control implementation
    pub fn new(
        config: ProcessConfig,
        control_config: ProcessControlConfig,
    ) -> Self {
        let process_id = config.id.clone();
        let state_machine = ProcessStateMachine::new(&process_id);
        
        // Extract restart policy based on process type
        let restart_policy = match &config.process_type {
            crate::config::ProcessManagementType::StandardManaged => {
                config.management.standard_managed.as_ref()
                    .and_then(|sm| sm.control.restart_policy.clone())
            }
            crate::config::ProcessManagementType::IntegratedManaged => {
                config.management.integrated_managed.as_ref()
                    .and_then(|im| im.control.restart_policy.clone())
            }
            crate::config::ProcessManagementType::Unmanaged => None,
        };
        
        // Convert restart policy to old RestartPolicyConfig format if needed
        // For now, create a default RestartPolicyConfig
        let restart_policy_config = restart_policy.map(|policy_str| {
            crate::config::RestartPolicyConfig {
                strategy: match policy_str.as_str() {
                    "never" => crate::config::RestartStrategy::Never,
                    "always" => crate::config::RestartStrategy::Always,
                    _ => crate::config::RestartStrategy::OnFailure,
                },
                max_attempts: 3,
                restart_delay: Duration::from_secs(5),
                backoff_multiplier: 1.5,
            }
        });
        
        let lifecycle_manager = Arc::new(RwLock::new(ProcessLifecycleManager::new(
            process_id.clone(),
            restart_policy_config,
        )));
        
        // Create restart request channel
        let (restart_tx, restart_rx) = tokio::sync::mpsc::unbounded_channel();
        
        // Extract log collection service before moving control_config
        let log_collection_service = control_config.log_collection_service.clone();
        
        Self {
            config,
            control_config,
            state_machine,
            lifecycle_manager,
            child: None,
            pid: None,
            start_time: None,
            restart_count: 0,
            last_restart_time: None,
            health_status: Arc::new(RwLock::new(HealthStatus::new())),
            health_monitor: None,
            health_monitor_task: None,
            resource_usage: Arc::new(RwLock::new(ResourceUsage {
                cpu_percent: None,
                memory_mb: None,
                file_descriptors: None,
                network_connections: None,
            })),
            resource_monitor: Arc::new(ResourceMonitor::new()),
            resource_monitor_task: None,
            violation_tx: None,
            violation_handler_task: None,
            exit_monitor_task: None,
            restart_handler_task: None,
            last_exit_status: None,
            log_collection_service,
            restart_tx,
            restart_rx: Arc::new(Mutex::new(restart_rx)),
            last_error: None,
        }
    }
    
    /// Spawn a new child process
    async fn spawn_process(&mut self) -> ProcessResult<()> {
        info!("Spawning process: {}", self.config.id);
        
        // Extract execution config based on process type
        let execution = match &self.config.process_type {
            crate::config::ProcessManagementType::StandardManaged => {
                self.config.management.standard_managed.as_ref()
                    .map(|sm| &sm.control.execution)
                    .ok_or_else(|| CommonProcessError::Configuration {
                        id: self.config.id.clone(),
                        reason: "Missing standard_managed configuration".to_string(),
                    })?
            }
            crate::config::ProcessManagementType::IntegratedManaged => {
                self.config.management.integrated_managed.as_ref()
                    .map(|im| &im.control.execution)
                    .ok_or_else(|| CommonProcessError::Configuration {
                        id: self.config.id.clone(),
                        reason: "Missing integrated_managed configuration".to_string(),
                    })?
            }
            crate::config::ProcessManagementType::Unmanaged => {
                return Err(CommonProcessError::Configuration {
                    id: self.config.id.clone(),
                    reason: "Cannot spawn unmanaged process".to_string(),
                });
            }
        };
        
        // Build command
        let mut cmd = Command::new(&execution.executable_path);
        cmd.args(&execution.args);
        
        if let Some(ref wd) = execution.working_directory {
            cmd.current_dir(wd);
        }
        
        for (key, value) in &execution.environment {
            cmd.env(key, value);
        }
        
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());
        
        // Windows-specific: Isolate child processes in a separate process group
        // This mirrors Go's CREATE_NEW_PROCESS_GROUP behavior and prevents:
        // 1. Console signals from propagating to/from parent processes
        // 2. Spurious Ctrl+C events reaching the test runner
        // 3. Signal interference during process restart cycles
        #[cfg(windows)]
        {
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
        
        // Spawn the process (spawn is synchronous, returns Result)
        match cmd.spawn() {
            Ok(mut child) => {
                let pid = child.id().unwrap_or(0);
                
                // Capture stdout/stderr for log collection before moving child
                if let Some(ref log_service) = self.log_collection_service {
                    self.spawn_log_collection_tasks(&mut child, log_service);
                }
                
                self.child = Some(child);
                self.pid = Some(pid);
                self.start_time = Some(chrono::Utc::now());
                
                info!("Process spawned successfully: {} (PID: {})", self.config.id, pid);
                Ok(())
            }
            Err(e) => {
                let err = ProcessError::new(
                    ErrorCategory::ExecutableNotFound,
                    format!("Failed to spawn process: {}", e),
                    false,
                );
                self.last_error = Some(err.clone());
                
                Err(CommonProcessError::SpawnFailed {
                    id: self.config.id.clone(),
                    reason: err.details,
                })
            }
        }
    }
    
    /// Spawn log collection tasks for stdout/stderr
    fn spawn_log_collection_tasks(&self, child: &mut Child, log_service: &Arc<LogCollectionService>) {
        let process_id = self.config.id.clone();
        
        // Take stdout (if available)
        if let Some(stdout) = child.stdout.take() {
            let log_service = Arc::clone(log_service);
            let process_id_clone = process_id.clone();
            
            tokio::spawn(async move {
                if let Err(e) = log_service.collect_from_stream(
                    &process_id_clone,
                    stdout,
                    StreamType::Stdout,
                ).await {
                    warn!("Failed to collect stdout for {}: {}", process_id_clone, e);
                }
            });
            
            debug!("Stdout log collection started for {}", process_id);
        }
        
        // Take stderr (if available)
        if let Some(stderr) = child.stderr.take() {
            let log_service = Arc::clone(log_service);
            let process_id_clone = process_id.clone();
            
            tokio::spawn(async move {
                if let Err(e) = log_service.collect_from_stream(
                    &process_id_clone,
                    stderr,
                    StreamType::Stderr,
                ).await {
                    warn!("Failed to collect stderr for {}: {}", process_id_clone, e);
                }
            });
            
            debug!("Stderr log collection started for {}", process_id);
        }
        
        info!("Log collection tasks spawned for {}", process_id);
    }
    
    /// Terminate the running process
    async fn terminate_process(&mut self) -> ProcessResult<()> {
        info!("Terminating process: {}", self.config.id);
        
        let pid = match self.pid {
            Some(pid) => pid,
            None => {
                debug!("No process PID to terminate for {}", self.config.id);
                return Ok(());
            }
        };
        
        // Try graceful termination via signal
        // Note: Child has been moved to exit_monitor_task, so we use PID-based termination
        info!("Sending termination signal to PID {}", pid);
        
        #[cfg(unix)]
        {
            // On Unix, send SIGTERM
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            
            if let Err(e) = kill(Pid::from_raw(pid as i32), Signal::SIGTERM) {
                warn!("Failed to send SIGTERM to PID {}: {}", pid, e);
            }
        }
        
        #[cfg(windows)]
        {
            // On Windows, use GenerateConsoleCtrlEvent to send Ctrl+Break signal
            // This works correctly with CREATE_NEW_PROCESS_GROUP and allows graceful shutdown
            // 
            // Note: is_dead=false because we're terminating an alive process
            // The AttachConsole hack will NOT be applied here (only for confirmed dead PIDs)
            let timeout = std::time::Duration::from_secs(1);
            match hsu_process::terminate_windows::send_termination_signal(pid, false, timeout) {
                Ok(_) => {
                    info!("Successfully sent Ctrl+Break signal to PID {}", pid);
                }
                Err(e) => {
                    warn!("Failed to send Ctrl+Break to PID {}: {}", pid, e);
                }
            }
        }

        // Wait for process exit (confirmed) with graceful timeout.
        let graceful_timeout = self.control_config.graceful_timeout;
        if self.wait_for_exit_confirmed(pid, graceful_timeout).await? {
            self.pid = None;
            info!("Process terminated gracefully: {}", self.config.id);
            return Ok(());
        }

        warn!(
            "Graceful shutdown timed out for {} (PID: {}), attempting force kill",
            self.config.id, pid
        );

        if !self.control_config.can_terminate {
            return Err(CommonProcessError::timeout(
                self.config.id.clone(),
                "stop (graceful timeout; force kill not supported)",
            ));
        }

        // Force kill.
        if let Err(e) = hsu_process::force_kill(pid) {
            error!("Force kill failed for {} (PID: {}): {}", self.config.id, pid, e);
        }

        // Wait for exit again (short timeout). If still not exited, return error.
        if self.wait_for_exit_confirmed(pid, FORCE_KILL_TIMEOUT).await? {
            self.pid = None;
            info!("Process terminated after force kill: {}", self.config.id);
            return Ok(());
        }

        Err(CommonProcessError::timeout(
            self.config.id.clone(),
            format!(
                "stop (did not exit after graceful timeout {:?} + force-kill timeout {:?})",
                graceful_timeout, FORCE_KILL_TIMEOUT
            ),
        ))
    }

    /// Best-effort exit confirmation with a timeout.
    ///
    /// Returns `Ok(true)` if the process is confirmed exited within the timeout,
    /// `Ok(false)` if the timeout elapsed and the process still appears to exist,
    /// or `Err(_)` if exit status could not be determined.
    async fn wait_for_exit_confirmed(&mut self, pid: u32, timeout_dur: Duration) -> ProcessResult<bool> {
        // Prefer the exit monitor task if present (it owns the Child handle and does child.wait()).
        if let Some(mut task) = self.exit_monitor_task.take() {
            let res = timeout(timeout_dur, &mut task).await;

            match res {
                Ok(join_res) => {
                    // Task finished. If it completed successfully, the child has been waited/reaped,
                    // which is the strongest exit confirmation we can have (PID reuse is possible,
                    // so do NOT rely on process_exists() in this success case).
                    if join_res.is_ok() {
                        return Ok(true);
                    }

                    warn!("Exit monitor task join error for {}: {:?}", self.config.id, join_res);

                    // If the monitor failed/panicked, fall back to PID existence check.
                    match hsu_process::process_exists(pid) {
                        Ok(true) => Ok(false),
                        Ok(false) => Ok(true),
                        Err(e) => Err(CommonProcessError::stop_failed(
                            self.config.id.clone(),
                            format!("Failed to confirm exit after monitor error: {}", e),
                        )),
                    }
                }
                Err(_) => {
                    // Timed out; keep monitoring task for possible later completion.
                    self.exit_monitor_task = Some(task);
                    match hsu_process::process_exists(pid) {
                        Ok(true) => Ok(false),
                        Ok(false) => Ok(true),
                        Err(e) => Err(CommonProcessError::stop_failed(
                            self.config.id.clone(),
                            format!("Failed to check process existence: {}", e),
                        )),
                    }
                }
            }
        } else {
            // Invariant: once we track a PID for a spawned child, `spawn_exit_monitor_task()`
            // must have been called and `exit_monitor_task` must exist. The monitor owns the
            // `Child` handle and provides the strongest exit confirmation (PID reuse is possible).
            //
            // If we ever reach this branch, it indicates an internal lifecycle bug (e.g. partial
            // start, lost monitor task). Prefer failing fast to avoid relying on weak PID polling.
            unreachable!(
                "exit monitor task is missing while confirming exit for process '{}' (pid={}); invariant violation",
                self.config.id,
                pid
            );
        }
    }

    async fn poll_pid_until_exit(pid: u32) -> ProcessResult<()> {
        loop {
            match hsu_process::process_exists(pid) {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(e) => {
                    return Err(CommonProcessError::stop_failed(
                        pid.to_string(),
                        format!("Failed to check process existence: {}", e),
                    ))
                }
            }
            tokio::time::sleep(EXIT_POLL_INTERVAL).await;
        }
    }
    
    /// Spawn exit monitor task to prevent zombie processes
    /// 
    /// This task waits for the child process to exit and logs the exit status.
    /// It does NOT trigger automatic restart - that's handled by health check monitoring.
    /// 
    /// Purpose:
    /// - Prevents zombie processes on Unix/Linux by calling wait()
    /// - Captures exit status for diagnostics
    /// - Provides logging when process exits
    fn spawn_exit_monitor_task(&mut self) {
        // Take ownership of child (we can't share mutable access to Child)
        let mut child = match self.child.take() {
            Some(c) => c,
            None => {
                warn!("No child process to monitor for {}", self.config.id);
                return;
            }
        };
        
        let process_id = self.config.id.clone();
        let pid = child.id().unwrap_or(0);
        
        // Spawn task to wait for process exit
        let task = tokio::spawn(async move {
            info!("Exit monitor started for process {} (PID: {})", process_id, pid);
            
            match child.wait().await {
                Ok(status) => {
                    if status.success() {
                        info!("Process {} (PID: {}) exited successfully with code: {:?}", 
                              process_id, pid, status.code());
                    } else {
                        warn!("Process {} (PID: {}) exited with non-zero status: {:?}", 
                              process_id, pid, status.code());
                    }
                }
                Err(e) => {
                    error!("Failed to wait for process {} (PID: {}): {}", process_id, pid, e);
                }
            }
            
            // CRITICAL: Apply AttachConsole Dead PID Hack after child exits
            // This is necessary for production use where there's no test runner
            // to apply the hack. Even with CREATE_NEW_PROCESS_GROUP isolation,
            // the hack should be applied to prevent console state corruption.
            // Following Go's pattern: apply hack from PROCMAN after child is confirmed dead.
            #[cfg(windows)]
            {
                // Wait a bit to ensure process is fully dead
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Err(e) = hsu_process::terminate_windows::send_termination_signal(
                    pid, 
                    true,  // Process is dead - apply console reset hack
                    std::time::Duration::from_millis(100)
                ) {
                    debug!("Console signal fix for PID {}: {}", pid, e);
                }
            }
            
            info!("Exit monitor completed for process {} (PID: {})", process_id, pid);
        });
        
        self.exit_monitor_task = Some(task);
        debug!("Exit monitor task spawned for {} (PID: {})", self.config.id, pid);
    }
    
    /// Spawn health check background task
    
    /// Spawn violation handler task to process resource violations
    fn spawn_violation_handler_task(&mut self, mut rx: tokio::sync::mpsc::UnboundedReceiver<(ResourcePolicy, ResourceViolation)>) {
        let process_id = self.config.id.clone();
        let restart_tx = self.restart_tx.clone();  // Clone the restart channel sender
        let lifecycle_manager = Arc::clone(&self.lifecycle_manager);
        
        info!("Spawning violation handler task for {}", process_id);
        
        let task = tokio::spawn(async move {
            while let Some((policy, violation)) = rx.recv().await {
                // Process violation based on policy
                Self::handle_resource_violation_with_channel(
                    &process_id,
                    policy,
                    violation,
                    &restart_tx,
                    &lifecycle_manager,
                ).await;
            }
        });
        
        self.violation_handler_task = Some(task);
    }
    
    /// Handle resource violation with policy using restart channel
    async fn handle_resource_violation_with_channel(
        process_id: &str,
        policy: ResourcePolicy,
        violation: ResourceViolation,
        restart_tx: &tokio::sync::mpsc::UnboundedSender<RestartRequest>,
        lifecycle_manager: &Arc<RwLock<ProcessLifecycleManager>>,
    ) {
        warn!(
            "🚨 Resource violation detected for process {}: {} (policy: {:?})",
            process_id, violation.message, policy
        );
        
        match policy {
            ResourcePolicy::None => {
                // No action - just detected
                debug!("No action for resource violation (policy: none)");
            }
            
            ResourcePolicy::Log => {
                // Log violation only
                warn!(
                    "📝 LOG: Resource limit exceeded (policy: log): {} - Type: {:?}, Severity: {:?}, Current: {}, Limit: {}",
                    violation.message, violation.limit_type, violation.severity, violation.current_value, violation.limit_value
                );
            }
            
            ResourcePolicy::Alert => {
                // Send alert/notification (for now, just error log)
                error!(
                    "🔔 ALERT: Resource limit exceeded: {} - Type: {:?}, Severity: {:?}, Current: {}, Limit: {}",
                    violation.message, violation.limit_type, violation.severity, violation.current_value, violation.limit_value
                );
                // Note: Alerting system integration planned for Phase 5 (Enterprise Features)
            }
            
            ResourcePolicy::Throttle => {
                // Future: Suspend/resume process
                warn!(
                    "⏸️ THROTTLE: Resource limit exceeded (not yet implemented): {}",
                    violation.message
                );
                // TODO: Implement process throttling (SIGSTOP/SIGCONT on Unix)
            }
            
            ResourcePolicy::Restart => {
                error!(
                    "🔄 RESTART: Resource limit exceeded, triggering automatic restart (policy: restart): {}",
                    violation.message
                );
                
                // Evaluate restart policy
                let should_restart = lifecycle_manager.try_read()
                    .map(|manager| {
                        let should = manager.should_restart();
                        let is_tripped = manager.is_circuit_breaker_tripped();
                        info!(
                            "🔧 Resource violation restart policy check for {}: should_restart={}, circuit_breaker_tripped={}",
                            process_id, should, is_tripped
                        );
                        should
                    })
                    .unwrap_or(true);
                
                if !should_restart {
                    warn!("⛔ Resource violation restart blocked by policy for {}", process_id);
                    return;
                }
                
                // Send restart request via channel
                info!("✅ Resource violation restart allowed, queueing restart request for {}", process_id);
                let request = RestartRequest {
                    reason: format!("Resource violation: {}", violation.message),
                    trigger_type: RestartTriggerType::ResourceViolation,
                    timestamp: chrono::Utc::now(),
                };
                
                match restart_tx.send(request) {
                    Ok(()) => {
                        info!("📬 Resource violation restart request queued for {}", process_id);
                    }
                    Err(e) => {
                        error!("❌ Failed to queue resource violation restart for {}: {}", process_id, e);
                    }
                }
            }
            
            ResourcePolicy::GracefulShutdown => {
                error!(
                    "🛑 GRACEFUL SHUTDOWN: Resource limit exceeded, process should be gracefully stopped (policy: graceful_shutdown): {}",
                    violation.message
                );
                // TODO: Implement graceful shutdown via channel (similar to restart)
                // This would send a StopRequest instead of RestartRequest
                warn!("⚠️ Graceful shutdown not yet fully implemented - requires stop channel");
            }
            
            ResourcePolicy::ImmediateKill => {
                error!(
                    "💀 IMMEDIATE KILL: Resource limit exceeded, process should be killed immediately (policy: immediate_kill): {}",
                    violation.message
                );
                // TODO: Implement immediate kill via channel (similar to restart)
                // This would send a KillRequest to immediately terminate the process
                warn!("⚠️ Immediate kill not yet fully implemented - requires kill channel");
            }
            
            ResourcePolicy::RestartAdjusted => {
                error!(
                    "🔧 RESTART ADJUSTED: Resource limit exceeded, attempting restart with adjusted limits (policy: restart_adjusted): {}",
                    violation.message
                );
                warn!("⚠️ Restart with adjusted limits not yet implemented - requires parameter adjustment logic");
                // TODO: Implement restart with adjusted resource limits
                // This would modify the process configuration before triggering restart
            }
        }
    }
    
    /// Spawn resource monitoring background task
    fn spawn_resource_monitor_task(&mut self) {
        let Some(pid) = self.pid else {
            return;
        };
        
        info!("Spawning resource monitor task for {} (PID: {})", self.config.id, pid);
        
        let process_id = self.config.id.clone();
        let resource_usage = Arc::clone(&self.resource_usage);
        let resource_monitor = Arc::clone(&self.resource_monitor);
        
        // Extract resource limits based on process type
        let limits = match &self.config.process_type {
            crate::config::ProcessManagementType::StandardManaged => {
                self.config.management.standard_managed.as_ref()
                    .and_then(|sm| sm.control.limits.clone())
            }
            crate::config::ProcessManagementType::IntegratedManaged => {
                self.config.management.integrated_managed.as_ref()
                    .and_then(|im| im.control.limits.clone())
            }
            crate::config::ProcessManagementType::Unmanaged => None,
        };
        
        // Create channel for sending violations
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.violation_tx = Some(tx.clone());
        
        // Spawn violation handler task
        self.spawn_violation_handler_task(rx);
        
        let task = tokio::spawn(async move {
            let mut interval_timer = interval(Duration::from_secs(10));
            
            loop {
                interval_timer.tick().await;
                
                // Collect resource usage
                match resource_monitor.get_process_resource_usage(pid) {
                    Ok(usage) => {
                        debug!("📊 Resource usage collected for {}: mem={:?} MB, cpu={:?}%", 
                               process_id, usage.memory_mb, usage.cpu_percent);
                        
                        // Update shared resource usage
                        {
                            let mut res = resource_usage.write().await;
                            *res = usage.clone();
                        }
                        
                        // Check limits with policy-aware checking
                        if let Some(ref limits_config) = limits {
                            debug!("✅ Resource limits config found for {}", process_id);
                            
                            // Convert new config structure to hsu_resource_limits types
                            let memory_limits = limits_config.memory.as_ref().map(|m| {
                                // Convert from ResourcePolicy enum to string
                                let policy_str = m.policy.as_ref().map(|p| p.as_str()).unwrap_or("none");
                                let policy = match policy_str {
                                    "log" => hsu_resource_limits::ResourcePolicy::Log,
                                    "alert" => hsu_resource_limits::ResourcePolicy::Alert,
                                    "throttle" => hsu_resource_limits::ResourcePolicy::Throttle,
                                    "graceful_shutdown" => hsu_resource_limits::ResourcePolicy::GracefulShutdown,
                                    "immediate_kill" => hsu_resource_limits::ResourcePolicy::ImmediateKill,
                                    "restart" => hsu_resource_limits::ResourcePolicy::Restart,
                                    "restart_adjusted" => hsu_resource_limits::ResourcePolicy::RestartAdjusted,
                                    _ => hsu_resource_limits::ResourcePolicy::None,
                                };
                                
                                // Get limit in bytes using helper method (handles both max_rss and limit_mb)
                                let max_rss_bytes = m.max_rss_bytes();
                                let max_rss_mb = max_rss_bytes / (1024 * 1024);
                                info!("💾 Memory limit configured for {}: {} MB (policy: {:?})", 
                                      process_id, max_rss_mb, policy);
                                
                                hsu_resource_limits::MemoryLimits {
                                    max_rss_mb,
                                    warning_threshold: m.warning_threshold.map(|t| t as f32),
                                    policy,
                                }
                            });
                            
                            let cpu_limits = limits_config.cpu.as_ref().map(|c| {
                                let policy_str = c.policy.as_ref().map(|p| p.as_str()).unwrap_or("none");
                                let policy = match policy_str {
                                    "log" => hsu_resource_limits::ResourcePolicy::Log,
                                    "alert" => hsu_resource_limits::ResourcePolicy::Alert,
                                    "throttle" => hsu_resource_limits::ResourcePolicy::Throttle,
                                    "graceful_shutdown" => hsu_resource_limits::ResourcePolicy::GracefulShutdown,
                                    "immediate_kill" => hsu_resource_limits::ResourcePolicy::ImmediateKill,
                                    "restart" => hsu_resource_limits::ResourcePolicy::Restart,
                                    "restart_adjusted" => hsu_resource_limits::ResourcePolicy::RestartAdjusted,
                                    _ => hsu_resource_limits::ResourcePolicy::None,
                                };
                                
                                hsu_resource_limits::CpuLimits {
                                    max_percent: c.max_percent,
                                    warning_threshold: c.warning_threshold.map(|t| t as f32),
                                    policy,
                                }
                            });
                            
                            let process_limits = limits_config.process.as_ref().map(|p| {
                                let policy_str = p.policy.as_ref().map(|p| p.as_str()).unwrap_or("none");
                                let policy = match policy_str {
                                    "log" => hsu_resource_limits::ResourcePolicy::Log,
                                    "alert" => hsu_resource_limits::ResourcePolicy::Alert,
                                    "throttle" => hsu_resource_limits::ResourcePolicy::Throttle,
                                    "graceful_shutdown" => hsu_resource_limits::ResourcePolicy::GracefulShutdown,
                                    "immediate_kill" => hsu_resource_limits::ResourcePolicy::ImmediateKill,
                                    "restart" => hsu_resource_limits::ResourcePolicy::Restart,
                                    "restart_adjusted" => hsu_resource_limits::ResourcePolicy::RestartAdjusted,
                                    _ => hsu_resource_limits::ResourcePolicy::None,
                                };
                                
                                hsu_resource_limits::ProcessLimits {
                                    max_file_descriptors: p.max_file_descriptors.unwrap_or(1024),
                                    warning_threshold: None,
                                    policy,
                                }
                            });
                            
                            // Log current usage for debugging (before move)
                            if let Some(ref mem_limits) = memory_limits {
                                let current_mb = usage.memory_mb.unwrap_or(0);
                                debug!("🔍 Checking memory for {}: current={} MB, limit={} MB", 
                                       process_id, current_mb, mem_limits.max_rss_mb);
                            }
                            
                            // Convert config to ResourceLimits
                            let resource_limits = hsu_resource_limits::ResourceLimits {
                                memory: memory_limits,
                                cpu: cpu_limits,
                                process: process_limits,
                                // Legacy fields (will be removed in future)
                                max_memory_mb: None,
                                max_cpu_percent: None,
                                max_file_descriptors: None,
                            };
                            
                            // Check with policy-aware function
                            let violations = check_resource_limits_with_policy(&process_id, &usage, &resource_limits);
                            
                            // Log violations detected
                            if !violations.is_empty() {
                                warn!("⚠️  Detected {} resource violation(s) for {}", violations.len(), process_id);
                            }
                            
                            // Send violations through channel for handling
                            for (violation, policy) in violations {
                                info!("📤 Sending violation to handler: {:?} → policy={:?}", 
                                      violation.limit_type, policy);
                                if let Err(e) = tx.send((policy, violation)) {
                                    error!("Failed to send resource violation for {}: {}", process_id, e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("❌ Failed to collect resource usage for {}: {}", process_id, e);
                        // Process might have exited, just continue
                    }
                }
                
                debug!("🔄 Resource monitor loop iteration complete for {}", process_id);
            }
        });
        
        self.resource_monitor_task = Some(task);
    }
    
    /// Spawn health check task for process health monitoring
    /// 
    /// Following Go's pattern: health check config is REQUIRED for restart logic to work.
    /// If not configured, we create a default "process" health check.
    fn spawn_health_check_task(&mut self) {
        let process_id = self.config.id.clone();
        
        // Get PID (required for all health check types)
        let Some(pid) = self.pid else {
            error!("No PID available for health check: {}", self.config.id);
            return;
        };
        
        // Extract health check config from the process configuration
        let health_check_config = match &self.config.process_type {
            crate::config::ProcessManagementType::StandardManaged => {
                self.config.management.standard_managed.as_ref()
                    .and_then(|sm| sm.health_check.as_ref())
            }
            crate::config::ProcessManagementType::IntegratedManaged => {
                self.config.management.integrated_managed.as_ref()
                    .and_then(|im| im.health_check.as_ref())
            }
            crate::config::ProcessManagementType::Unmanaged => None,
        };

        // Create health monitor config - use provided config or default
        // IMPORTANT: Following Go's pattern, we ALWAYS create a health monitor
        // because restart logic depends on it detecting process exit
        let monitor_config = if let Some(health_config) = health_check_config {
            // Use run_options if available, otherwise use defaults
            let (interval, timeout, retries) = if let Some(ref run_opts) = health_config.run_options {
                (run_opts.interval, run_opts.timeout, run_opts.retries)
            } else {
                (Duration::from_secs(10), Duration::from_secs(5), 3)
            };
            
            let check_type = match health_config.health_check_type.as_str() {
                "process" => HealthCheckType::Process,
                "http" => HealthCheckType::Http,
                _ => HealthCheckType::Process,
            };
            
            info!("Starting health monitor for {} (type: {:?})", process_id, check_type);
            if check_type == HealthCheckType::Http {
                if let Some(ref endpoint) = health_config.http_endpoint {
                    info!("HTTP health check endpoint: {}", endpoint);
                } else {
                    warn!("HTTP health check type specified but no endpoint configured for {}", process_id);
                }
            }
            
            HealthCheckConfig {
                check_type,
                interval,
                timeout,
                failure_threshold: retries,
                recovery_threshold: 2, // Default recovery threshold
                http_endpoint: health_config.http_endpoint.clone(),
            }
        } else {
            // No health check configured - create default process-based health check
            // This is CRITICAL for restart logic to work (detects process exit)
            info!("No health check configured for {}, creating default process health check", process_id);
            
            HealthCheckConfig {
                check_type: HealthCheckType::Process,
                interval: Duration::from_secs(10),
                timeout: Duration::from_secs(5),
                failure_threshold: 3,
                recovery_threshold: 2,
                http_endpoint: None,
            }
        };
        
        // Create health monitor
        let mut health_monitor = HealthMonitor::new(
            process_id.clone(),
            pid,
            monitor_config,
        );
        
        // Set up restart callback (fires when health check fails)
        let restart_process_id = process_id.clone();
        let lifecycle_manager = Arc::clone(&self.lifecycle_manager);
        let restart_tx = self.restart_tx.clone();
        
        health_monitor.set_restart_callback(move |reason: String| {
            info!("🔔 Health restart callback invoked for {}: {}", restart_process_id, reason);
            
            // Use try_read() which is non-blocking (returns immediately if lock is held)
            let should_restart = match lifecycle_manager.try_read() {
                Ok(manager) => {
                    let is_tripped = manager.is_circuit_breaker_tripped();
                    let should = manager.should_restart();
                    info!("🔧 Restart policy check for {}: should_restart={}, circuit_breaker_tripped={}", 
                          restart_process_id, should, is_tripped);
                    should
                }
                Err(_) => {
                    // Lock is held, assume restart is OK and let process_pending_restarts decide
                    warn!("⚠️ Could not acquire lock for restart policy check, queuing anyway");
                    true
                }
            };

            if !should_restart {
                warn!("⛔ Health restart blocked by policy for {}: {}", restart_process_id, reason);
                return Ok(());
            }

            info!("✅ Health restart allowed, queueing restart request for {}: {}", restart_process_id, reason);

            // Send restart request via channel
            let request = RestartRequest {
                reason: reason.clone(),
                trigger_type: RestartTriggerType::HealthFailure,
                timestamp: chrono::Utc::now(),
            };

            match restart_tx.send(request) {
                Ok(()) => {
                    info!("📬 Restart request queued successfully for {}", restart_process_id);
                    Ok(())
                }
                Err(e) => {
                    error!("❌ Failed to queue restart request for {}: {}", restart_process_id, e);
                    Err(format!("Failed to queue restart: {}", e))
                }
            }
        });

        // Set up recovery callback (fires when health recovers after failures)
        let recovery_process_id = process_id.clone();
        let recovery_lifecycle_manager = Arc::clone(&self.lifecycle_manager);
        health_monitor.set_recovery_callback(move || {
            info!("Health recovered for {}, resetting circuit breaker", recovery_process_id);
            // Use try_write() which is non-blocking
            match recovery_lifecycle_manager.try_write() {
                Ok(mut manager) => {
                    manager.reset_circuit_breaker();
                    info!("✅ Circuit breaker reset for {}", recovery_process_id);
                }
                Err(_) => {
                    warn!("⚠️ Could not acquire write lock to reset circuit breaker for {}", recovery_process_id);
                }
            }
        });

        // Start the health monitor (it spawns its own background task)
        health_monitor.start();
        
        info!("Health check task spawned for {}", self.config.id);
        
        // Store the health monitor
        self.health_monitor = Some(health_monitor);
    }
    
    /// Spawn restart handler task that monitors pending restart requests
    /// 
    /// Note: The actual restart is handled by calling process_pending_restarts() externally,
    /// as the background task cannot hold &mut self to call restart().
    fn spawn_restart_handler_task(&mut self) {
        let process_id = self.config.id.clone();
        let restart_rx = Arc::clone(&self.restart_rx);
        
        info!("Restart handler monitoring task spawned for {}", process_id);
        
        let task = tokio::spawn(async move {
            let mut check_interval = tokio::time::interval(Duration::from_secs(2));
            let mut pending_logged = false;
            
            loop {
                check_interval.tick().await;
                
                // Check if there are pending restart requests (peek without consuming)
                let has_pending = {
                    let rx = restart_rx.lock().await;
                    !rx.is_empty()
                };
                
                if has_pending && !pending_logged {
                    info!("Restart request(s) pending for {} - will be processed on next heartbeat",
                          process_id);
                    pending_logged = true;
                } else if !has_pending {
                    pending_logged = false;
                }
            }
        });
        
        self.restart_handler_task = Some(task);
    }
    
    /// Stop all background tasks
    fn stop_background_tasks(&mut self) {
        if let Some(task) = self.exit_monitor_task.take() {
            task.abort();
            debug!("Exit monitor task stopped for {}", self.config.id);
        }
        
        if let Some(mut monitor) = self.health_monitor.take() {
            monitor.stop();
            debug!("Health monitor stopped for {}", self.config.id);
        }
        
        if let Some(task) = self.health_monitor_task.take() {
            task.abort();
            debug!("Health monitor task stopped for {}", self.config.id);
        }
        
        if let Some(task) = self.resource_monitor_task.take() {
            task.abort();
            debug!("Resource monitor task stopped for {}", self.config.id);
        }
        
        if let Some(task) = self.violation_handler_task.take() {
            task.abort();
            debug!("Violation handler task stopped for {}", self.config.id);
        }
        
        if let Some(task) = self.restart_handler_task.take() {
            task.abort();
            debug!("Restart handler task stopped for {}", self.config.id);
        }
        
        // Drop the violation sender to signal handler task to exit
        self.violation_tx = None;
    }
    
}

#[async_trait]
impl ProcessControl for ProcessControlImpl {
    async fn start(&mut self) -> ProcessResult<()> {
        info!("Starting process control: {}", self.config.id);
        
        // Transition to starting state
        self.state_machine.transition_to_starting()
            .map_err(|e| CommonProcessError::StartFailed {
                id: self.config.id.clone(),
                reason: format!("Failed to transition to starting: {}", e),
            })?;
        
        // Spawn the process
        self.spawn_process().await?;
        
        // Transition to running
        self.state_machine.transition_to_running()
            .map_err(|e| CommonProcessError::StartFailed {
                id: self.config.id.clone(),
                reason: format!("Failed to transition to running: {}", e),
            })?;
        
        // Start background tasks
        self.spawn_exit_monitor_task();  // Must be first - takes ownership of child
        self.spawn_health_check_task();
        self.spawn_resource_monitor_task();
        self.spawn_restart_handler_task();  // Handle automatic restarts from callbacks
        
        info!("Process control started successfully: {}", self.config.id);
        Ok(())
    }
    
    async fn stop(&mut self) -> ProcessResult<()> {
        info!("Stopping process control: {}", self.config.id);
        
        // Transition to stopping state
        if self.state_machine.can_stop() {
            let _ = self.state_machine.transition_to_stopping();
        }
        
        // Terminate the process FIRST (this waits for exit_monitor_task to complete naturally)
        self.terminate_process().await?;
        
        // NOW stop remaining background tasks (exit_monitor already completed)
        self.stop_background_tasks();
        
        // Transition to stopped
        let _ = self.state_machine.transition_to_stopped();
        
        info!("Process control stopped successfully: {}", self.config.id);
        Ok(())
    }
    
    async fn restart(&mut self, force: bool) -> ProcessResult<()> {
        info!("Restarting process control: {} (force: {})", self.config.id, force);
        
        // Check circuit breaker unless forced
        if !force {
            let is_tripped = self.lifecycle_manager.read().await.is_circuit_breaker_tripped();
            if is_tripped {
                let err = ProcessError::new(
                    ErrorCategory::CircuitBreakerTripped,
                    "Circuit breaker is tripped, cannot restart".to_string(),
                    false,
                );
                self.last_error = Some(err.clone());
                
                return Err(CommonProcessError::Configuration {
                    id: self.config.id.clone(),
                    reason: err.details,
                });
            }
        }
        
        // Stop first
        self.stop().await?;
        
        // Wait a bit before restarting
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Update restart tracking
        self.restart_count += 1;
        self.last_restart_time = Some(chrono::Utc::now());
        
        // Reset health status for fresh start
        {
            let mut status = self.health_status.write().await;
            *status = HealthStatus::new();
        }
        
        // Start again
        self.start().await?;
        
        // Reset lifecycle manager on successful restart
        self.lifecycle_manager.write().await.reset_restart_counters();
        
        info!("Process control restarted successfully: {}", self.config.id);
        Ok(())
    }
    
    fn get_state(&self) -> ProcessState {
        self.state_machine.current_state()
    }
    
    fn get_diagnostics(&self) -> ProcessDiagnostics {
        let health_status = self.health_status.try_read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| HealthStatus::new());
        
        let resource_usage = self.resource_usage.try_read()
            .map(|r| r.clone())
            .unwrap_or(ResourceUsage {
                cpu_percent: None,
                memory_mb: None,
                file_descriptors: None,
                network_connections: None,
            });
        
        // Extract executable path based on process type
        let executable_path = match &self.config.process_type {
            crate::config::ProcessManagementType::StandardManaged => {
                self.config.management.standard_managed.as_ref()
                    .map(|sm| sm.control.execution.executable_path.clone())
                    .unwrap_or_else(|| "unknown".to_string())
            }
            crate::config::ProcessManagementType::IntegratedManaged => {
                self.config.management.integrated_managed.as_ref()
                    .map(|im| im.control.execution.executable_path.clone())
                    .unwrap_or_else(|| "unknown".to_string())
            }
            crate::config::ProcessManagementType::Unmanaged => "unmanaged".to_string(),
        };
        
        ProcessDiagnostics {
            state: self.state_machine.current_state(),
            last_error: self.last_error.clone(),
            process_id: self.pid,
            start_time: self.start_time,
            executable_path: executable_path.clone(),
            executable_exists: std::path::Path::new(&executable_path).exists(),
            failure_count: {
                // NOTE: `get_diagnostics()` is a synchronous method (trait requirement),
                // so we must not block the Tokio runtime thread here. Use a best-effort
                // snapshot of the lifecycle manager state.
                self.lifecycle_manager
                    .try_read()
                    .map(|m| m.get_restart_stats().consecutive_failures)
                    .unwrap_or(0)
            },
            last_attempt_time: self.last_restart_time,
            is_healthy: health_status.is_healthy,
            cpu_usage: resource_usage.cpu_percent,
            memory_usage: resource_usage.memory_mb,
        }
    }
    
    fn get_pid(&self) -> Option<u32> {
        self.pid
    }
    
    fn is_healthy(&self) -> bool {
        self.health_status.try_read()
            .map(|s| s.is_healthy)
            .unwrap_or(true)
    }
    
    /// Process pending restart requests from health checks and resource violations
    /// 
    /// This method is called by ProcessManager periodically to handle automatic restarts.
    async fn process_pending_restarts(&mut self) -> ProcessResult<usize> {
        info!("⏰ Heartbeat: Checking for pending restarts for {}", self.config.id);
        let mut restart_count = 0;
        
        // Process all pending restart requests (non-blocking)
        loop {
            let request = {
                let mut rx = self.restart_rx.lock().await;
                let req = rx.try_recv().ok();
                if req.is_some() {
                    info!("📦 Found pending restart request for {}", self.config.id);
                }
                req
            };
            
            match request {
                Some(req) => {
                    info!("🔄 Processing restart request for {}: {} (trigger: {:?})",
                          self.config.id, req.reason, req.trigger_type);
                    
                    // Attempt restart (force=false, use circuit breaker)
                    match self.restart(false).await {
                        Ok(()) => {
                            info!("✅ Automatic restart succeeded for {} (trigger: {:?})",
                                  self.config.id, req.trigger_type);
                            restart_count += 1;
                        }
                        Err(e) => {
                            error!("❌ Automatic restart failed for {}: {} (trigger: {:?})",
                                   self.config.id, e, req.trigger_type);
                            // Don't fail the whole operation, just log and continue
                        }
                    }
                }
                None => {
                    // No more pending requests
                    break;
                }
            }
        }
        
        if restart_count > 0 {
            info!("Processed {} restart request(s) for {}", restart_count, self.config.id);
        }
        
        Ok(restart_count)
    }
}

#[cfg(test)]
mod stop_tests {
    use super::*;
    use crate::config::{
        ExecutionConfig, ManagedProcessControlConfig, ProcessConfig, ProcessManagementConfig,
        ProcessManagementType, StandardManagedProcessConfig,
    };
    use std::collections::HashMap;
    use hsu_managed_process::ProcessControl;

    fn minimal_control() -> ProcessControlImpl {
        let config = ProcessConfig {
            id: "test-process".to_string(),
            process_type: ProcessManagementType::StandardManaged,
            profile_type: "test".to_string(),
            enabled: true,
            management: ProcessManagementConfig {
                standard_managed: Some(StandardManagedProcessConfig {
                    metadata: None,
                    control: ManagedProcessControlConfig {
                        execution: ExecutionConfig {
                            executable_path: "does-not-matter".to_string(),
                            args: vec![],
                            working_directory: None,
                            environment: HashMap::new(),
                            wait_delay: Duration::from_millis(10),
                        },
                        process_file: None,
                        context_aware_restart: None,
                        restart_policy: None,
                        limits: None,
                        graceful_timeout: Some(Duration::from_millis(100)),
                        log_collection: None,
                    },
                    health_check: None,
                }),
                integrated_managed: None,
                unmanaged: None,
            },
        };

        let control_config = ProcessControlConfig {
            process_id: "test-process".to_string(),
            can_attach: false,
            can_terminate: true,
            can_restart: true,
            graceful_timeout: Duration::from_millis(100),
            process_profile_type: "test".to_string(),
            log_collection_service: None,
            log_config: None,
        };

        ProcessControlImpl::new(config, control_config)
    }

    #[tokio::test]
    async fn stop_is_idempotent_when_not_running() {
        let mut control = minimal_control();
        // No pid/child => should be a no-op success.
        control.stop().await.unwrap();
    }
}

impl Drop for ProcessControlImpl {
    fn drop(&mut self) {
        // Clean up background tasks
        if let Some(mut monitor) = self.health_monitor.take() {
            monitor.stop();
        }
        if let Some(task) = self.resource_monitor_task.take() {
            task.abort();
        }
        if let Some(task) = self.violation_handler_task.take() {
            task.abort();
        }
    }
}

