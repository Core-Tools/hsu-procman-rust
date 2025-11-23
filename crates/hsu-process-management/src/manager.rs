//! Simplified Process Manager - Orchestration layer using ProcessControl trait
//!
//! This module focuses on high-level orchestration:
//! - Managing multiple processes
//! - Configuration management
//! - Process lifecycle coordination
//! - Reattachment after restart
//!
//! All process control logic is delegated to ProcessControl implementations.

use crate::config::{ProcessConfig, ProcessManagerConfig};
use crate::process_control_impl::ProcessControlImpl;
use hsu_common::ProcessError;
use hsu_process_state::ProcessState;
use hsu_process_file::ProcessFileManager;
use hsu_managed_process::{ProcessControl, ProcessControlConfig};
use hsu_monitoring::HealthStatus;
use hsu_resource_limits::ResourceUsage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Duration;
use tracing::{debug, error, info, warn};

/// Result type for process management operations
type Result<T> = std::result::Result<T, ProcessError>;

/// Main process manager that orchestrates all child processes
pub struct ProcessManager {
    config: ProcessManagerConfig,
    processes: Arc<RwLock<HashMap<String, ManagedProcessInstance>>>,
    state: Arc<Mutex<ProcessManagerState>>,
    pid_file_manager: ProcessFileManager,
}

/// Individual managed process instance with runtime state
/// 
/// This is now a lightweight wrapper around ProcessControl trait,
/// focusing on orchestration rather than implementation details.
pub struct ManagedProcessInstance {
    /// Process configuration
    pub config: ProcessConfig,
    
    /// Process control implementation (encapsulates all lifecycle logic)
    pub process_control: Box<dyn ProcessControl>,
    
    /// Manager-level restart tracking
    pub restart_count: u32,
    pub last_restart_time: Option<chrono::DateTime<chrono::Utc>>,
}

/// Process manager overall state
#[derive(Debug, Clone)]
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

impl ProcessManager {
    /// Create a new process manager with the given configuration
    pub async fn new(config: ProcessManagerConfig) -> Result<Self> {
        info!("Creating process manager with {} processes", config.managed_processes.len());
        
        let manager = Self {
            config,
            processes: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(Mutex::new(ProcessManagerState::Initializing)),
            pid_file_manager: ProcessFileManager::with_defaults(),
        };

        // Initialize processes from configuration
        manager.initialize_processes().await?;
        
        // Attempt to reattach to existing processes
        manager.attempt_reattachment().await?;

        Ok(manager)
    }

    /// Initialize all processes from configuration
    async fn initialize_processes(&self) -> Result<()> {
        let mut processes = self.processes.write().await;
        
        for process_config in &self.config.managed_processes {
            if !process_config.enabled {
                debug!("Skipping disabled process: {}", process_config.id);
                continue;
            }

            // Extract graceful timeout based on process type
            let graceful_timeout = match &process_config.process_type {
                crate::config::ProcessManagementType::StandardManaged => {
                    process_config.management.standard_managed.as_ref()
                        .and_then(|sm| sm.control.graceful_timeout)
                        .unwrap_or(Duration::from_secs(10))
                }
                crate::config::ProcessManagementType::IntegratedManaged => {
                    process_config.management.integrated_managed.as_ref()
                        .and_then(|im| im.control.graceful_timeout)
                        .unwrap_or(Duration::from_secs(10))
                }
                crate::config::ProcessManagementType::Unmanaged => {
                    process_config.management.unmanaged.as_ref()
                        .and_then(|u| u.control.as_ref())
                        .map(|c| c.graceful_timeout)
                        .unwrap_or(Duration::from_secs(10))
                }
            };

            // Create ProcessControl implementation
            let control_config = ProcessControlConfig {
                process_id: process_config.id.clone(),
                can_attach: matches!(process_config.process_type, crate::config::ProcessManagementType::Unmanaged),
                can_terminate: !matches!(process_config.process_type, crate::config::ProcessManagementType::Unmanaged),
                can_restart: !matches!(process_config.process_type, crate::config::ProcessManagementType::Unmanaged),
                graceful_timeout,
                process_profile_type: process_config.profile_type.clone(),
                log_collection_service: None,  // TODO: Add log collection service integration
                log_config: None,               // TODO: Add log config from process config
            };
            
            let process_control = Box::new(ProcessControlImpl::new(
                process_config.clone(),
                control_config,
            ));
            
            let managed_process = ManagedProcessInstance {
                config: process_config.clone(),
                process_control,
                restart_count: 0,
                last_restart_time: None,
            };

            processes.insert(process_config.id.clone(), managed_process);
            info!("Initialized process: {}", process_config.id);
        }

        Ok(())
    }
    
