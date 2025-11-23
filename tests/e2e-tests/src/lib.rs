// E2E Test Framework for HSU Process Manager

pub mod process_manager;
pub mod test_executor;
pub mod log_parser;
pub mod assertions;

pub use process_manager::ProcessManagerWrapper;
pub use test_executor::TestExecutor;
pub use log_parser::LogParser;

use std::path::PathBuf;
use std::env;

/// Get the path to the PROCMAN (hsu-process-manager) binary
pub fn get_procman_path() -> PathBuf {
    let mut path = env::current_exe()
        .expect("Failed to get current exe path")
        .parent()
        .expect("Failed to get parent dir")
        .to_path_buf();
    
    // If we're in deps/, go up one level
    if path.ends_with("deps") {
        path.pop();
    }
    
    #[cfg(windows)]
    path.push("hsu-process-manager.exe");
    
    #[cfg(not(windows))]
    path.push("hsu-process-manager");
    
    if !path.exists() {
        panic!("PROCMAN binary not found at: {}", path.display());
    }
    
    path
}

/// Get the path to the TESTEXE (testexe) binary
pub fn get_testexe_path() -> PathBuf {
    let mut path = env::current_exe()
        .expect("Failed to get current exe path")
        .parent()
        .expect("Failed to get parent dir")
        .to_path_buf();
    
    // If we're in deps/, go up one level
    if path.ends_with("deps") {
        path.pop();
    }
    
    #[cfg(windows)]
    path.push("testexe.exe");
    
    #[cfg(not(windows))]
    path.push("testexe");
    
    if !path.exists() {
        panic!("TESTEXE binary not found at: {}", path.display());
    }
    
    path
}

/// Create a temporary test directory with timestamp for uniqueness
pub fn create_test_dir(test_name: &str) -> PathBuf {
    // Get the target directory (exe is at target/debug/deps/test_exe)
    let target_dir = env::current_exe()
        .expect("Failed to get current exe path")
        .parent().expect("Failed to get parent (deps)")
        .parent().expect("Failed to get parent (debug)")
        .to_path_buf();
    
    // Add timestamp for unique test runs
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs();
    
    let temp_dir = target_dir
        .join("tmp")
        .join(format!("e2e-test-{}", test_name))
        .join(format!("run-{}", timestamp));
    
    std::fs::create_dir_all(&temp_dir).expect("Failed to create test directory");
    temp_dir
}

/// Clean up test directory
pub fn cleanup_test_dir(dir: &PathBuf) {
    if dir.exists() {
        std::fs::remove_dir_all(dir).ok();
    }
}

