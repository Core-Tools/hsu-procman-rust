//! Windows-specific process termination with graceful signal handling
//!
//! This module implements Windows-specific process termination using:
//! - GenerateConsoleCtrlEvent for graceful shutdown
//! - AttachConsole "dead PID hack" to fix console signal handling
//! - Console operation locking to prevent race conditions
//!
//! Mirrors Go's `pkg/process/terminate_windows.go`

use std::sync::Mutex;
use std::time::Duration;
use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT,
};

/// Global lock for console operations to prevent race conditions
static CONSOLE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

/// Send termination signal to a Windows process
///
/// # Arguments
/// * `pid` - Process ID to terminate
/// * `is_dead` - Whether the process is known to be dead (for console fix)
/// * `timeout` - Timeout for the operation
///
/// # Returns
/// Result indicating success or error message
pub fn send_termination_signal(pid: u32, is_dead: bool, timeout: Duration) -> Result<(), String> {
    if pid == 0 {
        return Err(format!("Invalid PID: {}", pid));
    }

    // Acquire lock to prevent race conditions with console operations
    let _lock = CONSOLE_OPERATION_LOCK
        .lock()
        .map_err(|e| format!("Failed to acquire console lock: {}", e))?;

    // Verify if process is actually dead (if claimed)
    let actually_dead = if is_dead {
        !crate::process_exists(pid).unwrap_or(false)
    } else {
        false
    };

    if actually_dead {
        // Apply the AttachConsole dead PID hack to fix Windows signal handling
        // This restores Ctrl+C functionality for the parent process
        console_signal_fix(pid)
    } else {
        // Send actual Ctrl+Break signal to alive process with safety checks
        send_ctrl_break_to_process_safe(pid, timeout)
    }
}

/// Console signal fix using the AttachConsole dead PID hack
///
/// This is a Windows-specific workaround that fixes console signal handling
/// by attempting to attach to a dead process, which resets console state.
fn console_signal_fix(dead_pid: u32) -> Result<(), String> {
    // Try to attach to the dead process - this triggers console state reset
    match attach_console(dead_pid) {
        Ok(_) => {
            // Unexpected success - should not happen with dead PID
            Err(format!(
                "Warning: AttachConsole unexpectedly succeeded for dead PID {}",
                dead_pid
            ))
        }
        Err(_) => {
            // Expected to fail for dead process - that's the magic!
            Ok(())
        }
    }
}

/// Send Ctrl+Break signal to process with liveness check and timeout protection
fn send_ctrl_break_to_process_safe(pid: u32, timeout: Duration) -> Result<(), String> {
    // Use channel to receive result from async operation
    let (tx, rx) = std::sync::mpsc::channel();

    // Spawn thread to send signal (to avoid blocking)
    std::thread::spawn(move || {
        let result = generate_console_ctrl_event(pid);
        let _ = tx.send(result);
    });

    // Wait for result with timeout
    match rx.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("Failed to send Ctrl+Break to PID {}: {}", pid, e)),
        Err(_) => Err(format!(
            "Timeout sending Ctrl+Break to PID {} after {:?}",
            pid, timeout
        )),
    }
}

/// Attach to console of given process ID
///
/// Uses Windows API AttachConsole
fn attach_console(pid: u32) -> Result<(), String> {
    unsafe {
        AttachConsole(pid).map_err(|_| "AttachConsole failed".to_string())
    }
}

/// Generate console control event (Ctrl+Break) for given process
///
/// Uses Windows API GenerateConsoleCtrlEvent
fn generate_console_ctrl_event(pid: u32) -> Result<(), String> {
    unsafe {
        GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)
            .map_err(|_| "GenerateConsoleCtrlEvent failed".to_string())
    }
}

/// Reset console session (advanced console management)
///
/// This is an alternative approach for complex scenarios.
/// Frees current console and allocates a new one.
#[allow(dead_code)]
pub fn reset_console_session() -> Result<(), String> {
    // Free current console (might fail if no console attached)
    let _ = free_console();

    // Allocate new console
    alloc_console()
}

/// Free the current console
fn free_console() -> Result<(), String> {
    unsafe {
        FreeConsole().map_err(|_| "FreeConsole failed".to_string())
    }
}

/// Allocate a new console
fn alloc_console() -> Result<(), String> {
    unsafe {
        AllocConsole().map_err(|_| "AllocConsole failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_signal_fix_with_invalid_pid() {
        // A very high PID is unlikely to exist
        let result = console_signal_fix(9999999);
        // Should succeed (expected failure means success for this hack)
        assert!(result.is_ok());
    }

    #[test]
    fn test_send_termination_signal_invalid_pid() {
        let result = send_termination_signal(0, false, Duration::from_secs(1));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid PID"));
    }
}