    /// Attempt to reattach to existing processes from previous manager run
    async fn attempt_reattachment(&self) -> Result<()> {
        info!("Scanning for existing processes to reattach");
        
        let processes = self.processes.read().await;
        let mut reattached_count = 0;
        let mut cleaned_count = 0;
        
        for (process_id, _managed_process) in processes.iter() {
            // Try to read the PID file for this process
            match self.pid_file_manager.read_pid_file(process_id).await {
                Ok(pid) => {
                    // Check if process still exists
                    match hsu_process::process_exists(pid) {
                        Ok(true) => {
                            info!("Found existing process: {} (PID: {})", process_id, pid);
                            // TODO: Implement reattachment logic in ProcessControl trait
                            // For now, just log and count
                            reattached_count += 1;
                        }
                        Ok(false) => {
                            info!("Process {} (PID: {}) no longer exists, cleaning up", process_id, pid);
                            if let Err(e) = self.pid_file_manager.delete_pid_file(process_id).await {
                                warn!("Failed to delete stale PID file for {}: {}", process_id, e);
                            } else {
                                cleaned_count += 1;
                            }
                        }
                        Err(e) => {
                            warn!("Failed to check if process {} exists: {}", process_id, e);
                        }
                    }
                }
                Err(_) => {
                    // No PID file found, process not running
                    debug!("No PID file found for process: {}", process_id);
                }
            }
        }
        
        info!("Reattachment scan complete: {} reattached, {} cleaned up", reattached_count, cleaned_count);
        Ok(())
    }

    /// Start the process manager and all enabled processes
    pub async fn start(&mut self) -> Result<()> {
        info!("Starting process manager");
        
        {
            let mut state = self.state.lock().await;
            *state = ProcessManagerState::Starting;
        }

        // Start all enabled processes
        let process_ids: Vec<String> = {
            let processes = self.processes.read().await;
            processes.keys().cloned().collect()
        };

        for process_id in process_ids {
            if let Err(e) = self.start_process(&process_id).await {
                error!("Failed to start process {}: {}", process_id, e);
            }
        }

        {
            let mut state = self.state.lock().await;
            *state = ProcessManagerState::Running;
        }

        info!("Process manager started successfully");
        
        // Spawn heartbeat task to handle automatic restarts
        self.spawn_heartbeat_task();
        
        Ok(())
    }
    
    /// Spawn a heartbeat task that periodically processes restart requests
    fn spawn_heartbeat_task(&self) {
        let processes = Arc::clone(&self.processes);
        
        tokio::spawn(async move {
            let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(2));
            
            loop {
                heartbeat_interval.tick().await;
                
                // Process pending restarts for all processes
                let process_ids: Vec<String> = {
                    let procs = processes.read().await;
                    procs.keys().cloned().collect()
                };
                
                for process_id in process_ids {
                    let mut processes_write = processes.write().await;
                    
                    if let Some(instance) = processes_write.get_mut(&process_id) {
                        // Process pending restarts via trait method
                        match instance.process_control.process_pending_restarts().await {
                            Ok(count) if count > 0 => {
                                info!("🔄 Processed {} automatic restart(s) for {}", count, process_id);
                                instance.restart_count += count as u32;
                                instance.last_restart_time = Some(chrono::Utc::now());
                            }
                            Ok(_) => {
                                // No restarts - this is normal, don't log
                            },
                            Err(e) => {
                                error!("❌ Error processing restart requests for {}: {}", process_id, e);
                            }
                        }
                    }
                }
            }
        });
        
