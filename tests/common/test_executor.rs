use std::path::PathBuf;
use std::time::Duration;
use super::process_manager::{ProcessManagerWrapper, TestConfigOptions, create_test_config};
use super::{get_procman_path, get_testexe_path, create_test_dir, cleanup_test_dir};

/// High-level test executor that manages the entire test lifecycle
pub struct TestExecutor {
    pub test_name: String,
    pub test_dir: PathBuf,
    pub procman_path: PathBuf,
    pub testexe_path: PathBuf,
}

impl TestExecutor {
    /// Create a new test executor
    pub fn new(test_name: &str) -> Self {
        let test_dir = create_test_dir(test_name);
        let procman_path = get_procman_path();
        let testexe_path = get_testexe_path();
        
        println!("=== Test Executor Setup ===");
        println!("Test: {}", test_name);
        println!("Test dir: {}", test_dir.display());
        println!("PROCMAN: {}", procman_path.display());
        println!("TESTEXE: {}", testexe_path.display());
        println!("===========================\n");
        
        Self {
            test_name: test_name.to_string(),
            test_dir,
            procman_path,
            testexe_path,
        }
    }

    /// Run a test scenario with the given configuration
    pub fn run_test<F>(&self, config_opts: TestConfigOptions, test_fn: F) -> Result<(), String>
    where
        F: FnOnce(&mut ProcessManagerWrapper) -> Result<(), String>,
    {
        // Create config file
        let config_path = create_test_config(
            &self.test_dir,
            &self.testexe_path,
            "testexe",
            config_opts,
        )?;

        // Create and start PROCMAN
        let mut procman = ProcessManagerWrapper::new(config_path, self.test_dir.clone());
        procman.start(&self.procman_path)?;

        // Wait for PROCMAN to be ready
        procman.wait_for_ready(Duration::from_secs(5))?;

        // Run the test
        let result = test_fn(&mut procman);

        // Shutdown PROCMAN
        procman.shutdown()?;

        result
    }

    /// Cleanup test directory
    pub fn cleanup(&self) {
        cleanup_test_dir(&self.test_dir);
    }
}

impl Drop for TestExecutor {
    fn drop(&mut self) {
        // Auto-cleanup in tests (but keep for debugging if test panics)
        if !std::thread::panicking() {
            self.cleanup();
        } else {
            println!("Test panicked, keeping test directory for debugging: {}", self.test_dir.display());
        }
    }
}

