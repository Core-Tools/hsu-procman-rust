//! Unmanaged Process
//!
//! An UnmanagedProcess provides monitoring for externally-managed processes:
//! - Attaches to existing process (via PID file)
//! - Monitors health and resources
//! - Does NOT spawn new processes
//! - Does NOT terminate the process on stop (only detaches)
//! - Does NOT restart the process
//!
//! This is ideal for system services or processes managed by other tools.

use crate::process_control::{ProcessControl, ProcessControlConfig, ProcessDiagnostics};
use async_trait::async_trait;
use hsu_common::ProcessResult;
use hsu_process_state::ProcessState;

/// Unmanaged process with monitoring-only capabilities
pub struct UnmanagedProcess {
    /// Internal process control implementation
    inner: Box<dyn ProcessControl>,
}

impl UnmanagedProcess {
    /// Create a new UnmanagedProcess
    ///
    /// This process type:
    /// - Attaches to existing process (doesn't spawn)
    /// - Monitors health and resources
    /// - Does NOT restart on failure
    /// - Detaches on stop (doesn't terminate)
    pub fn new(inner: Box<dyn ProcessControl>) -> Self {
        Self { inner }
    }
    
    /// Create configuration for an unmanaged process
    pub fn create_config(process_id: String, graceful_timeout: std::time::Duration) -> ProcessControlConfig {
        ProcessControlConfig {
            process_id,
            can_attach: true,       // Attaches to existing process
            can_terminate: false,    // Does NOT terminate external process
            can_restart: false,      // Does NOT restart
            graceful_timeout,
            process_profile_type: "unmanaged".to_string(),
            log_collection_service: None,  // Set later if needed
            log_config: None,              // Set later if needed
        }
    }
}

#[async_trait]
impl ProcessControl for UnmanagedProcess {
    async fn start(&mut self) -> ProcessResult<()> {
        // For unmanaged processes, "start" means "attach to existing"
        self.inner.start().await
    }

    async fn stop(&mut self) -> ProcessResult<()> {
        // For unmanaged processes, "stop" means "detach" (doesn't kill)
        self.inner.stop().await
    }

    async fn restart(&mut self, _force: bool) -> ProcessResult<()> {
        // Unmanaged processes cannot be restarted
        // This should typically not be called, but if it is, just return an error
        Err(hsu_common::ProcessError::Configuration {
            id: "unmanaged".to_string(),
            reason: "Unmanaged processes cannot be restarted by the process manager".to_string(),
        })
    }

    fn get_state(&self) -> ProcessState {
        self.inner.get_state()
    }

    fn get_diagnostics(&self) -> ProcessDiagnostics {
        self.inner.get_diagnostics()
    }

    fn get_pid(&self) -> Option<u32> {
        self.inner.get_pid()
    }

    fn is_healthy(&self) -> bool {
        self.inner.is_healthy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unmanaged_config() {
        let config = UnmanagedProcess::create_config(
            "external-service".to_string(),
            std::time::Duration::from_secs(5),
        );
        
        assert_eq!(config.process_id, "external-service");
        assert_eq!(config.can_attach, true);
        assert_eq!(config.can_terminate, false);  // Key difference!
        assert_eq!(config.can_restart, false);     // Key difference!
        assert_eq!(config.process_profile_type, "unmanaged");
    }
}

