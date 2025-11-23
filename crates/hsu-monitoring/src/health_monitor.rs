//! Health Monitor - Continuous health checking with restart callbacks
//!
//! This module provides a background task that continuously checks process health
//! and triggers callbacks when health fails or recovers. It supports:
//! - Process-based health checks (PID existence)
//! - HTTP health checks  
//! - Restart callbacks (triggered on failure)
//! - Recovery callbacks (triggered when health recovers)
//!
//! This corresponds to Go's `pkg/monitoring/health_check.go`

use crate::{HealthCheckData, HealthStatus};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tracing::{debug, error, info, warn};

/// Callback type for triggering process restart on health failure
pub type RestartCallback = Arc<dyn Fn(String) -> Result<(), String> + Send + Sync>;

/// Callback type for health recovery (resets circuit breaker)
pub type RecoveryCallback = Arc<dyn Fn() + Send + Sync>;

/// Health check configuration
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// Check type: "process", "http", etc.
    pub check_type: HealthCheckType,
    
    /// Interval between health checks
    pub interval: Duration,
    
    /// Timeout for each health check
    pub timeout: Duration,
    
    /// Number of consecutive failures before considering unhealthy
    pub failure_threshold: u32,
    
    /// Number of consecutive successes before considering healthy again
    pub recovery_threshold: u32,
    
    /// HTTP endpoint (for HTTP checks)
    pub http_endpoint: Option<String>,
}

/// Types of health checks
#[derive(Debug, Clone, PartialEq)]
pub enum HealthCheckType {
    /// Check if process PID exists
    Process,
    /// HTTP endpoint health check
    Http,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            check_type: HealthCheckType::Process,
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(5),
            failure_threshold: 3,
            recovery_threshold: 2,
            http_endpoint: None,
        }
    }
}

/// Health monitor - runs continuous health checking in background
pub struct HealthMonitor {
    /// Process ID being monitored
    process_id: String,
    
    /// PID of the process (for process-based health checks)
    pid: u32,
    
    /// Health check configuration
    config: HealthCheckConfig,
    
    /// Current health status
    status: Arc<RwLock<HealthStatus>>,
    
    /// Restart callback (called when health check fails)
    restart_callback: Option<RestartCallback>,
    
    /// Recovery callback (called when health recovers)
    recovery_callback: Option<RecoveryCallback>,
    
    /// Background task handle
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(
        process_id: String,
        pid: u32,
        config: HealthCheckConfig,
    ) -> Self {
        Self {
            process_id,
            pid,
            config,
            status: Arc::new(RwLock::new(HealthStatus::new())),
            restart_callback: None,
            recovery_callback: None,
            task_handle: None,
        }
    }
    
    /// Set the restart callback (called on health failure)
    pub fn set_restart_callback<F>(&mut self, callback: F)
    where
        F: Fn(String) -> Result<(), String> + Send + Sync + 'static,
    {
        self.restart_callback = Some(Arc::new(callback));
        debug!("Restart callback set for health monitor: {}", self.process_id);
    }
    
    /// Set the recovery callback (called when health recovers)
    pub fn set_recovery_callback<F>(&mut self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.recovery_callback = Some(Arc::new(callback));
        debug!("Recovery callback set for health monitor: {}", self.process_id);
    }
    
    /// Start the health monitor background task
    pub fn start(&mut self) {
        if self.task_handle.is_some() {
            warn!("Health monitor already started for {}", self.process_id);
            return;
        }
        
        let process_id = self.process_id.clone();
        let pid = self.pid;
        let config = self.config.clone();
        let status = Arc::clone(&self.status);
        let restart_callback = self.restart_callback.clone();
        let recovery_callback = self.recovery_callback.clone();
        
        let task = tokio::spawn(async move {
            Self::run_health_check_loop(
                process_id,
                pid,
                config,
                status,
                restart_callback,
                recovery_callback,
            ).await;
        });
        
        self.task_handle = Some(task);
        info!("Health monitor started for {}", self.process_id);
    }
    
    /// Stop the health monitor
    pub fn stop(&mut self) {
        if let Some(task) = self.task_handle.take() {
            task.abort();
            debug!("Health monitor stopped for {}", self.process_id);
        }
    }
    
    /// Get current health status
    pub async fn get_status(&self) -> HealthStatus {
        self.status.read().await.clone()
    }
    
