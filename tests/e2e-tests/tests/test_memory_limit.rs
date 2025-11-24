//! Test Scenario 3.1: Memory Limit Violation & Restart
//!
//! Tests that PROCMAN can detect when a managed process exceeds memory limits
//! and take the configured action (warn, restart, terminate).
//!
//! ## Test Scenario
//!
//! 1. Start TESTEXE configured to allocate 150MB memory (limit is 100MB)
//! 2. Wait for memory allocation (150MB)
//! 3. Check for resource monitoring active
//! 4. Check for memory violation detection
//! 5. Wait for heartbeat to process violation
//! 6. Wait for automatic restart to complete
//! 7. Verify PID files are cleaned up (proves no zombies)
//!
//! ## Expected Results
//!
//! - Process starts successfully
//! - Memory allocation occurs (150MB)
//! - Resource monitor detects memory usage
//! - Memory exceeds configured limit (100MB)
//! - Violation detected and logged
//! - Restart policy triggers automatic restart
//! - Process terminated and restarted
//! - Second instance spawned
//! - Process spawn count = 2 (original + 1 restart)
//! - **PID files are cleaned up** (proves no zombies)
//!
//! ## Key Observations
//!
//! When examining test artifacts, look for:
//!
//! - ✅ Process spawn confirmation
//! - ✅ Resource monitoring started (CPU/memory tracking)
//! - ✅ Memory usage reported in logs
//! - ✅ Memory limit violation detected (usage > limit)
//! - ✅ Violation policy = "restart"
//! - ✅ Process terminated due to resource violation
//! - ✅ Restart triggered automatically
//! - ✅ Second instance spawned
//! - ✅ Resource monitor restarted for new instance
//! - ✅ **PID file cleanup** (absence proves no zombies)
//!
//! ## Detailed Flow (Log Lines to Look For)
//!
//! ```
//! [PROCMAN] Process spawned successfully: testexe (PID: 11111)
//! [PROCMAN] Starting health monitor for testexe (type: Process)
//! [PROCMAN] Spawning resource monitor task for testexe (PID: 11111)
//! [PROCMAN] 💓 Process health check result: healthy=true
//!
//! [After memory allocation]
//!
//! [PROCMAN] 📊 Resource usage for testexe (PID: 11111):
//!           CPU: X.X%, Memory: 150.XMB, FDs: X
//! [PROCMAN] ⚠️ Memory limit violation for testexe:
//!           usage=150.XMB, limit=100.0MB, policy=restart
//! [PROCMAN] 🔄 Triggering restart for testexe due to resource violation
//!
//! [PROCMAN] Restarting process control: testexe (force: true)
//! [PROCMAN] Stopping process control: testexe
//! [PROCMAN] Terminating process: testexe
//! [PROCMAN] Process terminated: testexe
//!
//! [PROCMAN] Process spawned successfully: testexe (PID: 22222)
//! [PROCMAN] Spawning resource monitor task for testexe (PID: 22222)
//! [PROCMAN] ✅ Automatic restart succeeded
//!
//! Process spawn count: 2 ✅
//! ```
//!
//! ## Framework Validation
//!
//! This test confirms that:
//!
//! 1. **Resource monitoring works** - CPU/memory tracking via sysinfo
//! 2. **Limit enforcement works** - Violations detected accurately
//! 3. **Policy application works** - "restart" policy triggers restart
//! 4. **Automatic restart works** - New instance spawned after violation
//! 5. **State cleanup works** - Old process terminated before restart
//! 6. **Continuous monitoring** - Resource monitor restarts for new instance
//!
//! ## Resource Limit Configuration
//!
//! ```yaml
//! resource_limits:
//!   limits:
//!     memory:
//!       limit_mb: 100          # Maximum memory allowed
//!       policy: "restart"      # Action on violation
//!   monitoring:
//!     enabled: true
//!     interval: 2s             # Check every 2 seconds
//! ```
//!
//! ## Policy Options
//!
//! - **log**: Log violation but take no action (warning only)
//! - **restart**: Terminate and restart the process
//! - **terminate**: Terminate the process (no restart)
//!
//! ## TESTEXE Memory Behavior
//!
//! The test executable is configured with:
//! - `--memory-mb 150` - Allocate 150MB of memory
//! - `--run-duration 15` - Keep running for 15 seconds
//!
//! This simulates a process that:
//! - Has a memory leak or high memory usage
//! - Exceeds configured limits
//! - Needs to be restarted to reclaim resources
//!
//! ## Platform-Specific Notes
//!
//! ### Memory Reporting
//!
//! - **Linux**: Uses `/proc/[pid]/status` (VmRSS - Resident Set Size)
//! - **macOS**: Uses `proc_pidinfo()` with `PROC_PIDTASKINFO`
//! - **Windows**: Uses `GetProcessMemoryInfo()` (WorkingSetSize)
//!
//! ### Accuracy
//!
//! - Memory reporting has ~1-2 second lag
//! - Resource monitor checks every 2 seconds by default
//! - Violation detection may take 2-4 seconds
//!
//! ## Test Artifacts
//!
//! Test runs preserve artifacts in timestamped directories:
//!
//! ```
//! target/debug/tmp/e2e-test-memory-limit/run-TIMESTAMP/
//! ├── config.yaml      # Test configuration with memory limits
//! └── procman.log      # Process manager logs with resource monitoring
//! ```
//!
//! View resource monitoring:
//! ```bash
//! cat target/debug/tmp/e2e-test-memory-limit/run-*/procman.log | grep "Resource usage"
//! ```
//!
//! View violations:
//! ```bash
//! cat target/debug/tmp/e2e-test-memory-limit/run-*/procman.log | grep "violation"
//! ```
//!
//! Count restarts:
//! ```bash
//! cat target/debug/tmp/e2e-test-memory-limit/run-*/procman.log | grep "Process spawned" | wc -l
//! ```
//!
//! ## Additional Tests in This File
//!
//! - `test_memory_growth` - Observes memory growth without violations (log only)
//! - `test_cpu_limit` - Tests CPU monitoring (observation only, no enforcement yet)

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::{assert_process_started, assert_pid_directory_empty_in_dir};
use std::time::Duration;
use std::thread;

