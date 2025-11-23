//! Process execution primitives.
//!
//! This module provides low-level process spawning and execution.

use hsu_common::ProcessResult;
use std::process::Command;

/// Execute a process with the given command and arguments.
/// 
/// This is a placeholder implementation. Full implementation will follow.
pub fn execute_command(executable: &str, args: &[String]) -> ProcessResult<std::process::Child> {
    Command::new(executable)
        .args(args)
        .spawn()
        .map_err(|e| hsu_common::ProcessError::spawn_failed(executable, e.to_string()))
}