        info!("Heartbeat task spawned for automatic restart processing");
    }

    /// Start a specific process by ID
    pub async fn start_process(&self, process_id: &str) -> Result<()> {
        info!("Starting process: {}", process_id);

        let mut processes = self.processes.write().await;
        let managed_process = processes.get_mut(process_id)
            .ok_or_else(|| ProcessError::NotFound { 
                id: process_id.to_string() 
            })?;

        // Delegate to ProcessControl
        managed_process.process_control.start().await?;

        // Write PID file for managed processes
        if let Some(pid) = managed_process.process_control.get_pid() {
            if let Err(e) = self.pid_file_manager.write_pid_file(process_id, pid).await {
                warn!("Failed to write PID file for {}: {}", process_id, e);
            }
        }

        info!("Process started successfully: {}", process_id);
        Ok(())
    }

    /// Stop a specific process by ID
    pub async fn stop_process(&self, process_id: &str) -> Result<()> {
        info!("Stopping process: {}", process_id);

        let mut processes = self.processes.write().await;
        let managed_process = processes.get_mut(process_id)
            .ok_or_else(|| ProcessError::NotFound { 
                id: process_id.to_string() 
            })?;

        // Delegate to ProcessControl
        managed_process.process_control.stop().await?;

        // Delete PID file for managed processes
        if let Err(e) = self.pid_file_manager.delete_pid_file(process_id).await {
            warn!("Failed to delete PID file for {}: {}", process_id, e);
        }

        info!("Process stopped successfully: {}", process_id);
        Ok(())
    }

    /// Restart a specific process by ID
    pub async fn restart_process(&self, process_id: &str, force: bool) -> Result<()> {
        info!("Restarting process: {} (force: {})", process_id, force);

        let mut processes = self.processes.write().await;
        let managed_process = processes.get_mut(process_id)
            .ok_or_else(|| ProcessError::NotFound { 
                id: process_id.to_string() 
            })?;

        // Update manager-level tracking
        managed_process.restart_count += 1;
        managed_process.last_restart_time = Some(chrono::Utc::now());

        // Delegate to ProcessControl
        managed_process.process_control.restart(force).await?;

        // Update PID file if PID changed
        if let Some(pid) = managed_process.process_control.get_pid() {
            if let Err(e) = self.pid_file_manager.write_pid_file(process_id, pid).await {
                warn!("Failed to update PID file for {}: {}", process_id, e);
            }
        }

        info!("Process restarted successfully: {}", process_id);
        Ok(())
    }

    /// Shutdown the process manager and all processes
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down process manager");

        {
            let mut state = self.state.lock().await;
            *state = ProcessManagerState::Stopping;
        }

        // Stop all processes
        let process_ids: Vec<String> = {
            let processes = self.processes.read().await;
            processes.keys().cloned().collect()
        };

        for process_id in process_ids {
            if let Err(e) = self.stop_process(&process_id).await {
                error!("Failed to stop process {} during shutdown: {}", process_id, e);
            }
        }

        {
            let mut state = self.state.lock().await;
            *state = ProcessManagerState::Stopped;
        }

        info!("Process manager shut down successfully");
        Ok(())
    }

    /// Get information about a specific process
    pub async fn get_process_info(&self, process_id: &str) -> Result<ProcessInfo> {
        let processes = self.processes.read().await;
        let managed_process = processes.get(process_id)
            .ok_or_else(|| ProcessError::NotFound { 
                id: process_id.to_string() 
            })?;

        // Get diagnostics from ProcessControl
        let diagnostics = managed_process.process_control.get_diagnostics();

        Ok(ProcessInfo {
            id: process_id.to_string(),
            state: diagnostics.state,
            pid: diagnostics.process_id,
            start_time: diagnostics.start_time,
            restart_count: managed_process.restart_count,
            cpu_usage: diagnostics.cpu_usage,
            memory_usage: diagnostics.memory_usage,
            uptime: diagnostics.start_time.map(|st| {
                let now = chrono::Utc::now();
                let duration = now.signed_duration_since(st);
                Duration::from_secs(duration.num_seconds() as u64)
            }),
            is_healthy: diagnostics.is_healthy,
            last_health_check: None, // TODO: Add to diagnostics
            consecutive_health_failures: diagnostics.failure_count,
        })
    }

    /// Get information about all processes
    pub async fn get_all_process_info(&self) -> Vec<ProcessInfo> {
        // Collect process IDs first
        let process_ids: Vec<String> = {
            let processes = self.processes.read().await;
            processes.keys().cloned().collect()
        };

        // Then get info for each (lock is released between iterations)
        let mut info_list = Vec::new();
        for process_id in process_ids {
            if let Ok(info) = self.get_process_info(&process_id).await {
                info_list.push(info);
            }
        }

        info_list
    }

    /// Get the current manager state
    pub async fn get_manager_state(&self) -> ProcessManagerState {
        let state = self.state.lock().await;
        state.clone()
    }
}

