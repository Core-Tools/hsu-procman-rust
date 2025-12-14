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
use std::sync::atomic::{AtomicU64, Ordering};

// -----------------------------------------------------------------------------
// ProcessManager implementation selection (procman-v1 vs procman-v2)
// -----------------------------------------------------------------------------

#[cfg(all(feature = "procman-v1", feature = "procman-v2"))]
compile_error!(
    "Exactly one procman feature must be enabled for e2e-tests: enable either \
     `procman-v1` or `procman-v2` (default)."
);

#[cfg(not(any(feature = "procman-v1", feature = "procman-v2")))]
compile_error!(
    "No procman feature selected for e2e-tests: enable `procman-v2` (default) \
     or explicitly enable `procman-v1`."
);

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

static TEST_RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// Create a temporary test directory with high-entropy uniqueness.
///
/// NOTE: This is intentionally collision-resistant even for parallel test runs started
/// in the same instant (PID + monotonic sequence + high-res time).
pub fn create_test_dir(test_name: &str) -> PathBuf {
    // Get the target directory (exe is at target/debug/deps/test_exe)
    let target_dir = env::current_exe()
        .expect("Failed to get current exe path")
        .parent().expect("Failed to get parent (deps)")
        .parent().expect("Failed to get parent (debug)")
        .to_path_buf();
    
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos();
    let pid = std::process::id();
    let seq = TEST_RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    
    let temp_dir = target_dir
        .join("tmp")
        .join(format!("e2e-test-{}", test_name))
        .join(format!("run-{}-pid{}-seq{}", now, pid, seq));
    
    std::fs::create_dir_all(&temp_dir).expect("Failed to create test directory");
    temp_dir
}

/// Clean up test directory
pub fn cleanup_test_dir(dir: &PathBuf) {
    if dir.exists() {
        std::fs::remove_dir_all(dir).ok();
    }
}

/// RAII guard that cleans up a test directory on scope exit (including unwinding panics).
#[derive(Debug)]
pub struct TestDirGuard {
    dir: PathBuf,
}

impl TestDirGuard {
    pub fn path(&self) -> &PathBuf {
        &self.dir
    }
}

impl Drop for TestDirGuard {
    fn drop(&mut self) {
        cleanup_test_dir(&self.dir);
    }
}

/// Create a temporary test directory and return a guard that will delete it on drop.
pub fn create_test_dir_guard(test_name: &str) -> (PathBuf, TestDirGuard) {
    let dir = create_test_dir(test_name);
    let guard = TestDirGuard { dir: dir.clone() };
    (dir, guard)
}

