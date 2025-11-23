//! # HSU Resource Limits
//!
//! Resource monitoring and enforcement for the HSU framework.
//!
//! This crate provides:
//! - Resource usage monitoring (CPU, memory, file descriptors)
//! - Resource limit enforcement
//! - Policy-based violation handling
//! - Cross-platform resource management
//!
//! This corresponds to the Go package `pkg/resourcelimits`.

pub mod policy;

use hsu_common::{ProcessError, ProcessResult};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, System};
use std::sync::{Arc, Mutex};
use tracing::debug;

// Re-export policy types
pub use policy::{
    ResourcePolicy, ResourceLimitType, ViolationSeverity, 
    ResourceViolation, ResourceViolationCallback,
};

/// Resource usage information for a process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_percent: Option<f32>,
    pub memory_mb: Option<u64>,
    pub file_descriptors: Option<u32>,
    pub network_connections: Option<u32>,
}

/// Resource limits configuration with policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Memory limits and policy
    pub memory: Option<MemoryLimits>,
    
    /// CPU limits and policy
    pub cpu: Option<CpuLimits>,
    
    /// Process/file descriptor limits and policy
    pub process: Option<ProcessLimits>,
    
    /// Legacy fields (for backward compatibility)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_descriptors: Option<u32>,
}

/// Memory-specific limits with policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLimits {
    /// Maximum RSS (Resident Set Size) in MB
    pub max_rss_mb: u64,
    
    /// Warning threshold (0-100%)
    #[serde(default)]
    pub warning_threshold: Option<f32>,
    
    /// Policy to apply when limit is exceeded
    #[serde(default)]
    pub policy: ResourcePolicy,
}

/// CPU-specific limits with policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuLimits {
    /// Maximum CPU percentage (0-100 per core, so 200% on 2-core system)
    pub max_percent: f32,
    
    /// Warning threshold (0-100%)
    #[serde(default)]
    pub warning_threshold: Option<f32>,
    
    /// Policy to apply when limit is exceeded
    #[serde(default)]
    pub policy: ResourcePolicy,
}

/// Process/file descriptor limits with policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessLimits {
    /// Maximum file descriptors
    pub max_file_descriptors: u32,
    
    /// Warning threshold (0-100%)
    #[serde(default)]
    pub warning_threshold: Option<f32>,
    
    /// Policy to apply when limit is exceeded
    #[serde(default)]
    pub policy: ResourcePolicy,
}

/// Resource monitor that tracks process resource usage
pub struct ResourceMonitor {
    system: Arc<Mutex<System>>,
}

impl ResourceMonitor {
    /// Create a new resource monitor
    pub fn new() -> Self {
        Self {
            system: Arc::new(Mutex::new(System::new_all())),
        }
    }
    
    /// Get current resource usage for a process
    pub fn get_process_resource_usage(&self, pid: u32) -> ProcessResult<ResourceUsage> {
        let mut system = self.system.lock().unwrap();
        
        // Refresh process information - IMPORTANT: Must specify what to refresh!
        // Without this, sysinfo returns stale/zero data
        let sysinfo_pid = Pid::from_u32(pid);
        system.refresh_process_specifics(
            sysinfo_pid, 
            ProcessRefreshKind::new()
                .with_memory()      // Refresh memory stats
                .with_cpu()         // Refresh CPU stats
        );
        
        // Get process information
        let process = system.process(sysinfo_pid)
            .ok_or_else(|| ProcessError::NotFound { 
                id: format!("Process with PID {} not found", pid) 
            })?;
        
        // Get CPU usage (percentage)
        let cpu_percent = process.cpu_usage();
        
        // Get memory usage (convert from bytes to MB)
        let memory_bytes = process.memory();
        let memory_mb = (memory_bytes as f64 / 1024.0 / 1024.0) as u64;
        
        // Get file descriptor count (Unix-specific)
        #[cfg(unix)]
        let file_descriptors = get_process_fd_count(pid);
        
        #[cfg(not(unix))]
        let file_descriptors = None;
        
        debug!(
            "Resource usage for PID {}: CPU={:.1}%, Memory={} MB, FDs={:?}",
            pid, cpu_percent, memory_mb, file_descriptors
        );
        
        Ok(ResourceUsage {
            cpu_percent: Some(cpu_percent),
            memory_mb: Some(memory_mb),
            file_descriptors,
            network_connections: None, // TODO: Implement network connection counting
        })
    }
    
