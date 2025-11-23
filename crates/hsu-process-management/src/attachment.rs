// Process attachment and reattachment after manager restart
// This module handles discovering and attaching to existing processes

use hsu_common::{ProcessError, ProcessResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

/// Process attachment manager for handling restarts
pub struct ProcessAttachmentManager {
    state_directory: String,
}

/// Saved process state for reattachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProcessState {
    pub process_id: String,
    pub pid: u32,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub state: String,
    pub restart_count: u32,
    pub config_hash: String, // To verify configuration hasn't changed
}

impl ProcessAttachmentManager {
    pub fn new(state_directory: impl Into<String>) -> Self {
        Self {
            state_directory: state_directory.into(),
        }
    }

    /// Save process state to disk for later reattachment
    pub fn save_process_state(&self, state: &SavedProcessState) -> ProcessResult<()> {
        let state_dir = Path::new(&self.state_directory);
        if !state_dir.exists() {
            fs::create_dir_all(state_dir)
                .map_err(|e| ProcessError::Configuration {
                    id: state.process_id.clone(),
                    reason: format!("Failed to create state directory: {}", e),
                })?;
        }

        let state_file = state_dir.join(format!("{}.json", state.process_id));
        let state_json = serde_json::to_string_pretty(state)
            .map_err(|e| ProcessError::Configuration {
                id: state.process_id.clone(),
                reason: format!("Failed to serialize state: {}", e),
            })?;

        fs::write(&state_file, state_json)
            .map_err(|e| ProcessError::Configuration {
                id: state.process_id.clone(),
                reason: format!("Failed to write state file: {}", e),
            })?;

        debug!("Saved process state for: {}", state.process_id);
        Ok(())
    }

    /// Load saved process states from disk
    pub fn load_saved_states(&self) -> ProcessResult<HashMap<String, SavedProcessState>> {
        let state_dir = Path::new(&self.state_directory);
        if !state_dir.exists() {
            debug!("State directory does not exist: {}", self.state_directory);
            return Ok(HashMap::new());
        }

        let mut states = HashMap::new();
        
        let entries = fs::read_dir(state_dir)
            .map_err(|e| ProcessError::Configuration {
                id: "attachment_manager".to_string(),
                reason: format!("Failed to read state directory: {}", e),
            })?;

        for entry in entries {
            let entry = entry.map_err(|e| ProcessError::Configuration {
                id: "attachment_manager".to_string(),
                reason: format!("Failed to read directory entry: {}", e),
            })?;

            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("json")) {
                match self.load_state_file(&path) {
                    Ok(state) => {
                        states.insert(state.process_id.clone(), state);
                    }
                    Err(e) => {
                        warn!("Failed to load state file {:?}: {}", path, e);
                    }
                }
            }
        }

        info!("Loaded {} saved process states", states.len());
        Ok(states)
    }

    /// Load a single state file
    fn load_state_file(&self, path: &Path) -> ProcessResult<SavedProcessState> {
        let content = fs::read_to_string(path)
            .map_err(|e| ProcessError::Configuration {
                id: "attachment_manager".to_string(),
                reason: format!("Failed to read state file {:?}: {}", path, e),
            })?;

        let state: SavedProcessState = serde_json::from_str(&content)
            .map_err(|e| ProcessError::Configuration {
                id: "attachment_manager".to_string(),
                reason: format!("Failed to parse state file {:?}: {}", path, e),
            })?;

        Ok(state)
    }

    /// Remove saved state for a process
    pub fn remove_saved_state(&self, process_id: &str) -> ProcessResult<()> {
        let state_file = Path::new(&self.state_directory).join(format!("{}.json", process_id));
        
        if state_file.exists() {
            fs::remove_file(&state_file)
                .map_err(|e| ProcessError::Configuration {
                    id: process_id.to_string(),
                    reason: format!("Failed to remove state file: {}", e),
                })?;
            
            debug!("Removed saved state for process: {}", process_id);
        }

        Ok(())
    }

    /// Check if a process is still running
    pub fn is_process_running(&self, pid: u32) -> bool {
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            
            match kill(Pid::from_raw(pid as i32), Some(Signal::SIGCONT)) {
                Ok(_) => true,
                Err(nix::errno::Errno::ESRCH) => false, // No such process
                Err(_) => true, // Process exists but we can't signal it (permission issue)
            }
        }

        #[cfg(windows)]
        {
            // TODO: Implement Windows process existence check
            // For now, assume process exists if PID > 0
            pid > 0
        }
    }

    /// Attempt to reattach to existing processes
    pub fn reattach_processes(
        &self,
        saved_states: HashMap<String, SavedProcessState>,
    ) -> HashMap<String, ProcessAttachmentResult> {
        let mut results = HashMap::new();

        for (process_id, saved_state) in saved_states {
            let result = if self.is_process_running(saved_state.pid) {
                info!(
                    "Process {} (PID: {}) is still running, reattaching",
                    process_id, saved_state.pid
                );
                ProcessAttachmentResult::Reattached(saved_state)
            } else {
                info!(
                    "Process {} (PID: {}) is no longer running",
                    process_id, saved_state.pid
                );
                ProcessAttachmentResult::ProcessDied(saved_state)
            };

            results.insert(process_id, result);
        }

        results
    }

    /// Clean up all saved states
    pub fn cleanup_all_states(&self) -> ProcessResult<()> {
        let state_dir = Path::new(&self.state_directory);
        if state_dir.exists() {
            fs::remove_dir_all(state_dir)
                .map_err(|e| ProcessError::Configuration {
                    id: "attachment_manager".to_string(),
                    reason: format!("Failed to remove state directory: {}", e),
                })?;
            
            info!("Cleaned up all saved process states");
        }

        Ok(())
    }
}

