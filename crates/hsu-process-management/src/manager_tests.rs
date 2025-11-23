//! Trait-based mock tests for ProcessManager
//!
//! These tests demonstrate the testability improvements from the refactoring.

#[cfg(test)]
mod tests {
    use hsu_managed_process::{ProcessControl, ProcessDiagnostics};
    use hsu_process_state::ProcessState;
    use hsu_common::ProcessResult;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// Mock ProcessControl for testing
    struct MockProcessControl {
        state: Arc<Mutex<ProcessState>>,
        start_called: Arc<Mutex<bool>>,
        stop_called: Arc<Mutex<bool>>,
        restart_called: Arc<Mutex<bool>>,
    }

    impl MockProcessControl {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(ProcessState::Stopped)),
                start_called: Arc::new(Mutex::new(false)),
                stop_called: Arc::new(Mutex::new(false)),
                restart_called: Arc::new(Mutex::new(false)),
            }
        }
        
        fn was_start_called(&self) -> bool {
            *self.start_called.lock().unwrap()
        }
        
        fn was_stop_called(&self) -> bool {
            *self.stop_called.lock().unwrap()
        }
        
        fn was_restart_called(&self) -> bool {
            *self.restart_called.lock().unwrap()
        }
    }

    #[async_trait]
    impl ProcessControl for MockProcessControl {
        async fn start(&mut self) -> ProcessResult<()> {
            *self.start_called.lock().unwrap() = true;
            *self.state.lock().unwrap() = ProcessState::Running;
            Ok(())
        }

        async fn stop(&mut self) -> ProcessResult<()> {
            *self.stop_called.lock().unwrap() = true;
            *self.state.lock().unwrap() = ProcessState::Stopped;
            Ok(())
        }

        async fn restart(&mut self, _force: bool) -> ProcessResult<()> {
            *self.restart_called.lock().unwrap() = true;
            *self.state.lock().unwrap() = ProcessState::Running;
            Ok(())
        }

        fn get_state(&self) -> ProcessState {
            *self.state.lock().unwrap()
        }

        fn get_diagnostics(&self) -> ProcessDiagnostics {
            ProcessDiagnostics {
                state: self.get_state(),
                last_error: None,
                process_id: Some(12345),
                start_time: Some(chrono::Utc::now()),
                executable_path: "/usr/bin/test".to_string(),
                executable_exists: true,
                failure_count: 0,
                last_attempt_time: Some(chrono::Utc::now()),
                is_healthy: true,
                cpu_usage: Some(5.0),
                memory_usage: Some(100),
            }
        }

        fn get_pid(&self) -> Option<u32> {
            if matches!(self.get_state(), ProcessState::Running) {
                Some(12345)
            } else {
                None
            }
        }

        fn is_healthy(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn test_mock_process_control_start() {
        let mut mock = MockProcessControl::new();
        
        assert_eq!(mock.get_state(), ProcessState::Stopped);
        assert!(!mock.was_start_called());
        
        mock.start().await.unwrap();
        
        assert!(mock.was_start_called());
        assert_eq!(mock.get_state(), ProcessState::Running);
        assert_eq!(mock.get_pid(), Some(12345));
    }

    #[tokio::test]
    async fn test_mock_process_control_stop() {
        let mut mock = MockProcessControl::new();
        
        mock.start().await.unwrap();
        assert_eq!(mock.get_state(), ProcessState::Running);
        
        mock.stop().await.unwrap();
        
        assert!(mock.was_stop_called());
        assert_eq!(mock.get_state(), ProcessState::Stopped);
        assert_eq!(mock.get_pid(), None);
    }

    #[tokio::test]
    async fn test_mock_process_control_restart() {
        let mut mock = MockProcessControl::new();
        
        mock.start().await.unwrap();
        assert!(!mock.was_restart_called());
        
        mock.restart(false).await.unwrap();
        
        assert!(mock.was_restart_called());
        assert_eq!(mock.get_state(), ProcessState::Running);
    }

    #[tokio::test]
    async fn test_mock_process_diagnostics() {
        let mock = MockProcessControl::new();
        
        let diagnostics = mock.get_diagnostics();
        
        assert_eq!(diagnostics.state, ProcessState::Stopped);
        assert_eq!(diagnostics.process_id, Some(12345));
        assert!(diagnostics.is_healthy);
        assert_eq!(diagnostics.cpu_usage, Some(5.0));
        assert_eq!(diagnostics.memory_usage, Some(100));
    }
}

