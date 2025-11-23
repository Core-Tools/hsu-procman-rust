//! Resource limit policies and violation handling

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Policy to apply when resource limits are violated
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePolicy {
    /// No action taken
    None,
    
    /// Log the violation only
    Log,
    
    /// Send an alert/notification
    Alert,
    
    /// Throttle the process (suspend/resume)
    Throttle,
    
    /// Graceful shutdown (SIGTERM then SIGKILL)
    GracefulShutdown,
    
    /// Immediate kill (SIGKILL)
    ImmediateKill,
    
    /// Restart process with same limits
    Restart,
    
    /// Restart process with adjusted (increased) limits
    RestartAdjusted,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        ResourcePolicy::Log
    }
}

impl fmt::Display for ResourcePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourcePolicy::None => write!(f, "none"),
            ResourcePolicy::Log => write!(f, "log"),
            ResourcePolicy::Alert => write!(f, "alert"),
            ResourcePolicy::Throttle => write!(f, "throttle"),
            ResourcePolicy::GracefulShutdown => write!(f, "graceful_shutdown"),
            ResourcePolicy::ImmediateKill => write!(f, "immediate_kill"),
            ResourcePolicy::Restart => write!(f, "restart"),
            ResourcePolicy::RestartAdjusted => write!(f, "restart_adjusted"),
        }
    }
}

/// Type of resource limit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceLimitType {
    Memory,
    Cpu,
    Io,
    Network,
    Process,
}

impl fmt::Display for ResourceLimitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceLimitType::Memory => write!(f, "memory"),
            ResourceLimitType::Cpu => write!(f, "cpu"),
            ResourceLimitType::Io => write!(f, "io"),
            ResourceLimitType::Network => write!(f, "network"),
            ResourceLimitType::Process => write!(f, "process"),
        }
    }
}

/// Severity of a resource limit violation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViolationSeverity {
    Warning,
    Critical,
}

impl fmt::Display for ViolationSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViolationSeverity::Warning => write!(f, "warning"),
            ViolationSeverity::Critical => write!(f, "critical"),
        }
    }
}

/// Resource limit violation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceViolation {
    /// Type of limit that was violated
    pub limit_type: ResourceLimitType,
    
    /// Current resource value (varies by type)
    pub current_value: f64,
    
    /// Limit value that was exceeded
    pub limit_value: f64,
    
    /// Severity of the violation
    pub severity: ViolationSeverity,
    
    /// When the violation occurred
    pub timestamp: DateTime<Utc>,
    
    /// Human-readable message
    pub message: String,
    
    /// Process ID where violation occurred
    pub process_id: String,
}

impl ResourceViolation {
    /// Create a new resource violation
    pub fn new(
        limit_type: ResourceLimitType,
        process_id: String,
        current_value: f64,
        limit_value: f64,
        severity: ViolationSeverity,
        message: String,
    ) -> Self {
        Self {
            limit_type,
            current_value,
            limit_value,
            severity,
            timestamp: Utc::now(),
            message,
            process_id,
        }
    }
    
    /// Create a memory violation
    pub fn memory(process_id: String, current_mb: u64, limit_mb: u64, severity: ViolationSeverity) -> Self {
        Self::new(
            ResourceLimitType::Memory,
            process_id,
            current_mb as f64,
            limit_mb as f64,
            severity,
            format!("Memory usage {} MB exceeds limit {} MB", current_mb, limit_mb),
        )
    }
    
    /// Create a CPU violation
    pub fn cpu(process_id: String, current_percent: f32, limit_percent: f32, severity: ViolationSeverity) -> Self {
        Self::new(
            ResourceLimitType::Cpu,
            process_id,
            current_percent as f64,
            limit_percent as f64,
            severity,
            format!("CPU usage {:.1}% exceeds limit {:.1}%", current_percent, limit_percent),
        )
    }
    
    /// Create a file descriptor violation
    pub fn file_descriptors(process_id: String, current_fds: u32, limit_fds: u32, severity: ViolationSeverity) -> Self {
        Self::new(
            ResourceLimitType::Process,
            process_id,
            current_fds as f64,
            limit_fds as f64,
            severity,
            format!("File descriptors {} exceeds limit {}", current_fds, limit_fds),
        )
    }
}

impl fmt::Display for ResourceViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} violation for {}: {} (current: {}, limit: {})",
            self.severity, self.limit_type, self.process_id, self.message, self.current_value, self.limit_value
        )
    }
}

/// Callback function type for resource violations
///
/// Called when a resource limit is violated, providing the policy to apply
/// and details about the violation.
pub type ResourceViolationCallback = Box<dyn Fn(ResourcePolicy, &ResourceViolation) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_display() {
        assert_eq!(ResourcePolicy::Log.to_string(), "log");
        assert_eq!(ResourcePolicy::Restart.to_string(), "restart");
        assert_eq!(ResourcePolicy::GracefulShutdown.to_string(), "graceful_shutdown");
    }

    #[test]
    fn test_violation_creation() {
        let violation = ResourceViolation::memory("test-proc".to_string(), 1024, 512, ViolationSeverity::Critical);
        assert_eq!(violation.limit_type, ResourceLimitType::Memory);
        assert_eq!(violation.current_value, 1024.0);
        assert_eq!(violation.limit_value, 512.0);
        assert_eq!(violation.severity, ViolationSeverity::Critical);
    }

    #[test]
    fn test_violation_display() {
        let violation = ResourceViolation::cpu("test-proc".to_string(), 85.5, 70.0, ViolationSeverity::Warning);
        let display = format!("{}", violation);
        assert!(display.contains("warning"));
        assert!(display.contains("cpu"));
        assert!(display.contains("85.5"));
    }
}

