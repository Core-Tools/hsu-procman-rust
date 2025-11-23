//! Process validation utilities.
//!
//! Validation functions for process configuration and operations.

use hsu_common::ProcessResult;

/// Validate that an executable exists and is accessible.
pub fn validate_executable(path: &str) -> ProcessResult<()> {
    if path.is_empty() {
        return Err(hsu_common::ProcessError::Configuration {
            id: "validation".to_string(),
            reason: "Executable path cannot be empty".to_string(),
        });
    }

    // TODO: Check if file exists and is executable
    Ok(())
}

/// Validate process ID format.
pub fn validate_process_id(id: &str) -> ProcessResult<()> {
    if id.is_empty() {
        return Err(hsu_common::ProcessError::Configuration {
            id: "validation".to_string(),
            reason: "Process ID cannot be empty".to_string(),
        });
    }

    if !id.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(hsu_common::ProcessError::Configuration {
            id: id.to_string(),
            reason: "Process ID can only contain alphanumeric characters, hyphens, and underscores".to_string(),
        });
    }

    Ok(())
}