// Re-export for compatibility
pub use ProcessManagerState as State;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ProcessManagerOptions, ProcessManagementType, ProcessManagementConfig,
        StandardManagedProcessConfig, ManagedProcessControlConfig, ExecutionConfig,
        ContextAwareRestartConfig, RestartConfig,
    };

    fn create_test_config() -> ProcessManagerConfig {
        ProcessManagerConfig {
            process_manager: ProcessManagerOptions {
                port: 50055,
                log_level: "info".to_string(),
                force_shutdown_timeout: Duration::from_secs(30),
            },
            managed_processes: vec![
                ProcessConfig {
                    id: "test-process".to_string(),
                    enabled: true,
                    profile_type: "test".to_string(),
                    process_type: ProcessManagementType::StandardManaged,
                    management: ProcessManagementConfig {
                        standard_managed: Some(StandardManagedProcessConfig {
                            metadata: None,
                            control: ManagedProcessControlConfig {
                                execution: ExecutionConfig {
                                    executable_path: "echo".to_string(),
                                    args: vec!["hello".to_string()],
                                    working_directory: None,
                                    environment: HashMap::new(),
                                    wait_delay: Duration::from_millis(100),
                                },
                                process_file: None,
                                context_aware_restart: Some(ContextAwareRestartConfig {
                                    default: RestartConfig {
                                        max_retries: 3,
                                        retry_delay: Duration::from_secs(1),
                                        backoff_rate: 1.5,
                                    },
                                    health_failures: None,
                                    resource_violations: None,
                                    startup_grace_period: None,
                                    sustained_violation_time: None,
                                }),
                                restart_policy: None,
                                limits: None,
                                graceful_timeout: Some(Duration::from_secs(5)),
                                log_collection: None,
                            },
                            health_check: None,
                        }),
                        integrated_managed: None,
                        unmanaged: None,
                    },
                },
            ],
            log_collection: None,
        }
    }

    #[tokio::test]
    async fn test_process_manager_creation() {
        let config = create_test_config();
        let manager = ProcessManager::new(config).await;
        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_process_manager_state_transitions() {
        let config = create_test_config();
        let mut manager = ProcessManager::new(config).await.unwrap();
        
        // Check initial state
        let state = manager.get_manager_state().await;
        assert!(matches!(state, ProcessManagerState::Initializing));
        
        // Start manager
        manager.start().await.unwrap();
        let state = manager.get_manager_state().await;
        assert!(matches!(state, ProcessManagerState::Running));
        
        // Shutdown manager
        manager.shutdown().await.unwrap();
        let state = manager.get_manager_state().await;
        assert!(matches!(state, ProcessManagerState::Stopped));
    }
}

