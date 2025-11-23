//! Standard Managed Process
//!
//! A StandardManagedProcess has full lifecycle control:
//! - Spawns new process
//! - Monitors health (HTTP/gRPC)
//! - Enforces resource limits
//! - Restarts on failure
//! - Terminates on shutdown
//!
//! This is the default process type for most services.

use crate::process_control::{ProcessControl, ProcessControlConfig, ProcessDiagnostics};
use async_trait::async_trait;
use hsu_common::ProcessResult;
use hsu_process_state::ProcessState;

/// Standard managed process with full lifecycle control
pub struct StandardManagedProcess {
    /// Internal process control implementation
    inner: Box<dyn ProcessControl>,
}

impl StandardManagedProcess {
    /// Create a new StandardManagedProcess
    ///
    /// This process type:
    /// - Spawns a new child process
    /// - Monitors health and resources
    /// - Restarts on failure (according to policy)
    /// - Terminates gracefully on stop
    pub fn new(inner: Box<dyn ProcessControl>) -> Self {
        Self { inner }
    }
    
    /// Create configuration for a standard managed process
    pub fn create_config(process_id: String, graceful_timeout: std::time::Duration) -> ProcessControlConfig {
        ProcessControlConfig {
            process_id,
            can_attach: false,      // Always spawns new process
            can_terminate: true,     // Can terminate
            can_restart: true,       // Can restart
            graceful_timeout,
            process_profile_type: "standard".to_string(),
            log_collection_service: None,  // Set later if needed
            log_config: None,              // Set later if needed
        }
    }
}

#[async_trait]
impl ProcessControl for StandardManagedProcess {
    async fn start(&mut self) -> ProcessResult<()> {
        self.inner.start().await
    }

    async fn stop(&mut self) -> ProcessResult<()> {
        self.inner.stop().await
    }

    async fn restart(&mut self, force: bool) -> ProcessResult<()> {
        self.inner.restart(force).await
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
    fn test_standard_managed_config() {
        let config = StandardManagedProcess::create_config(
            "test-process".to_string(),
            std::time::Duration::from_secs(10),
        );
        
        assert_eq!(config.process_id, "test-process");
        assert_eq!(config.can_attach, false);
        assert_eq!(config.can_terminate, true);
        assert_eq!(config.can_restart, true);
    }
}

