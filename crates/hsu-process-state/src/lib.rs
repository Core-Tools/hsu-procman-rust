use hsu_common::errors::{ProcessError, ProcessResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Process state enumeration matching the Go implementation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessState {
    /// Process is not yet started
    Stopped,
    /// Process is in the process of starting
    Starting,
    /// Process is running normally
    Running,
    /// Process is in the process of stopping
    Stopping,
    /// Process has failed and cannot be restarted automatically
    Failed,
    /// Process is scheduled for restart
    PendingRestart,
    /// Process is disabled and should not be started
    Disabled,
}

impl fmt::Display for ProcessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessState::Stopped => write!(f, "stopped"),
            ProcessState::Starting => write!(f, "starting"),
            ProcessState::Running => write!(f, "running"),
            ProcessState::Stopping => write!(f, "stopping"),
            ProcessState::Failed => write!(f, "failed"),
            ProcessState::PendingRestart => write!(f, "pending_restart"),
            ProcessState::Disabled => write!(f, "disabled"),
        }
    }
}

impl ProcessState {
    /// Check if the process is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, ProcessState::Stopped | ProcessState::Failed | ProcessState::Disabled)
    }

    /// Check if the process is in a transitional state
    pub fn is_transitional(&self) -> bool {
        matches!(self, ProcessState::Starting | ProcessState::Stopping | ProcessState::PendingRestart)
    }

    /// Check if the process is active (running or transitional)
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

/// Process state machine that manages transitions between states
#[derive(Debug, Clone)]
pub struct ProcessStateMachine {
    process_id: String,
    current_state: ProcessState,
    previous_state: Option<ProcessState>,
    state_history: Vec<StateTransition>,
    last_transition_time: DateTime<Utc>,
}

/// Represents a state transition with timestamp and optional reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    pub from_state: ProcessState,
    pub to_state: ProcessState,
    pub timestamp: DateTime<Utc>,
    pub reason: Option<String>,
}

impl ProcessStateMachine {
    /// Create a new state machine for a process
    pub fn new(process_id: &str) -> Self {
        let now = Utc::now();
        Self {
            process_id: process_id.to_string(),
            current_state: ProcessState::Stopped,
            previous_state: None,
            state_history: Vec::new(),
            last_transition_time: now,
        }
    }

    /// Get the current state
    pub fn current_state(&self) -> ProcessState {
        self.current_state
    }

    /// Get the previous state
    pub fn previous_state(&self) -> Option<ProcessState> {
        self.previous_state
    }

    /// Get the state history
    pub fn state_history(&self) -> &[StateTransition] {
        &self.state_history
    }

    /// Get the time of the last state transition
    pub fn last_transition_time(&self) -> DateTime<Utc> {
        self.last_transition_time
    }

    /// Check if a transition from current state to target state is valid
    pub fn is_valid_transition(&self, target_state: ProcessState) -> bool {
        match (self.current_state, target_state) {
            // From Stopped
            (ProcessState::Stopped, ProcessState::Starting) => true,
            (ProcessState::Stopped, ProcessState::Disabled) => true,
            
            // From Starting
            (ProcessState::Starting, ProcessState::Running) => true,
            (ProcessState::Starting, ProcessState::Failed) => true,
            (ProcessState::Starting, ProcessState::Stopping) => true, // Cancel startup
            
            // From Running
            (ProcessState::Running, ProcessState::Stopping) => true,
            (ProcessState::Running, ProcessState::Failed) => true,
            (ProcessState::Running, ProcessState::PendingRestart) => true,
            
            // From Stopping
            (ProcessState::Stopping, ProcessState::Stopped) => true,
            (ProcessState::Stopping, ProcessState::Failed) => true,
            
            // From Failed
            (ProcessState::Failed, ProcessState::Starting) => true, // Manual restart
            (ProcessState::Failed, ProcessState::Disabled) => true,
            (ProcessState::Failed, ProcessState::PendingRestart) => true,
            
            // From PendingRestart
            (ProcessState::PendingRestart, ProcessState::Starting) => true,
            (ProcessState::PendingRestart, ProcessState::Failed) => true,
            (ProcessState::PendingRestart, ProcessState::Disabled) => true,
            
            // From Disabled
            (ProcessState::Disabled, ProcessState::Stopped) => true,
            
            // Same state (no-op)
            (state, target) if state == target => true,
            
            // All other transitions are invalid
            _ => false,
        }
    }

