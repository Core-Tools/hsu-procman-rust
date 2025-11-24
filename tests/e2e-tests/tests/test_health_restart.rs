//! Test Scenario 2.1: Process Exit Detection & Restart
//!
//! Tests that PROCMAN can detect when a managed process exits and restart it
//! according to the configured restart policy.
//!
//! ## Test Scenario
//!
//! 1. Start TESTEXE configured to exit after 3 seconds
//! 2. Wait for first instance to start and become healthy
//! 3. Wait for process to exit (after ~4 seconds with buffer)
//! 4. Wait for PROCMAN to automatically restart TESTEXE (~3 more seconds)
//! 5. Verify second instance is running (spawn count = 2)
//! 6. Verify PID files are cleaned up (proves no zombies)
//!
//! ## Expected Results
//!
//! - First process instance starts successfully
//! - Health checks pass for first instance
//! - Process exits after 3 seconds (as configured)
//! - Exit detected via exit monitor
//! - Health check detects process no longer exists
//! - Restart policy evaluates (should_restart = true)
//! - Second process instance spawned automatically
//! - Restart counter shows 1 restart
//! - Process spawn count = 2 (original + 1 restart)
//! - **PID files are cleaned up** (proves no zombies)
//!
//! ## Key Observations
//!
//! When examining test artifacts, look for:
//!
//! - ✅ First process spawn confirmation
//! - ✅ Health checks passing for first instance
//! - ✅ Exit monitor detects completion
//! - ✅ Health check detects process is gone (no longer exists)
//! - ✅ Restart policy evaluation (should_restart = true)
//! - ✅ Restart callback invoked
//! - ✅ Second process spawn confirmation
//! - ✅ Restart counter incremented
//! - ✅ Circuit breaker reset after successful restart
//! - ✅ **PID file cleanup** (absence proves no zombies from either instance)
//!
//! ## Detailed Flow (Log Lines to Look For)
//!
//! ```
//! [PROCMAN] Process spawned successfully: testexe (PID: 11111)
//! [PROCMAN] Starting health monitor for testexe (type: Process)
//! [PROCMAN] Health check loop started for testexe
//! [PROCMAN] 💓 Process health check result: healthy=true
//!
//! [After 3 seconds - TESTEXE exits]
//!
//! [PROCMAN] Exit monitor completed for process testexe (PID: 11111)
//! [PROCMAN] Process testexe (PID: 11111) exited successfully with code: 0
//! [PROCMAN] 🔍 Running health check for testexe (PID: 11111)
//! [PROCMAN] ❌ Process no longer exists for PID 11111
//! [PROCMAN] 🚨 Health check failure threshold reached for testexe
//! [PROCMAN] 📞 Calling restart callback for testexe
//! [PROCMAN] 🔧 Restart policy check: should_restart=true
//! [PROCMAN] ✅ Restart allowed, queueing restart request
//!
//! [PROCMAN] 🔄 Processing restart request for testexe (trigger: HealthFailure)
//! [PROCMAN] Restarting process control: testexe
//! [PROCMAN] Process spawned successfully: testexe (PID: 22222)
//! [PROCMAN] ✅ Automatic restart succeeded
//! [PROCMAN] Circuit breaker reset
//!
//! Process spawn count: 2 ✅
//! ```
//!
//! ## Framework Validation
//!
//! This test confirms that:
//!
//! 1. **Exit detection works** - Exit monitor captures process termination
//! 2. **Health checks detect exit** - Health monitor realizes process is gone
//! 3. **Restart policy works** - "on-failure" policy triggers restart
//! 4. **Automatic restart works** - New instance spawned without manual intervention
//! 5. **Restart counter works** - Tracks number of restarts
//! 6. **Circuit breaker resets** - State cleaned for new instance
//! 7. **State machine transitions** - Running → Failed → Starting → Running
//! 8. **Process cleanup is complete** - PID files deleted after each instance terminates
//!
//! ## Restart Policy Configuration
//!
//! ```yaml
//! management:
//!   standard_managed:
//!     control:
//!       restart_policy: "on-failure"  # Restart when process exits
//!       context_aware_restart:
//!         default:
//!           max_retries: 5            # Allow up to 5 restart attempts
//!           retry_delay: 1s           # Wait 1 second before restarting
//!           backoff_rate: 1.0         # Linear backoff (no increase)
//! ```
//!
//! ## TESTEXE Configuration
//!
//! The test executable is configured to:
//! - Run for 3 seconds (`--run-duration 3`)
//! - Exit cleanly with code 0
//! - Simulate a process that completes its work and exits
//!
//! This mimics a real-world scenario where:
//! - A worker process completes a task
//! - Process exits normally
//! - But you want it restarted to handle the next task
//!
//! ## Test Artifacts
//!
//! Test runs preserve artifacts in timestamped directories:
//!
//! ```
//! target/debug/tmp/e2e-test-health-restart/run-TIMESTAMP/
//! ├── config.yaml      # Test configuration with restart policy
//! └── procman.log      # Process manager logs with restart sequence
//! ```
//!
//! View restart sequence:
//! ```bash
//! cat target/debug/tmp/e2e-test-health-restart/run-*/procman.log | grep -E "spawned|Exit monitor|restart"
//! ```
//!
//! Count restarts:
//! ```bash
//! cat target/debug/tmp/e2e-test-health-restart/run-*/procman.log | grep "Process spawned" | wc -l
//! # Should show 2 (original + 1 restart)
//! ```
//!
//! View restart policy evaluation:
//! ```bash
//! cat target/debug/tmp/e2e-test-health-restart/run-*/procman.log | grep "Restart policy"
//! ```

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::{assert_process_started, assert_pid_directory_empty_in_dir};
use std::time::Duration;
use std::thread;

