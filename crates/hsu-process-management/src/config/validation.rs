use super::*;
use anyhow::{anyhow, Result};
use std::collections::HashSet;

/// Validate the complete configuration
pub fn validate_config(config: &ProcessManagerConfig) -> Result<()> {
    validate_process_manager_options(&config.process_manager)?;
    validate_process_configs(&config.managed_processes)?;
    
    if let Some(ref log_config) = config.log_collection {
        validate_log_collection_config(log_config)?;
    }
    
    Ok(())
}

/// Validate process manager options
fn validate_process_manager_options(options: &ProcessManagerOptions) -> Result<()> {
    if options.port == 0 {
        return Err(anyhow!("Port must be between 1 and 65535, got: {}", options.port));
    }
    
    if options.force_shutdown_timeout.as_secs() == 0 {
        return Err(anyhow!("Force shutdown timeout must be greater than 0"));
    }
    
    match options.log_level.to_lowercase().as_str() {
        "trace" | "debug" | "info" | "warn" | "error" => Ok(()),
        _ => Err(anyhow!("Invalid log level: {}, must be one of: trace, debug, info, warn, error", options.log_level))
    }
}

/// Validate all process configurations
fn validate_process_configs(processes: &[ProcessConfig]) -> Result<()> {
    if processes.is_empty() {
        return Err(anyhow!("At least one process must be configured"));
    }
    
    // Check for duplicate IDs
    let mut ids = HashSet::new();
    for process in processes {
        if !ids.insert(&process.id) {
            return Err(anyhow!("Duplicate process ID: {}", process.id));
        }
        
        validate_process_config(process)?;
    }
    
    Ok(())
}

/// Validate a single process configuration
fn validate_process_config(process: &ProcessConfig) -> Result<()> {
    if process.id.is_empty() {
        return Err(anyhow!("Process ID cannot be empty"));
    }
    
    if process.id.len() > 64 {
        return Err(anyhow!("Process ID too long (max 64 characters): {}", process.id));
    }
    
    // Validate ID contains only safe characters
    if !process.id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("Process ID can only contain alphanumeric characters, hyphens, and underscores: {}", process.id));
    }
    
    validate_process_management_config(&process.process_type, &process.management)?;
    
    Ok(())
}

/// Validate process management configuration matches type
fn validate_process_management_config(process_type: &ProcessManagementType, management: &ProcessManagementConfig) -> Result<()> {
    match process_type {
        ProcessManagementType::StandardManaged => {
            if management.standard_managed.is_none() {
                return Err(anyhow!("standard_managed configuration is required for standard_managed process type"));
            }
            if management.integrated_managed.is_some() || management.unmanaged.is_some() {
                return Err(anyhow!("Only standard_managed configuration should be specified for standard_managed process type"));
            }
            validate_standard_managed_config(management.standard_managed.as_ref().unwrap())?;
        }
        ProcessManagementType::IntegratedManaged => {
            if management.integrated_managed.is_none() {
                return Err(anyhow!("integrated_managed configuration is required for integrated_managed process type"));
            }
            if management.standard_managed.is_some() || management.unmanaged.is_some() {
                return Err(anyhow!("Only integrated_managed configuration should be specified for integrated_managed process type"));
            }
            validate_integrated_managed_config(management.integrated_managed.as_ref().unwrap())?;
        }
        ProcessManagementType::Unmanaged => {
            if management.unmanaged.is_none() {
                return Err(anyhow!("unmanaged configuration is required for unmanaged process type"));
            }
            if management.standard_managed.is_some() || management.integrated_managed.is_some() {
                return Err(anyhow!("Only unmanaged configuration should be specified for unmanaged process type"));
            }
            validate_unmanaged_config(management.unmanaged.as_ref().unwrap())?;
        }
    }
    
    Ok(())
}

/// Validate standard managed process configuration
fn validate_standard_managed_config(config: &StandardManagedProcessConfig) -> Result<()> {
    validate_managed_process_control_config(&config.control)?;
    
    if let Some(ref health_check) = config.health_check {
        validate_health_check_config(health_check)?;
    }
    
    Ok(())
}

