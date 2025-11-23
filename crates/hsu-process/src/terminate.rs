//! Process termination primitives.
//!
//! This module provides cross-platform process termination.

use hsu_common::ProcessResult;

/// Terminate a process gracefully (SIGTERM on Unix, WM_CLOSE on Windows).
pub fn terminate_gracefully(pid: u32) -> ProcessResult<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        let nix_pid = Pid::from_raw(pid as i32);
        kill(nix_pid, Signal::SIGTERM)
            .map_err(|e| hsu_common::ProcessError::stop_failed(pid.to_string(), e.to_string()))
    }

    #[cfg(windows)]
    {
        use std::time::Duration;
        
        // Use Windows-specific graceful termination with Ctrl+Break
        crate::terminate_windows::send_termination_signal(
            pid,
            false, // Process is alive
            Duration::from_secs(5),
        )
        .map_err(|e| hsu_common::ProcessError::stop_failed(pid.to_string(), e))
    }
}

/// Force kill a process (SIGKILL on Unix, TerminateProcess on Windows).
pub fn force_kill(pid: u32) -> ProcessResult<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        let nix_pid = Pid::from_raw(pid as i32);
        kill(nix_pid, Signal::SIGKILL)
            .map_err(|e| hsu_common::ProcessError::stop_failed(pid.to_string(), e.to_string()))
    }

    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        
        unsafe {
            // Open process with terminate rights
            let handle = match OpenProcess(PROCESS_TERMINATE, false, pid) {
                Ok(h) if !h.is_invalid() => h,
                _ => {
                    return Err(hsu_common::ProcessError::stop_failed(
                        pid.to_string(),
                        "Failed to open process for termination".to_string(),
                    ));
                }
            };
            
            // Terminate process with exit code 1
            let result = TerminateProcess(handle, 1);
            
            // Close handle
            let _ = CloseHandle(handle);
            
            result.map_err(|e| hsu_common::ProcessError::stop_failed(
                pid.to_string(),
                format!("TerminateProcess failed: {}", e),
            ))
        }
    }
}