    /// Health check loop (runs in background task)
    async fn run_health_check_loop(
        process_id: String,
        pid: u32,
        config: HealthCheckConfig,
        status: Arc<RwLock<HealthStatus>>,
        restart_callback: Option<RestartCallback>,
        recovery_callback: Option<RecoveryCallback>,
    ) {
        let mut check_interval = interval(config.interval);
        let mut was_healthy = true;
        
        info!("Health check loop started for {} (PID: {}, interval: {:?}, failure_threshold: {})", 
              process_id, pid, config.interval, config.failure_threshold);
        
        loop {
            check_interval.tick().await;
            
            info!("🔍 Running health check for {} (PID: {})", process_id, pid);
            
            // Perform health check based on type
            let check_result = match config.check_type {
                HealthCheckType::Process => {
                    let result = Self::check_process_health(pid);
                    info!("💓 Process health check result for {}: healthy={}, error={:?}", 
                           process_id, result.is_healthy, result.error_message);
                    result
                }
                HealthCheckType::Http => {
                    // HTTP health check implementation
                    if let Some(ref endpoint) = config.http_endpoint {
                        Self::check_http_health(endpoint, config.timeout).await
                    } else {
                        warn!("HTTP health check configured but no endpoint provided for {}", process_id);
                        continue;
                    }
                }
            };
            
            // Update status based on check result
            let mut current_status = status.write().await;
            let was_healthy_before = current_status.is_healthy;
            
            if check_result.is_healthy {
                current_status.record_success();
                
                // Check if we just recovered (was unhealthy, now healthy for recovery_threshold times)
                if !was_healthy_before && current_status.consecutive_successes >= config.recovery_threshold {
                    info!("Process {} health recovered (consecutive successes: {})",
                          process_id, current_status.consecutive_successes);
                    
                    // Call recovery callback (resets circuit breaker)
                    if let Some(ref callback) = recovery_callback {
                        callback();
                    }
                    
                    was_healthy = true;
                }
                
                debug!("Health check passed for {} (consecutive successes: {})", 
                       process_id, current_status.consecutive_successes);
            } else {
                let reason = check_result.error_message.unwrap_or_else(|| "Unknown failure".to_string());
                current_status.record_failure(reason.clone(), config.failure_threshold);
                
                warn!("❌ Health check failed for {}: consecutive failures = {}/{}, reason: {}",
                      process_id, current_status.consecutive_failures, config.failure_threshold, reason);
                
                // Check if we just became unhealthy (hit failure threshold)
                if was_healthy && current_status.consecutive_failures >= config.failure_threshold {
                    error!("🚨 Health check failure threshold reached for {} (failures: {})",
                           process_id, current_status.consecutive_failures);
                    
                    // Call restart callback
                    if let Some(ref callback) = restart_callback {
                        info!("📞 Calling restart callback for {}", process_id);
                        match callback(reason.clone()) {
                            Ok(()) => {
                                info!("✅ Restart callback succeeded for {}", process_id);
                            }
                            Err(e) => {
                                error!("❌ Restart callback failed for {}: {}", process_id, e);
                            }
                        }
                    } else {
                        warn!("⚠️ No restart callback configured for {}, cannot trigger restart", process_id);
                    }
                    
                    was_healthy = false;
                }
            }
        }
    }
    
    /// Check if process is still running (by PID)
    fn check_process_health(pid: u32) -> HealthCheckData {
        let checked_at = chrono::Utc::now();
        
        // Check if process exists using platform-specific method
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            
            // Send signal 0 (no-op) to check if process exists
            match kill(Pid::from_raw(pid as i32), Signal::from_c_int(0).ok()) {
                Ok(()) => {
                    debug!("Process {} exists (Unix signal check)", pid);
                    HealthCheckData {
                        is_healthy: true,
                        checked_at,
                        response_time_ms: Some(0),
                        error_message: None,
                    }
                }
                Err(e) => {
                    info!("Process {} not found (Unix signal check failed: {:?})", pid, e);
                    HealthCheckData {
                        is_healthy: false,
                        checked_at,
                        response_time_ms: Some(0),
                        error_message: Some(format!("Process {} not found", pid)),
                    }
                }
            }
        }
        
        #[cfg(windows)]
        {
            // On Windows, use OpenProcess + GetExitCodeProcess to check if process is alive
            use std::ptr;
            use winapi::um::processthreadsapi::{OpenProcess, GetExitCodeProcess};
            use winapi::um::winnt::PROCESS_QUERY_INFORMATION;
            use winapi::um::handleapi::CloseHandle;
            use winapi::shared::minwindef::DWORD;
            
            const STILL_ACTIVE: DWORD = 259;
            
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
                if handle != ptr::null_mut() {
                    // Process handle opened successfully, but check if it's actually alive
                    let mut exit_code: DWORD = 0;
                    let result = GetExitCodeProcess(handle, &mut exit_code);
                    CloseHandle(handle);
                    
                    if result != 0 && exit_code == STILL_ACTIVE {
                        // Process is still running
                        debug!("Process {} is alive (Windows: exit_code=STILL_ACTIVE)", pid);
                        HealthCheckData {
                            is_healthy: true,
                            checked_at,
                            response_time_ms: Some(0),
                            error_message: None,
                        }
                    } else {
                        // Process has exited (or GetExitCodeProcess failed)
                        info!("Process {} has exited (Windows: exit_code={}, result={})", pid, exit_code, result);
                        HealthCheckData {
                            is_healthy: false,
                            checked_at,
                            response_time_ms: Some(0),
                            error_message: Some(format!("Process {} has exited (exit code: {})", pid, exit_code)),
                        }
                    }
                } else {
                    info!("Process {} not found (Windows OpenProcess failed)", pid);
                    HealthCheckData {
                        is_healthy: false,
                        checked_at,
                        response_time_ms: Some(0),
                        error_message: Some(format!("Process {} not found", pid)),
                    }
                }
            }
        }
    }
    
    /// Check HTTP health endpoint
    async fn check_http_health(endpoint: &str, timeout_duration: Duration) -> HealthCheckData {
        let checked_at = chrono::Utc::now();
        let start = std::time::Instant::now();
        
        match crate::http::check_http_health(endpoint, timeout_duration).await {
            Ok(data) => data,
            Err(e) => HealthCheckData {
                is_healthy: false,
                checked_at,
                response_time_ms: Some(start.elapsed().as_millis() as u64),
                error_message: Some(format!("HTTP health check failed: {}", e)),
            },
        }
    }
}

impl Drop for HealthMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

