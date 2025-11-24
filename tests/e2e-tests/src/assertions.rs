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

/// Assert that PID files have been cleaned up (proves no zombies)
/// 
/// PID files are deleted AFTER child.wait() completes, which reaps zombies.
/// If PID files are absent, it proves processes exited cleanly with no zombies.
/// 
/// # Arguments
/// * `procman` - The process manager wrapper
/// * `process_id` - The process ID to check (e.g., "testexe")
/// 
/// # Returns
/// * `Ok(())` if no PID file exists (clean termination)
/// * `Err(msg)` if PID file still exists (indicates incomplete termination or zombie)
pub fn assert_no_pid_file(procman: &ProcessManagerWrapper, process_id: &str) -> Result<(), String> {
    assert_no_pid_file_in_dir(&procman.test_dir, process_id)
}

/// Assert that a specific PID file doesn't exist in the given test directory
/// 
/// This is a lower-level version that works with just a Path, useful for
/// checking after a test completes and the wrapper is no longer available.
pub fn assert_no_pid_file_in_dir(test_dir: &Path, process_id: &str) -> Result<(), String> {
    let pid_dir = test_dir.join("hsu-procman");
    let pid_file = pid_dir.join(format!("{}.pid", process_id));
    
    if !pid_file.exists() {
        println!("✓ PID file cleaned up: {} (proves clean termination)", pid_file.display());
        Ok(())
    } else {
        // Try to read the PID file to provide more debugging info
        let pid_content = fs::read_to_string(&pid_file)
            .unwrap_or_else(|_| "<unreadable>".to_string());
        
        Err(format!(
            "PID file still exists: {}\nContent: {}\n\
             This indicates the process may not have terminated cleanly or a zombie may exist.\n\
             PID files are deleted AFTER child.wait() completes, so presence indicates incomplete termination.",
            pid_file.display(),
            pid_content.trim()
        ))
    }
}

/// Assert that the PID directory is empty (all PID files cleaned up)
/// 
/// This is a stronger check that verifies ALL managed processes have been cleaned up.
/// Useful for testing complete shutdown scenarios.
pub fn assert_pid_directory_empty(procman: &ProcessManagerWrapper) -> Result<(), String> {
    assert_pid_directory_empty_in_dir(&procman.test_dir)
}

/// Assert that the PID directory is empty in the given test directory
/// 
/// This is a lower-level version that works with just a Path, useful for
/// checking after a test completes and the wrapper is no longer available.
pub fn assert_pid_directory_empty_in_dir(test_dir: &Path) -> Result<(), String> {
    let pid_dir = test_dir.join("hsu-procman");
    
    if !pid_dir.exists() {
        println!("✓ PID directory doesn't exist: {} (no PID files created, or fully cleaned)", pid_dir.display());
        return Ok(());
    }
    
    // Read directory contents
    let entries: Vec<_> = fs::read_dir(&pid_dir)
        .map_err(|e| format!("Failed to read PID directory: {}", e))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            // Filter for .pid files only
            e.path().extension().and_then(|ext| ext.to_str()) == Some("pid")
        })
        .collect();
    
    if entries.is_empty() {
        println!("✓ PID directory is empty: {} (all processes terminated cleanly)", pid_dir.display());
        Ok(())
    } else {
        let pid_files: Vec<String> = entries.iter()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        
        Err(format!(
            "PID directory is not empty: {}\nRemaining PID files: {:?}\n\
             This indicates some processes did not terminate cleanly.",
            pid_dir.display(),
            pid_files
        ))
    }
}

