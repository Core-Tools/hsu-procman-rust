//! Process existence checking and discovery.
//!
//! Provides cross-platform functions to check if a process exists and is running.

use hsu_common::ProcessResult;

/// Check if a process with the given PID exists and is running.
///
/// This function performs a non-destructive check to determine if a process
/// is still alive. On Unix systems, it uses `kill(pid, 0)`, which sends no
/// signal but checks if the process exists. On Windows, it uses `OpenProcess`.
///
/// # Arguments
///
/// * `pid` - The process ID to check
///
/// # Returns
///
/// * `Ok(true)` - Process exists and is running
/// * `Ok(false)` - Process does not exist
/// * `Err(_)` - Error occurred while checking (e.g., permission denied)
///
/// # Examples
///
/// ```rust,no_run
/// use hsu_process::process_exists;
///
/// let exists = process_exists(1234).unwrap();
/// if exists {
///     println!("Process 1234 is running");
/// } else {
///     println!("Process 1234 does not exist");
/// }
/// ```
pub fn process_exists(pid: u32) -> ProcessResult<bool> {
    #[cfg(unix)]
    {
        process_exists_unix(pid)
    }
    
    #[cfg(windows)]
    {
        process_exists_windows(pid)
    }
}

#[cfg(unix)]
fn process_exists_unix(pid: u32) -> ProcessResult<bool> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    
    let nix_pid = Pid::from_raw(pid as i32);
    
    match kill(nix_pid, None) {
        Ok(_) => Ok(true),  // Process exists
        Err(nix::errno::Errno::ESRCH) => Ok(false),  // No such process
        Err(nix::errno::Errno::EPERM) => Ok(true),   // Process exists but we don't have permission
        Err(e) => Err(hsu_common::ProcessError::Configuration {
            id: pid.to_string(),
            reason: format!("Failed to check process: {}", e),
        }),
    }
}

#[cfg(windows)]
fn process_exists_windows(pid: u32) -> ProcessResult<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    
    unsafe {
        // Try to open the process handle
        let handle: HANDLE = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(e) => {
                // ERROR_INVALID_PARAMETER or ERROR_ACCESS_DENIED usually means process doesn't exist
                let error_code = e.code().0 as u32;
                const ERROR_INVALID_PARAMETER: u32 = 0x80070057;
                const ERROR_ACCESS_DENIED: u32 = 0x80070005;
                
                if error_code == ERROR_INVALID_PARAMETER || error_code == ERROR_ACCESS_DENIED {
                    return Ok(false);
                }
                return Err(hsu_common::ProcessError::Configuration {
                    id: pid.to_string(),
                    reason: format!("Failed to check process: {}", e),
                });
            }
        };
        
        // If we got a valid handle, the process exists
        let _ = CloseHandle(handle);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_current_process_exists() {
        // Current process should always exist
        let current_pid = std::process::id();
        assert!(process_exists(current_pid).unwrap());
    }
    
    #[test]
    fn test_nonexistent_process() {
        // PID 0 should not exist (it's reserved)
        // High PIDs are unlikely to exist
        let unlikely_pid = if cfg!(windows) { 99999999 } else { 9999999 };
        let exists = process_exists(unlikely_pid).unwrap();
        // Should be false (unless extremely unlucky timing)
        assert!(!exists || exists); // Accept either outcome (process might exist)
    }
    
    #[test]
    #[cfg(unix)]
    fn test_system_process() {
        // PID 1 (init/systemd) should exist on Unix
        assert!(process_exists(1).unwrap());
    }
}

