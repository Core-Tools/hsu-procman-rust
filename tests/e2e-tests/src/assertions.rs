//! Custom assertions for E2E tests

use crate::process_manager::ProcessManagerWrapper;
use crate::log_parser::LogParser;
use std::path::Path;
use std::fs;

/// Assert that a process was started successfully
pub fn assert_process_started(procman: &ProcessManagerWrapper, process_id: &str) -> Result<(), String> {
    let has_started = procman.get_logs().iter().any(|line| {
        line.contains(process_id) && (
            line.contains("started") ||
            line.contains("Started process") ||
            line.contains("running")
        )
    });
    
    if has_started {
        Ok(())
    } else {
        Err(format!("Process '{}' was not started. Logs:\n{:#?}", process_id, procman.get_logs()))
    }
}

/// Assert that a process stopped gracefully
pub fn assert_process_stopped_gracefully(procman: &ProcessManagerWrapper, process_id: &str) -> Result<(), String> {
    let parser = LogParser::new(procman.get_logs().to_vec());
    
    // Look for graceful shutdown indicators
    let graceful_indicators = [
        "received signal",
        "Received SIGTERM",
        "Received SIGINT",
        "stopped",
    ];
    
    let has_graceful = graceful_indicators.iter().any(|pattern| parser.contains(pattern));
    
    if has_graceful {
        Ok(())
    } else {
        Err(format!(
            "Process '{}' did not stop gracefully. Expected one of: {:?}. Logs:\n{:#?}",
            process_id,
            graceful_indicators,
            procman.get_logs()
        ))
    }
}

/// Assert that a process was forcefully terminated
pub fn assert_process_force_killed(procman: &ProcessManagerWrapper, process_id: &str) -> Result<(), String> {
    let parser = LogParser::new(procman.get_logs().to_vec());
    
    // Forceful termination should NOT have graceful shutdown logs from the process itself
    let has_graceful = parser.contains("received signal") || 
                      parser.contains("stopped gracefully");
    
    if !has_graceful {
        Ok(())
    } else {
        Err(format!(
            "Process '{}' appears to have stopped gracefully, but expected force kill",
            process_id
        ))
    }
}

/// Assert that a process was restarted
pub fn assert_process_restarted(procman: &ProcessManagerWrapper, process_id: &str, reason: Option<&str>) -> Result<(), String> {
    let parser = LogParser::new(procman.get_logs().to_vec());
    
    let restart_count = parser.count_occurrences("restart");
    
    if restart_count == 0 {
        return Err(format!("Process '{}' was not restarted", process_id));
    }
    
    if let Some(expected_reason) = reason {
        if !parser.contains(expected_reason) {
            return Err(format!(
                "Process '{}' was restarted but reason '{}' not found in logs",
                process_id, expected_reason
            ));
        }
    }
    
    Ok(())
}

/// Assert that a resource violation was detected
pub fn assert_resource_violation(
    procman: &ProcessManagerWrapper,
    process_id: &str,
    resource_type: &str,
) -> Result<(), String> {
    let parser = LogParser::new(procman.get_logs().to_vec());
    
    let has_violation = parser.contains(resource_type) && 
                       (parser.contains("violation") || parser.contains("exceeded") || parser.contains("limit"));
    
    if has_violation {
        Ok(())
    } else {
        Err(format!(
            "Resource violation for '{}' not detected for process '{}'. Logs:\n{:#?}",
            resource_type,
            process_id,
            procman.get_logs()
        ))
    }
}

/// Assert that a log file exists and contains expected content
pub fn assert_log_file_contains(file_path: &Path, pattern: &str) -> Result<(), String> {
    if !file_path.exists() {
        return Err(format!("Log file does not exist: {}", file_path.display()));
    }
    
    let content = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read log file: {}", e))?;
    
    if content.contains(pattern) {
        Ok(())
    } else {
        Err(format!(
            "Log file {} does not contain pattern '{}'. Content:\n{}",
            file_path.display(),
            pattern,
            content
        ))
    }
}

/// Assert that a log file exists
pub fn assert_log_file_exists(file_path: &Path) -> Result<(), String> {
    if file_path.exists() {
        Ok(())
    } else {
        Err(format!("Log file does not exist: {}", file_path.display()))
    }
}

/// Assert that PROCMAN itself is running
pub fn assert_procman_running(procman: &ProcessManagerWrapper) -> Result<(), String> {
    if procman.get_pid().is_some() {
        Ok(())
    } else {
        Err("PROCMAN is not running".to_string())
    }
}

/// Assert that logs contain a specific pattern
pub fn assert_logs_contain(procman: &ProcessManagerWrapper, pattern: &str) -> Result<(), String> {
    if procman.has_log_matching(pattern) {
        Ok(())
    } else {
        Err(format!("Logs do not contain pattern: '{}'. Logs:\n{:#?}", pattern, procman.get_logs()))
    }
}

/// Assert that logs do NOT contain a specific pattern
pub fn assert_logs_not_contain(procman: &ProcessManagerWrapper, pattern: &str) -> Result<(), String> {
    if !procman.has_log_matching(pattern) {
        Ok(())
    } else {
        Err(format!("Logs should NOT contain pattern: '{}', but it was found", pattern))
    }
}

