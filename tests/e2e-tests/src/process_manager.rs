//! ProcessManager wrapper for E2E testing

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use std::thread;
use std::fs;
use wait_timeout::ChildExt;

/// Wrapper for managing the Process Manager (PROCMAN) during tests
pub struct ProcessManagerWrapper {
    process: Option<Child>,
    config_path: PathBuf,
    log_output: Vec<String>,
    pub test_dir: PathBuf,
    log_file_path: Option<PathBuf>,
}

impl ProcessManagerWrapper {
    /// Create a new ProcessManagerWrapper with the given config file
    pub fn new(config_path: PathBuf, test_dir: PathBuf) -> Self {
        Self {
            process: None,
            config_path,
            log_output: Vec::new(),
            test_dir,
            log_file_path: None,
        }
    }

    /// Start PROCMAN with the configured settings
    pub fn start(&mut self, procman_path: &Path) -> Result<(), String> {
        if self.process.is_some() {
            return Err("Process manager is already running".to_string());
        }

        println!("Starting PROCMAN: {}", procman_path.display());
        println!("Config: {}", self.config_path.display());
        println!("Working dir: {}", self.test_dir.display());

        // Create log file for PROCMAN output
        let log_file_path = self.test_dir.join("procman.log");
        let log_file = std::fs::File::create(&log_file_path)
            .map_err(|e| format!("Failed to create log file: {}", e))?;
        
        let log_file_clone = log_file.try_clone()
            .map_err(|e| format!("Failed to clone log file: {}", e))?;

        let mut cmd = Command::new(procman_path);
        cmd.arg("--config")
            .arg(&self.config_path)
            .current_dir(&self.test_dir)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_clone));

        let child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn PROCMAN: {}", e))?;

        self.process = Some(child);
        self.log_file_path = Some(log_file_path);
        println!("PROCMAN started with PID: {:?}", self.get_pid());

        Ok(())
    }

    /// Wait for PROCMAN to be ready (check logs for ready signal)
    pub fn wait_for_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            self.collect_logs();
            
            // Check if PROCMAN has logged that it's running
            if self.log_output.iter().any(|line| {
                line.contains("Process manager started successfully") ||
                line.contains("gRPC server listening") ||
                line.contains("Starting process manager")
            }) {
                println!("PROCMAN is ready");
                return Ok(());
            }
            
            thread::sleep(Duration::from_millis(100));
        }
        
        Err(format!("PROCMAN did not become ready within {} seconds", timeout.as_secs()))
    }

    /// Wait for a managed process to be running
    pub fn wait_for_process_running(&mut self, process_id: &str, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            self.collect_logs();
            
            // Look for process start indication in logs
            if self.log_output.iter().any(|line| {
                line.contains(process_id) && (
                    line.contains("started") ||
                    line.contains("running") ||
                    line.contains("Started process")
                )
            }) {
                println!("Process '{}' is running", process_id);
                return Ok(());
            }
            
            thread::sleep(Duration::from_millis(100));
        }
        
        Err(format!("Process '{}' did not start within {} seconds", process_id, timeout.as_secs()))
    }

    /// Send termination signal to PROCMAN
    pub fn shutdown(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.process.take() {
            println!("Sending termination signal to PROCMAN...");
            
            #[cfg(windows)]
            {
                // On Windows, we need to send Ctrl+C
                // For now, just kill it
                child.kill().map_err(|e| format!("Failed to kill PROCMAN: {}", e))?;
            }
            
            #[cfg(unix)]
            {
                // On Unix, send SIGTERM
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(child.id() as i32),
                    nix::sys::signal::Signal::SIGTERM
                ).map_err(|e| format!("Failed to send SIGTERM: {}", e))?;
            }
            
            // Collect remaining logs
            thread::sleep(Duration::from_millis(500));
            self.collect_final_logs(&mut child);
            
            // Wait for process to exit
            match child.wait_timeout(Duration::from_secs(10)) {
                Ok(Some(status)) => {
                    println!("PROCMAN exited with status: {}", status);
                    Ok(())
                }
                Ok(None) => {
                    println!("PROCMAN did not exit in time, forcing kill");
                    child.kill().map_err(|e| format!("Failed to force kill: {}", e))?;
                    child.wait().ok();
                    Ok(())
                }
                Err(e) => Err(format!("Error waiting for PROCMAN: {}", e)),
            }
        } else {
            Ok(()) // Already stopped
        }
    }

    /// Collect logs from PROCMAN log file (non-blocking)
    fn collect_logs(&mut self) {
        if let Some(log_file_path) = &self.log_file_path {
            // Read all lines from the log file
            if let Ok(content) = fs::read_to_string(log_file_path) {
                let new_lines: Vec<String> = content.lines()
                    .map(|s| s.to_string())
                    .collect();
                
                // Only print and add lines we haven't seen yet
                for (i, line) in new_lines.iter().enumerate() {
                    if i >= self.log_output.len() {
                        println!("[PROCMAN] {}", line);
                    }
                }
                
                self.log_output = new_lines;
            }
        }
    }

    /// Collect final logs before process exits
    fn collect_final_logs(&mut self, _child: &mut Child) {
        // Just read from the log file one more time
        self.collect_logs();
    }

    /// Get all collected logs
    pub fn get_logs(&self) -> &[String] {
        &self.log_output
    }

    /// Manually trigger log collection (public version of collect_logs)
    pub fn collect_logs_now(&mut self) {
        self.collect_logs();
    }

    /// Check if a log line exists matching the pattern
    pub fn has_log_matching(&self, pattern: &str) -> bool {
        self.log_output.iter().any(|line| line.contains(pattern))
    }

    /// Get the PID of PROCMAN
    pub fn get_pid(&self) -> Option<u32> {
        self.process.as_ref().map(|p| p.id())
    }

    /// Force kill PROCMAN
    pub fn force_kill(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.process.take() {
            child.kill().map_err(|e| format!("Failed to kill PROCMAN: {}", e))?;
            child.wait().ok();
        }
        Ok(())
    }
}