/// Result of attempting to reattach to a process
#[derive(Debug, Clone)]
pub enum ProcessAttachmentResult {
    /// Successfully reattached to running process
    Reattached(SavedProcessState),
    /// Process was found to be dead, needs restart
    ProcessDied(SavedProcessState),
    /// Configuration changed, cannot reattach safely
    ConfigurationChanged(SavedProcessState),
    /// Error occurred during reattachment
    Error(String),
}

/// Generate a hash of the process configuration for validation
pub fn generate_config_hash(config: &crate::config::ProcessConfig) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    
    // Hash key configuration elements that would affect process behavior
    config.id.hash(&mut hasher);
    config.process_type.hash(&mut hasher);
    
    // Hash the management config (which already implements Hash correctly)
    config.management.hash(&mut hasher);

    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_save_and_load_state() {
        let temp_dir = tempdir().unwrap();
        let manager = ProcessAttachmentManager::new(temp_dir.path().to_string_lossy().to_string());

        let state = SavedProcessState {
            process_id: "test-process".to_string(),
            pid: 1234,
            start_time: chrono::Utc::now(),
            state: "running".to_string(),
            restart_count: 0,
            config_hash: "abc123".to_string(),
        };

        // Save state
        assert!(manager.save_process_state(&state).is_ok());

        // Load states
        let loaded_states = manager.load_saved_states().unwrap();
        assert_eq!(loaded_states.len(), 1);
        assert!(loaded_states.contains_key("test-process"));
        
        let loaded_state = &loaded_states["test-process"];
        assert_eq!(loaded_state.process_id, state.process_id);
        assert_eq!(loaded_state.pid, state.pid);
        assert_eq!(loaded_state.config_hash, state.config_hash);
    }

    #[test]
    fn test_remove_saved_state() {
        let temp_dir = tempdir().unwrap();
        let manager = ProcessAttachmentManager::new(temp_dir.path().to_string_lossy().to_string());

        let state = SavedProcessState {
            process_id: "test-process".to_string(),
            pid: 1234,
            start_time: chrono::Utc::now(),
            state: "running".to_string(),
            restart_count: 0,
            config_hash: "abc123".to_string(),
        };

        // Save and then remove
        manager.save_process_state(&state).unwrap();
        manager.remove_saved_state("test-process").unwrap();

        // Should be empty now
        let loaded_states = manager.load_saved_states().unwrap();
        assert!(loaded_states.is_empty());
    }

    #[test]
    fn test_config_hash_generation() {
        use crate::config::{
            ProcessConfig, ProcessManagementType, ProcessManagementConfig,
            StandardManagedProcessConfig, ManagedProcessControlConfig, ExecutionConfig,
        };
        use std::collections::HashMap;
        use std::time::Duration;

        let config = ProcessConfig {
            id: "test".to_string(),
            process_type: ProcessManagementType::StandardManaged,
            profile_type: "test".to_string(),
            enabled: true,
            management: ProcessManagementConfig {
                standard_managed: Some(StandardManagedProcessConfig {
                    metadata: None,
                    control: ManagedProcessControlConfig {
                        execution: ExecutionConfig {
                            executable_path: "./test".to_string(),
                            args: vec!["arg1".to_string()],
                            working_directory: None,
                            environment: HashMap::new(),
                            wait_delay: Duration::from_millis(100),
                        },
                        process_file: None,
                        context_aware_restart: None,
                        restart_policy: None,
                        limits: None,
                        graceful_timeout: Some(Duration::from_secs(10)),
                        log_collection: None,
                    },
                    health_check: None,
                }),
                integrated_managed: None,
                unmanaged: None,
            },
        };

        let hash1 = generate_config_hash(&config);
        let hash2 = generate_config_hash(&config);
        
        // Same configuration should produce same hash
        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }
}