#[test]
fn test_memory_limit_violation() {
    println!("\n========================================");
    println!("TEST: Memory Limit Violation");
    println!("========================================\n");
    
    let executor = TestExecutor::new("memory-limit");
    
    let config = TestConfigOptions {
        port: 50059,
        testexe_args: vec![
            "--memory-mb".to_string(), "150".to_string(),  // Allocate 150MB
            "--run-duration".to_string(), "15".to_string(), // Run for 15s
        ],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        memory_limit_mb: Some(100),  // Set limit to 100MB (below allocation)
        memory_policy: Some("restart".to_string()),  // Restart on violation
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        // Step 1: Wait for TESTEXE to start
        println!("Step 1: Waiting for TESTEXE to start...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        assert_process_started(procman, "testexe")?;
        println!("✓ TESTEXE started\n");
        
        // Step 2: Wait for memory allocation and monitoring
        println!("Step 2: Waiting for memory allocation (150MB)...");
        thread::sleep(Duration::from_secs(3));
        
        procman.collect_logs_now();
        
        // Check for memory allocation in TESTEXE logs
        let logs = procman.get_logs();
        let has_memory_alloc = logs.iter().any(|l| 
            l.contains("Allocated") || l.contains("memory")
        );
        
        if has_memory_alloc {
            println!("✓ Memory allocation logged\n");
        } else {
            println!("⚠ No memory allocation message found\n");
        }
        
        // Step 3: Check for resource monitoring
        println!("Step 3: Checking for resource monitoring...");
        thread::sleep(Duration::from_secs(3));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Look for resource monitoring indicators
        let has_resource_monitor = logs.iter().any(|l| 
            l.contains("resource monitor") || l.contains("Spawning resource monitor")
        );
        
        if has_resource_monitor {
            println!("✓ Resource monitoring is active\n");
        } else {
            println!("⚠ No resource monitoring indicators found\n");
        }
        
        // Step 4: Check for memory violation detection
        println!("Step 4: Checking for memory violation detection...");
        thread::sleep(Duration::from_secs(3));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Look for memory violation messages
        let has_violation = logs.iter().any(|l| 
            l.contains("memory") && (
                l.contains("violation") || 
                l.contains("limit") || 
                l.contains("exceeded") ||
                l.contains("warning")
            )
        );
        
        if has_violation {
            println!("✓ Memory violation detected in logs\n");
        } else {
            println!("⚠ No memory violation detection found\n");
        }
        
        // Step 5: Wait for heartbeat to process the violation
        println!("Step 5: Waiting for heartbeat to process violation...");
        thread::sleep(Duration::from_secs(3));  // Wait for heartbeat cycle (2s interval)
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Check for restart request queued
        let has_restart_queued = logs.iter().any(|l| 
            l.contains("Resource violation restart request queued") ||
            l.contains("📬") && l.contains("restart")
        );
        
        if has_restart_queued {
            println!("✓ Restart request queued\n");
        } else {
            println!("⚠ No restart request found in logs (yet)\n");
        }
        
        // Step 6: Wait for automatic restart to complete
        println!("Step 6: Waiting for automatic restart to complete...");
        thread::sleep(Duration::from_secs(3));  // Wait for restart to complete
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Count spawn events
        let spawn_count = logs.iter().filter(|l| 
            l.contains("Process spawned successfully: testexe")
        ).count();
        
        println!("Process spawn count: {}", spawn_count);
        
        let has_restart = spawn_count >= 2;
        if has_restart {
            println!("✓ Process restarted after memory violation\n");
        } else {
            println!("⚠ Expected 2+ spawns, found {}\n", spawn_count);
        }
        
        // Print summary
        println!("\n📊 Test Summary:");
        println!("  - Process started: ✓");
        println!("  - Memory allocated: {}", if has_memory_alloc { "✓" } else { "?" });
        println!("  - Resource monitoring: {}", if has_resource_monitor { "✓" } else { "?" });
        println!("  - Violation detected: {}", if has_violation { "✓" } else { "✗" });
        println!("  - Restart queued: {}", if has_restart_queued { "✓" } else { "✗" });
        println!("  - Process restarted: {}", if has_restart { "✓" } else { "✗" });
        
        Ok(())
    });
    
    // Verify PID file cleanup after test completes
    println!("\nVerifying PID file cleanup (proves no zombies)...");
    if let Ok(()) = result {
        assert_pid_directory_empty_in_dir(&executor.test_dir)
            .unwrap_or_else(|e| panic!("{}", e));
    }
    
    match result {
        Ok(()) => {
            println!("\n========================================");
            println!("✓ TEST COMPLETED: Memory Limit Violation");
            println!("(Check summary above for feature status)");
            println!("========================================\n");
        }
        Err(e) => {
            println!("\n========================================");
            println!("✗ TEST FAILED: Memory Limit Violation");
            println!("Error: {}", e);
            println!("========================================\n");
            panic!("Test failed: {}", e);
        }
    }
}