    /// Transition to a new state with optional reason
    pub fn transition_to(&mut self, target_state: ProcessState, reason: Option<String>) -> ProcessResult<()> {
        if !self.is_valid_transition(target_state) {
            return Err(ProcessError::invalid_state(
                &self.process_id,
                format!("{:?}", target_state),
                format!("{:?}", self.current_state),
            ));
        }

        let now = Utc::now();
        let transition = StateTransition {
            from_state: self.current_state,
            to_state: target_state,
            timestamp: now,
            reason,
        };

        // Update state
        self.previous_state = Some(self.current_state);
        self.current_state = target_state;
        self.last_transition_time = now;
        self.state_history.push(transition);

        // Limit history size to prevent unbounded growth
        if self.state_history.len() > 100 {
            self.state_history.remove(0);
        }

        tracing::debug!(
            "Process {} transitioned from {:?} to {:?}",
            self.process_id,
            self.previous_state.unwrap(),
            self.current_state
        );

        Ok(())
    }

    /// Convenience methods for specific transitions
    pub fn transition_to_starting(&mut self) -> ProcessResult<()> {
        self.transition_to(ProcessState::Starting, Some("Process start requested".to_string()))
    }

    pub fn transition_to_running(&mut self) -> ProcessResult<()> {
        self.transition_to(ProcessState::Running, Some("Process started successfully".to_string()))
    }

    pub fn transition_to_stopping(&mut self) -> ProcessResult<()> {
        self.transition_to(ProcessState::Stopping, Some("Process stop requested".to_string()))
    }

    pub fn transition_to_stopped(&mut self) -> ProcessResult<()> {
        self.transition_to(ProcessState::Stopped, Some("Process stopped successfully".to_string()))
    }

    pub fn transition_to_failed(&mut self, reason: String) -> ProcessResult<()> {
        self.transition_to(ProcessState::Failed, Some(reason))
    }

    pub fn transition_to_pending_restart(&mut self, reason: String) -> ProcessResult<()> {
        self.transition_to(ProcessState::PendingRestart, Some(reason))
    }

    pub fn transition_to_disabled(&mut self) -> ProcessResult<()> {
        self.transition_to(ProcessState::Disabled, Some("Process disabled".to_string()))
    }

    /// Check if the process can be started
    pub fn can_start(&self) -> bool {
        matches!(
            self.current_state,
            ProcessState::Stopped | ProcessState::Failed | ProcessState::PendingRestart
        )
    }

    /// Check if the process can be stopped
    pub fn can_stop(&self) -> bool {
        matches!(
            self.current_state,
            ProcessState::Running | ProcessState::Starting
        )
    }

    /// Check if the process can be restarted
    pub fn can_restart(&self) -> bool {
        !matches!(self.current_state, ProcessState::Disabled)
    }

    /// Get the time spent in the current state
    pub fn time_in_current_state(&self) -> chrono::Duration {
        Utc::now() - self.last_transition_time
    }

    /// Get the most recent transition
    pub fn last_transition(&self) -> Option<&StateTransition> {
        self.state_history.last()
    }

    /// Count transitions to a specific state
    pub fn count_transitions_to(&self, state: ProcessState) -> usize {
        self.state_history
            .iter()
            .filter(|t| t.to_state == state)
            .count()
    }

    /// Get transitions within a time range
    pub fn transitions_since(&self, since: DateTime<Utc>) -> Vec<&StateTransition> {
        self.state_history
            .iter()
            .filter(|t| t.timestamp >= since)
            .collect()
    }

    /// Reset the state machine to initial state
    pub fn reset(&mut self) {
        self.current_state = ProcessState::Stopped;
        self.previous_state = None;
        self.state_history.clear();
        self.last_transition_time = Utc::now();
    }

    /// Create a state machine from a saved state (for process attachment)
    pub fn from_saved_state(
        process_id: &str,
        current_state: ProcessState,
        last_transition_time: DateTime<Utc>,
    ) -> Self {
        Self {
            process_id: process_id.to_string(),
            current_state,
            previous_state: None,
            state_history: Vec::new(),
            last_transition_time,
        }
    }
}

/// State machine statistics for monitoring and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStatistics {
    pub total_transitions: usize,
    pub time_in_current_state: chrono::Duration,
    pub failure_count: usize,
    pub restart_count: usize,
    pub uptime_percentage: f64,
    pub last_failure_time: Option<DateTime<Utc>>,
    pub average_uptime: Option<chrono::Duration>,
}