/// Validate integrated managed process configuration
fn validate_integrated_managed_config(config: &IntegratedManagedProcessConfig) -> Result<()> {
    validate_managed_process_control_config(&config.control)?;
    
    if let Some(ref health_check) = config.health_check {
        validate_health_check_config(health_check)?;
    }
    
    Ok(())
}

/// Validate unmanaged process configuration
fn validate_unmanaged_config(config: &UnmanagedProcessConfig) -> Result<()> {
    validate_process_discovery_config(&config.discovery)?;
    Ok(())
}

/// Validate managed process control configuration
fn validate_managed_process_control_config(control: &ManagedProcessControlConfig) -> Result<()> {
    validate_execution_config(&control.execution)?;
    
    if let Some(ref restart_config) = control.context_aware_restart {
        validate_context_aware_restart_config(restart_config)?;
    }
    
    if let Some(ref limits) = control.limits {
        validate_resource_limits_config(limits)?;
    }
    
    Ok(())
}

/// Validate execution configuration
fn validate_execution_config(execution: &ExecutionConfig) -> Result<()> {
    if execution.executable_path.is_empty() {
        return Err(anyhow!("Executable path cannot be empty"));
    }
    
    // Check if wait_delay is zero using Duration::ZERO (supports milliseconds)
    if execution.wait_delay == std::time::Duration::ZERO {
        return Err(anyhow!("Wait delay must be greater than 0"));
    }
    
    // Validate environment variable names
    for (key, _) in &execution.environment {
        if key.is_empty() {
            return Err(anyhow!("Environment variable name cannot be empty"));
        }
        
        if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(anyhow!("Environment variable name can only contain alphanumeric characters and underscores: {}", key));
        }
    }
    
    Ok(())
}

/// Validate context-aware restart configuration
fn validate_context_aware_restart_config(config: &ContextAwareRestartConfig) -> Result<()> {
    validate_restart_config(&config.default)?;
    
    if let Some(ref health_failures) = config.health_failures {
        validate_restart_config(health_failures)?;
    }
    
    if let Some(ref resource_violations) = config.resource_violations {
        validate_restart_config(resource_violations)?;
    }
    
    Ok(())
}

/// Validate restart configuration
fn validate_restart_config(config: &RestartConfig) -> Result<()> {
    if config.max_retries == 0 {
        return Err(anyhow!("Max retries must be greater than 0"));
    }
    
    if config.max_retries > 100 {
        return Err(anyhow!("Max retries too high (max 100): {}", config.max_retries));
    }
    
    if config.retry_delay.as_secs() == 0 {
        return Err(anyhow!("Retry delay must be greater than 0"));
    }
    
    if config.backoff_rate < 1.0 || config.backoff_rate > 10.0 {
        return Err(anyhow!("Backoff rate must be between 1.0 and 10.0, got: {}", config.backoff_rate));
    }
    
    Ok(())
}

/// Validate resource limits configuration
fn validate_resource_limits_config(limits: &ResourceLimitsConfig) -> Result<()> {
    if let Some(ref memory) = limits.memory {
        let max_rss_bytes = memory.max_rss_bytes();
        if max_rss_bytes == 0 {
            return Err(anyhow!("Memory limit must be greater than 0 (specify either max_rss or limit_mb)"));
        }
        
        if let Some(warning) = memory.warning_threshold {
            if warning > 100 {
                return Err(anyhow!("Memory warning threshold must be between 0 and 100, got: {}", warning));
            }
        }
        
        if let Some(ref policy) = memory.policy {
            validate_resource_policy(policy)?;
        }
    }
    
    if let Some(ref cpu) = limits.cpu {
        if cpu.max_percent <= 0.0 || cpu.max_percent > 100.0 {
            return Err(anyhow!("Max CPU percent must be between 0 and 100, got: {}", cpu.max_percent));
        }
        
        if let Some(warning) = cpu.warning_threshold {
            if warning > 100 {
                return Err(anyhow!("CPU warning threshold must be between 0 and 100, got: {}", warning));
            }
        }
        
        if let Some(ref policy) = cpu.policy {
            validate_resource_policy(policy)?;
        }
    }
    
    if let Some(ref process) = limits.process {
        if let Some(fd) = process.max_file_descriptors {
            if fd == 0 {
                return Err(anyhow!("Max file descriptors must be greater than 0"));
            }
        }
        
        if let Some(ref policy) = process.policy {
            validate_resource_policy(policy)?;
        }
    }
    
    Ok(())
}