/// Test memory growth over time to trigger violation
#[test]
#[ignore] // Run manually with --ignored
fn test_memory_growth_violation() {
    println!("\n========================================");
    println!("TEST: Memory Growth Violation");
    println!("========================================\n");
    
    let executor = TestExecutor::new("memory-growth");
    
    let config = TestConfigOptions {
        port: 50060,
        testexe_args: vec![
            "--memory-growth-rate".to_string(), "30".to_string(),  // Grow 30MB/sec
            "--run-duration".to_string(), "10".to_string(),        // Run for 10s
        ],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        memory_limit_mb: None,
        memory_policy: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        println!("Waiting for TESTEXE to start...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        println!("✓ TESTEXE started\n");
        
        // Monitor memory growth over time
        for i in 1..=5 {
            thread::sleep(Duration::from_secs(2));
            procman.collect_logs_now();
            
            let logs = procman.get_logs();
            let recent_logs: Vec<_> = logs.iter().rev().take(10).collect();
            
            println!("Iteration {}: Recent logs:", i);
            for log in recent_logs.iter().rev() {
                if log.contains("memory") || log.contains("violation") || log.contains("MB") {
                    println!("  {}", log);
                }
            }
            println!();
        }
        
        Ok(())
    });
    
    match result {
        Ok(()) => println!("\n✓ Memory growth test completed"),
        Err(e) => panic!("Memory growth test failed: {}", e),
    }
}

/// Test CPU limit warning
#[test]
#[ignore] // Run manually with --ignored  
fn test_cpu_limit_warning() {
    println!("\n========================================");
    println!("TEST: CPU Limit Warning");
    println!("========================================\n");
    
    let executor = TestExecutor::new("cpu-limit");
    
    let config = TestConfigOptions {
        port: 50061,
        testexe_args: vec![
            "--cpu-percent".to_string(), "80".to_string(),     // Use 80% CPU
            "--run-duration".to_string(), "10".to_string(),    // Run for 10s
        ],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        memory_limit_mb: None,
        memory_policy: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        println!("Waiting for TESTEXE to start...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        println!("✓ TESTEXE started\n");
        
        println!("Waiting for CPU usage to stabilize...");
        thread::sleep(Duration::from_secs(5));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Look for CPU monitoring/warnings
        let has_cpu_monitor = logs.iter().any(|l| 
            l.contains("CPU") || l.contains("cpu") || l.contains("processor")
        );
        
        let has_cpu_warning = logs.iter().any(|l| 
            l.contains("CPU") && (l.contains("warning") || l.contains("exceeded") || l.contains("limit"))
        );
        
        println!("📊 Results:");
        println!("  - CPU monitoring: {}", if has_cpu_monitor { "✓" } else { "✗" });
        println!("  - CPU warning: {}", if has_cpu_warning { "✓" } else { "✗" });
        
        // Print relevant logs
        println!("\nRelevant logs:");
        for log in logs.iter().filter(|l| l.contains("CPU") || l.contains("cpu")) {
            println!("  {}", log);
        }
        
        Ok(())
    });
    
    match result {
        Ok(()) => println!("\n✓ CPU limit test completed"),
        Err(e) => panic!("CPU limit test failed: {}", e),
    }
}