impl ProcessStateMachine {
    /// Calculate statistics for the state machine
    pub fn calculate_statistics(&self, start_time: DateTime<Utc>) -> StateStatistics {
        let total_time = Utc::now() - start_time;
        let total_transitions = self.state_history.len();
        let time_in_current_state = self.time_in_current_state();

        let failure_count = self.count_transitions_to(ProcessState::Failed);
        let restart_count = self.count_transitions_to(ProcessState::Starting);

        // Calculate uptime percentage
        let mut running_time = chrono::Duration::zero();
        let mut last_start: Option<DateTime<Utc>> = None;

        for transition in &self.state_history {
            match transition.to_state {
                ProcessState::Running => {
                    last_start = Some(transition.timestamp);
                }
                ProcessState::Stopping | ProcessState::Stopped | ProcessState::Failed => {
                    if let Some(start_time) = last_start {
                        running_time = running_time + (transition.timestamp - start_time);
                        last_start = None;
                    }
                }
                _ => {}
            }
        }

        // Add current running time if still running
        if self.current_state == ProcessState::Running {
            if let Some(start_time) = last_start {
                running_time = running_time + (Utc::now() - start_time);
            }
        }

        let uptime_percentage = if total_time.num_milliseconds() > 0 {
            (running_time.num_milliseconds() as f64 / total_time.num_milliseconds() as f64) * 100.0
        } else {
            0.0
        };

        let last_failure_time = self.state_history
            .iter()
            .filter(|t| t.to_state == ProcessState::Failed)
            .last()
            .map(|t| t.timestamp);

        let average_uptime = if restart_count > 0 {
            Some(running_time / restart_count as i32)
        } else {
            None
        };

        StateStatistics {
            total_transitions,
            time_in_current_state,
            failure_count,
            restart_count,
            uptime_percentage,
            last_failure_time,
            average_uptime,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_creation() {
        let sm = ProcessStateMachine::new("test-process");
        assert_eq!(sm.current_state(), ProcessState::Stopped);
        assert_eq!(sm.previous_state(), None);
        assert_eq!(sm.state_history().len(), 0);
    }

    #[test]
    fn test_valid_transitions() {
        let mut sm = ProcessStateMachine::new("test-process");
        
        // Stopped -> Starting
        assert!(sm.is_valid_transition(ProcessState::Starting));
        assert!(sm.transition_to_starting().is_ok());
        assert_eq!(sm.current_state(), ProcessState::Starting);
        
        // Starting -> Running
        assert!(sm.is_valid_transition(ProcessState::Running));
        assert!(sm.transition_to_running().is_ok());
        assert_eq!(sm.current_state(), ProcessState::Running);
        
        // Running -> Stopping
        assert!(sm.is_valid_transition(ProcessState::Stopping));
        assert!(sm.transition_to_stopping().is_ok());
        assert_eq!(sm.current_state(), ProcessState::Stopping);
        
        // Stopping -> Stopped
        assert!(sm.is_valid_transition(ProcessState::Stopped));
        assert!(sm.transition_to_stopped().is_ok());
        assert_eq!(sm.current_state(), ProcessState::Stopped);
    }

    #[test]
    fn test_invalid_transitions() {
        let mut sm = ProcessStateMachine::new("test-process");
        
        // Stopped -> Running (invalid, must go through Starting)
        assert!(!sm.is_valid_transition(ProcessState::Running));
        assert!(sm.transition_to(ProcessState::Running, None).is_err());
        
        // Stopped -> Stopping (invalid)
        assert!(!sm.is_valid_transition(ProcessState::Stopping));
        assert!(sm.transition_to(ProcessState::Stopping, None).is_err());
    }

    #[test]
    fn test_state_properties() {
        assert!(ProcessState::Stopped.is_terminal());
        assert!(ProcessState::Failed.is_terminal());
        assert!(ProcessState::Disabled.is_terminal());
        
        assert!(ProcessState::Starting.is_transitional());
        assert!(ProcessState::Stopping.is_transitional());
        assert!(ProcessState::PendingRestart.is_transitional());
        
        assert!(ProcessState::Running.is_active());
        assert!(ProcessState::Starting.is_active());
        assert!(!ProcessState::Stopped.is_active());
    }

    #[test]
    fn test_state_history() {
        let mut sm = ProcessStateMachine::new("test-process");
        
        sm.transition_to_starting().unwrap();
        sm.transition_to_running().unwrap();
        sm.transition_to_stopping().unwrap();
        sm.transition_to_stopped().unwrap();
        
        assert_eq!(sm.state_history().len(), 4);
        assert_eq!(sm.state_history()[0].from_state, ProcessState::Stopped);
        assert_eq!(sm.state_history()[0].to_state, ProcessState::Starting);
        assert_eq!(sm.state_history()[3].from_state, ProcessState::Stopping);
        assert_eq!(sm.state_history()[3].to_state, ProcessState::Stopped);
    }

    #[test]
    fn test_can_operations() {
        let mut sm = ProcessStateMachine::new("test-process");
        
        // Initially stopped, can start but not stop
        assert!(sm.can_start());
        assert!(!sm.can_stop());
        assert!(sm.can_restart());
        
        // While running, can stop but not start
        sm.transition_to_starting().unwrap();
        sm.transition_to_running().unwrap();
        assert!(!sm.can_start());
        assert!(sm.can_stop());
        assert!(sm.can_restart());
        
        // When disabled, cannot restart
        sm.transition_to_stopping().unwrap();
        sm.transition_to_stopped().unwrap();
        sm.transition_to_disabled().unwrap();
        assert!(!sm.can_start());
        assert!(!sm.can_stop());
        assert!(!sm.can_restart());
    }
}