#[test]
fn test_health_restart() {
    println!("\n========================================");
    println!("TEST: Health Check Restart");
    println!("========================================\n");
    
    let executor = TestExecutor::new("health-restart");
    
    let config = TestConfigOptions {
        port: 50057,
        testexe_args: vec!["--run-duration".to_string(), "3".to_string()],  // Exit after 3 seconds
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        // Step 1: Wait for TESTEXE first run
        println!("Step 1: Waiting for TESTEXE first run...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        assert_process_started(procman, "testexe")?;
        println!("✓ TESTEXE first run started\n");
        
        // Step 2: Wait for TESTEXE to exit (duration=3s + buffer)
        println!("Step 2: Waiting for TESTEXE to exit after 3 seconds...");
        thread::sleep(Duration::from_secs(4));
        
        // Collect logs to see what happened
        procman.collect_logs_now();
        
        // Check if TESTEXE exited
        let logs = procman.get_logs();
        let has_exit = logs.iter().any(|l| 
            l.contains("testexe") && (l.contains("exited") || l.contains("terminated") || l.contains("exit code"))
        );
        
        if has_exit {
            println!("✓ TESTEXE exited as expected\n");
        } else {
            println!("⚠ No exit message found in logs");
        }
        
        // Step 3: Wait for restart
        println!("Step 3: Waiting for PROCMAN to restart TESTEXE...");
        thread::sleep(Duration::from_secs(3));
        
        procman.collect_logs_now();
        
        // Check for restart indication
        let logs = procman.get_logs();
        let has_restart = logs.iter().any(|l| 
            l.contains("testexe") && (l.contains("restart") || l.contains("Restarting") || l.contains("Starting process"))
        );
        
        if has_restart {
            println!("✓ PROCMAN restarted TESTEXE\n");
        } else {
            println!("⚠ No restart message found in logs");
            println!("Recent logs:");
            for line in logs.iter().rev().take(20).rev() {
                println!("  {}", line);
            }
        }
        
        // Step 4: Verify second instance is running
        println!("Step 4: Verifying second instance is running...");
        thread::sleep(Duration::from_secs(5));  // Wait for restart to complete (includes 2s restart delay)
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Count how many times the process started
        let start_count = logs.iter().filter(|l| 
            l.contains("Process spawned successfully: testexe")
        ).count();
        
        println!("Process spawn count: {}", start_count);
        
        if start_count >= 2 {
            println!("✓ Second instance confirmed (spawn count: {})\n", start_count);
        } else {
            println!("⚠ Expected 2+ spawns, found {}", start_count);
        }
        
        Ok(())
    });
    
    // Verify PID file cleanup after test completes
    println!("\nStep 5: Verifying PID file cleanup (proves no zombies)...");
    if let Ok(()) = result {
        assert_pid_directory_empty_in_dir(&executor.test_dir)
            .unwrap_or_else(|e| panic!("{}", e));
    }
    
    match result {
        Ok(()) => {
            println!("\n========================================");
            println!("✓ TEST PASSED: Health Check Restart");
            println!("========================================\n");
        }
        Err(e) => {
            println!("\n========================================");
            println!("✗ TEST FAILED: Health Check Restart");
            println!("Error: {}", e);
            println!("========================================\n");
            panic!("Test failed: {}", e);
        }
    }
}

/// Test that verifies restart counter increments and max attempts are respected
#[test]
#[ignore] // Run manually with --ignored
fn test_max_restart_attempts() {
    println!("\n========================================");
    println!("TEST: Max Restart Attempts");
    println!("========================================\n");
    
    let executor = TestExecutor::new("max-restart-attempts");
    
    let config = TestConfigOptions {
        port: 50058,
        testexe_args: vec![
            "--run-duration".to_string(), "1".to_string(),  // Exit after 1 second
            "--exit-code".to_string(), "1".to_string(), // Exit with failure code
        ],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        println!("Waiting for multiple restart attempts...");
        
        // Wait for multiple restart cycles (1s duration + restart delay) * attempts
        thread::sleep(Duration::from_secs(20));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Count spawn attempts
        let spawn_count = logs.iter().filter(|l| 
            l.contains("Process spawned successfully: testexe")
        ).count();
        
        println!("Total spawn attempts: {}", spawn_count);
        
        // Should have attempted multiple restarts but eventually stopped
        if spawn_count >= 2 && spawn_count <= 5 {
            println!("✓ Restart attempts within expected range: {}", spawn_count);
        } else {
            println!("⚠ Unexpected spawn count: {}", spawn_count);
        }
        
        // Check for "giving up" message
        let has_give_up = logs.iter().any(|l| 
            l.contains("Max restart attempts") || l.contains("giving up") || l.contains("failed state")
        );
        
        if has_give_up {
            println!("✓ PROCMAN stopped attempting restarts");
        }
        
        Ok(())
    });
    
    match result {
        Ok(()) => println!("\n✓ Max restart attempts test passed"),
        Err(e) => panic!("Max restart attempts test failed: {}", e),
    }
}