    /// Refresh system information (should be called periodically)
    pub fn refresh(&self) {
        let mut system = self.system.lock().unwrap();
        system.refresh_all();
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current resource usage for a process (convenience function)
pub fn get_process_resource_usage(pid: u32) -> ProcessResult<ResourceUsage> {
    let monitor = ResourceMonitor::new();
    monitor.get_process_resource_usage(pid)
}

/// Get file descriptor count for a process (Unix-specific)
#[cfg(unix)]
fn get_process_fd_count(pid: u32) -> Option<u32> {
    use std::fs;
    
    // On Linux, read /proc/[pid]/fd directory
    let fd_path = format!("/proc/{}/fd", pid);
    
    match fs::read_dir(&fd_path) {
        Ok(entries) => {
            let count = entries.count() as u32;
            Some(count)
        }
        Err(_) => {
            // Failed to read FD directory (process may not exist or access denied)
            None
        }
    }
}

/// Get file descriptor count for a process (non-Unix stub)
#[cfg(not(unix))]
#[allow(dead_code)]
fn get_process_fd_count(_pid: u32) -> Option<u32> {
    None
}

/// Check if resource usage exceeds limits and return violations with policies.
///
/// This function checks both new structured limits and legacy limits for backward compatibility.
pub fn check_resource_limits_with_policy(
    process_id: &str,
    usage: &ResourceUsage,
    limits: &ResourceLimits,
) -> Vec<(ResourceViolation, ResourcePolicy)> {
    let mut violations = Vec::new();

    // Check memory limits (new structured format)
    if let Some(mem_limits) = &limits.memory {
        if let Some(current_mb) = usage.memory_mb {
            let limit_mb = mem_limits.max_rss_mb;
            
            // Check warning threshold
            if let Some(warning_threshold) = mem_limits.warning_threshold {
                let percent_used = (current_mb as f32 / limit_mb as f32) * 100.0;
                if percent_used >= warning_threshold && percent_used < 100.0 {
                    let violation = ResourceViolation::memory(
                        process_id.to_string(),
                        current_mb,
                        limit_mb,
                        ViolationSeverity::Warning,
                    );
                    violations.push((violation, mem_limits.policy));
                }
            }
            
            // Check hard limit
            if current_mb > limit_mb {
                let violation = ResourceViolation::memory(
                    process_id.to_string(),
                    current_mb,
                    limit_mb,
                    ViolationSeverity::Critical,
                );
                violations.push((violation, mem_limits.policy));
            }
        }
    }
    // Fallback to legacy memory limit
    else if let (Some(current_mb), Some(limit_mb)) = (usage.memory_mb, limits.max_memory_mb) {
        if current_mb > limit_mb {
            let violation = ResourceViolation::memory(
                process_id.to_string(),
                current_mb,
                limit_mb,
                ViolationSeverity::Critical,
            );
            violations.push((violation, ResourcePolicy::Log));
        }
    }

    // Check CPU limits (new structured format)
    if let Some(cpu_limits) = &limits.cpu {
        if let Some(current_percent) = usage.cpu_percent {
            let limit_percent = cpu_limits.max_percent;
            
            // Check warning threshold
            if let Some(warning_threshold) = cpu_limits.warning_threshold {
                let percent_used = (current_percent / limit_percent) * 100.0;
                if percent_used >= warning_threshold && percent_used < 100.0 {
                    let violation = ResourceViolation::cpu(
                        process_id.to_string(),
                        current_percent,
                        limit_percent,
                        ViolationSeverity::Warning,
                    );
                    violations.push((violation, cpu_limits.policy));
                }
            }
            
            // Check hard limit
            if current_percent > limit_percent {
                let violation = ResourceViolation::cpu(
                    process_id.to_string(),
                    current_percent,
                    limit_percent,
                    ViolationSeverity::Critical,
                );
                violations.push((violation, cpu_limits.policy));
            }
        }
    }
    // Fallback to legacy CPU limit
    else if let (Some(current_percent), Some(limit_percent)) = (usage.cpu_percent, limits.max_cpu_percent) {
        if current_percent > limit_percent {
            let violation = ResourceViolation::cpu(
                process_id.to_string(),
                current_percent,
                limit_percent,
                ViolationSeverity::Critical,
            );
            violations.push((violation, ResourcePolicy::Log));
        }
    }

    // Check process/FD limits (new structured format)
    if let Some(proc_limits) = &limits.process {
        if let Some(current_fds) = usage.file_descriptors {
            let limit_fds = proc_limits.max_file_descriptors;
            
            // Check warning threshold
            if let Some(warning_threshold) = proc_limits.warning_threshold {
                let percent_used = (current_fds as f32 / limit_fds as f32) * 100.0;
                if percent_used >= warning_threshold && percent_used < 100.0 {
                    let violation = ResourceViolation::file_descriptors(
                        process_id.to_string(),
                        current_fds,
                        limit_fds,
                        ViolationSeverity::Warning,
                    );
                    violations.push((violation, proc_limits.policy));
                }
            }
            
            // Check hard limit
            if current_fds > limit_fds {
                let violation = ResourceViolation::file_descriptors(
                    process_id.to_string(),
                    current_fds,
                    limit_fds,
                    ViolationSeverity::Critical,
                );
                violations.push((violation, proc_limits.policy));
            }
        }
    }
    // Fallback to legacy FD limit
    else if let (Some(current_fds), Some(limit_fds)) = (usage.file_descriptors, limits.max_file_descriptors) {
        if current_fds > limit_fds {
            let violation = ResourceViolation::file_descriptors(
                process_id.to_string(),
                current_fds,
                limit_fds,
                ViolationSeverity::Critical,
            );
            violations.push((violation, ResourcePolicy::Log));
        }
    }

    violations
}

/// Legacy function for backward compatibility
/// 
/// Returns simple string violations without policy information.
pub fn check_resource_limits(
    usage: &ResourceUsage,
    limits: &ResourceLimits,
) -> Result<(), Vec<String>> {
    let violations = check_resource_limits_with_policy("unknown", usage, limits);
    
    if violations.is_empty() {
        Ok(())
    } else {
        let messages: Vec<String> = violations
            .iter()
            .map(|(v, _)| v.message.clone())
            .collect();
        Err(messages)
    }
}