/// Validate resource policy
fn validate_resource_policy(policy: &str) -> Result<()> {
    match policy {
        "none" | "log" | "alert" | "throttle" | "graceful_shutdown" | "immediate_kill" | "restart" | "restart_adjusted" => Ok(()),
        _ => Err(anyhow!("Invalid resource policy: {}", policy))
    }
}

/// Validate health check configuration
fn validate_health_check_config(health_check: &HealthCheckConfig) -> Result<()> {
    // Validate health check type
    match health_check.health_check_type.as_str() {
        "process" | "http" | "grpc" | "tcp" => {},
        _ => return Err(anyhow!("Invalid health check type: {}", health_check.health_check_type)),
    }
    
    if let Some(ref run_options) = health_check.run_options {
        validate_health_check_run_options(run_options)?;
    }
    
    Ok(())
}

/// Validate health check run options
fn validate_health_check_run_options(options: &HealthCheckRunOptions) -> Result<()> {
    if options.interval == std::time::Duration::ZERO {
        return Err(anyhow!("Health check interval must be greater than 0"));
    }
    
    if options.timeout == std::time::Duration::ZERO {
        return Err(anyhow!("Health check timeout must be greater than 0"));
    }
    
    if options.timeout >= options.interval {
        return Err(anyhow!("Health check timeout must be less than interval"));
    }
    
    Ok(())
}

/// Validate process discovery configuration
fn validate_process_discovery_config(discovery: &ProcessDiscoveryConfig) -> Result<()> {
    match discovery.method.as_str() {
        "pid_file" => {
            if discovery.pid_file.is_none() {
                return Err(anyhow!("pid_file must be specified for pid_file discovery method"));
            }
        }
        "process_name" => {
            if discovery.process_name.is_none() {
                return Err(anyhow!("process_name must be specified for process_name discovery method"));
            }
        }
        "port" => {
            // Port-based discovery validation can be added here
        }
        _ => return Err(anyhow!("Invalid discovery method: {}", discovery.method)),
    }
    
    if discovery.check_interval.as_secs() == 0 {
        return Err(anyhow!("Check interval must be greater than 0"));
    }
    
    Ok(())
}

/// Validate log collection configuration
fn validate_log_collection_config(log_config: &LogCollectionConfig) -> Result<()> {
    if let Some(ref aggregation) = log_config.global_aggregation {
        validate_global_aggregation_config(aggregation)?;
    }
    
    Ok(())
}

/// Validate global aggregation configuration
fn validate_global_aggregation_config(aggregation: &GlobalAggregationConfig) -> Result<()> {
    for target in &aggregation.targets {
        validate_log_target_config(target)?;
    }
    
    Ok(())
}

/// Validate log target configuration
fn validate_log_target_config(target: &LogTargetConfig) -> Result<()> {
    match target.target_type.as_str() {
        "file" => {
            if target.path.is_none() {
                return Err(anyhow!("File log target must specify a path"));
            }
        }
        "process_manager_stdout" | "process_manager_stderr" => {
            // These don't need additional validation
        }
        _ => {
            return Err(anyhow!("Invalid log target type: {}", target.target_type));
        }
    }
    
    match target.format.as_str() {
        "plain" | "enhanced_plain" | "json" => Ok(()),
        _ => Err(anyhow!("Invalid log format: {}, must be one of: plain, enhanced_plain, json", target.format))
    }
}