impl Drop for ProcessManagerWrapper {
    fn drop(&mut self) {
        // Ensure PROCMAN is stopped when wrapper is dropped
        if self.process.is_some() {
            println!("Cleaning up PROCMAN in Drop");
            self.force_kill().ok();
        }
    }
}

/// Helper to create a config file for testing
pub fn create_test_config(
    test_dir: &Path,
    testexe_path: &Path,
    process_id: &str,
    config_overrides: TestConfigOptions,
) -> Result<PathBuf, String> {
    let config_path = test_dir.join("config.yaml");
    
    // Convert path to string and escape backslashes for YAML
    let testexe_str = testexe_path.to_string_lossy().replace('\\', "\\\\");
    let args = config_overrides.testexe_args.join("\",\"");
    let graceful_timeout = config_overrides.graceful_timeout_ms;
    
    // Build resource limits section if memory limit is specified
    let resource_limits = if let Some(memory_mb) = config_overrides.memory_limit_mb {
        let policy = config_overrides.memory_policy.as_deref().unwrap_or("log");
        format!(r#"
          limits:
            memory:
              limit_mb: {}
              policy: "{}""#, memory_mb, policy)
    } else {
        String::new()
    };
    
    // Build health check section based on type
    let health_check_type = config_overrides.health_check_type.as_deref().unwrap_or("process");
    let health_check_interval = config_overrides.health_check_interval_ms.unwrap_or(1000);
    let health_check_timeout = config_overrides.health_check_timeout_ms.unwrap_or(500);
    let health_check_retries = config_overrides.health_check_failure_threshold.unwrap_or(3);
    
    let health_check_section = if health_check_type == "http" {
        let endpoint = config_overrides.health_check_endpoint
            .as_deref()
            .unwrap_or("http://localhost:8080/health");
        format!(r#"
        health_check:
          type: "http"
          http_endpoint: "{}"
          run_options:
            enabled: true
            interval: {}ms
            timeout: {}ms
            initial_delay: 0s
            retries: {}"#, endpoint, health_check_interval, health_check_timeout, health_check_retries)
    } else {
        format!(r#"
        health_check:
          type: "process"
          run_options:
            enabled: true
            interval: {}ms
            timeout: {}ms
            initial_delay: 0s
            retries: {}"#, health_check_interval, health_check_timeout, health_check_retries)
    };
    
    let config_content = format!(r#"
process_manager:
  port: {}
  log_level: "debug"
  force_shutdown_timeout: 30s

managed_processes:
  - id: "{}"
    enabled: true
    profile_type: "test"
    type: "standard_managed"
    management:
      standard_managed:
        control:
          execution:
            executable_path: "{}"
            args: ["{}"]
            wait_delay: 100ms
          graceful_timeout: {}ms
          restart_policy: "on-failure"
          context_aware_restart:
            default:
              max_retries: 5
              retry_delay: 1s
              backoff_rate: 1.0{}{}
"#, config_overrides.port, process_id, testexe_str, args, graceful_timeout, resource_limits, health_check_section);
    
    fs::write(&config_path, config_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;
    
    println!("Created test config at: {}", config_path.display());
    Ok(config_path)
}

/// Configuration options for test configs
pub struct TestConfigOptions {
    pub port: u16,
    pub testexe_args: Vec<String>,
    pub graceful_timeout_ms: u64,
    pub enable_logging: bool,
    pub log_dir: Option<PathBuf>,
    pub memory_limit_mb: Option<u64>,
    pub memory_policy: Option<String>,
    // HTTP health check options
    pub health_check_type: Option<String>,  // "process" or "http"
    pub health_check_endpoint: Option<String>,  // For HTTP health checks
    pub health_check_interval_ms: Option<u64>,
    pub health_check_timeout_ms: Option<u64>,
    pub health_check_failure_threshold: Option<u32>,
}

impl Default for TestConfigOptions {
    fn default() -> Self {
        Self {
            port: 50055,
            testexe_args: Vec::new(),
            graceful_timeout_ms: 5000,
            enable_logging: false,
            log_dir: None,
            memory_limit_mb: None,
            memory_policy: None,
            health_check_type: None,
            health_check_endpoint: None,
            health_check_interval_ms: None,
            health_check_timeout_ms: None,
            health_check_failure_threshold: None,
        }
    }
}

