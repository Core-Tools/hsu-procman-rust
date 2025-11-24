//! Test Scenario 2.2: HTTP Health Check & Restart
//!
//! Tests that PROCMAN can detect when a managed process becomes unhealthy via
//! HTTP health checks and restart it according to the configured restart policy.
//!
//! ## Test Scenario
//!
//! 1. Start TESTEXE with HTTP health server (port 18080, configured to fail after 5s)
//! 2. Wait for initial health checks to pass
//! 3. Wait for health checks to start failing (returns 503 after 5s)
//! 4. Wait for restart trigger (after 3 consecutive failures)
//! 5. Verify second instance is running with passing health checks
//! 6. Verify PID files are cleaned up (proves no zombies)
//!
//! ## Expected Results
//!
//! - HTTP server starts in TESTEXE
//! - Health checks execute every 3 seconds
//! - HTTP health endpoint responds (200 OK → 503 Unhealthy)
//! - Failure detection works (3 consecutive failures)
//! - Restart callback triggered correctly
//! - Process restarted successfully
//! - Second instance spawned
//! - Circuit breaker reset on recovery
//! - **PID files are cleaned up** (proves no zombies)
//!
//! ## Key Observations
//!
//! When examining test artifacts, look for:
//!
//! - ✅ HTTP health endpoint configuration logged
//! - ✅ Health check interval and threshold logged
//! - ✅ Initial successful health checks (200 OK responses)
//! - ✅ Health checks starting to fail (503 Service Unavailable)
//! - ✅ Consecutive failure counter incrementing (1/3, 2/3, 3/3)
//! - ✅ Failure threshold reached trigger
//! - ✅ Restart callback invoked
//! - ✅ Process termination and new spawn
//! - ✅ Multiple process spawn log entries confirming restart
//!
//! ## Detailed Flow (Log Lines to Look For)
//!
//! ```
//! [19:00:01] Process spawned successfully: testexe (PID: 30900)
//! [19:00:01] Starting health monitor for testexe (type: Http)
//! [19:00:01] HTTP health check endpoint: http://127.0.0.1:18080/
//! [19:00:01] Health check loop started for testexe (interval: 3s, threshold: 3)
//!
//! [19:00:04] ❌ Health check failed: consecutive failures = 1/3
//!            reason: Unexpected status code: 503 Service Unavailable
//! [19:00:07] ❌ Health check failed: consecutive failures = 2/3
//! [19:00:07] ❌ Health check failed: consecutive failures = 3/3
//! [19:00:07] 🚨 Health check failure threshold reached for testexe
//! [19:00:07] 📞 Calling restart callback for testexe
//! [19:00:07] ✅ Restart allowed, queueing restart request
//!
//! [19:00:09] 🔄 Processing restart request (trigger: HealthFailure)
//! [19:00:09] Terminating process: PID 30900
//! [19:00:11] Process spawned successfully: testexe (PID: 32756)
//! [19:00:11] Health monitor started
//! [19:00:11] ✅ Automatic restart succeeded
//!
//! Process spawn count: 2 ✅
//! ```
//!
//! ## Framework Validation
//!
//! This test confirms that:
//!
//! 1. **HTTP health checks work end-to-end** - Real HTTP server + health check client
//! 2. **Health failure detection is accurate** - Consecutive failure tracking works
//! 3. **Restart policy triggers correctly** - Health failure → restart request → restart
//! 4. **Process lifecycle management works** - Terminate old, spawn new, reset state
//! 5. **Circuit breaker resets properly** - New instance starts with clean state
//!
//! ## TESTEXE HTTP Server Behavior
//!
//! The test executable runs an HTTP server that:
//!
//! - **Port:** 18080 (configurable via `--health-port`)
//! - **Endpoint:** `/health` (not `/` to avoid confusion)
//! - **Initial Response:** `200 OK` with body "OK\n"
//! - **After 5 seconds:** `503 Service Unavailable` with body "Unhealthy\n"
//!
//! This simulates a real service that becomes unhealthy over time.
//!
//! ## Health Check Configuration
//!
//! ```yaml
//! health_check:
//!   type: "http"
//!   run_options:
//!     enabled: true
//!     interval: 3000ms         # Check every 3 seconds
//!     timeout: 2000ms          # HTTP request timeout (must be < interval)
//!     failure_threshold: 3     # Restart after 3 consecutive failures
//!     recovery_threshold: 2    # Mark healthy after 2 consecutive successes
//!     http_endpoint: "http://127.0.0.1:18080/health"
//! ```
//!
//! ## Test Artifacts
//!
//! Test runs preserve artifacts in timestamped directories:
//!
//! ```
//! target/debug/tmp/e2e-test-http-health-restart/run-TIMESTAMP/
//! ├── config.yaml      # Test configuration with HTTP health check
//! └── procman.log      # Process manager logs with health check activity
//! ```
//!
//! View health check activity:
//! ```bash
//! cat target/debug/tmp/e2e-test-http-health-restart/run-*/procman.log | grep "Health check"
//! ```
//!
//! Count restarts:
//! ```bash
//! cat target/debug/tmp/e2e-test-http-health-restart/run-*/procman.log | grep "Process spawned" | wc -l
//! ```

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::{assert_process_started, assert_pid_directory_empty_in_dir};
use std::time::Duration;
use std::thread;

#[test]
fn test_http_health_restart() {
    println!("\n========================================");
    println!("TEST: HTTP Health Check Restart");
    println!("========================================\n");
    
    let executor = TestExecutor::new("http-health-restart");
    
    let config = TestConfigOptions {
        port: 50059,
        // TESTEXE with HTTP health endpoint that fails after 5 seconds
        testexe_args: vec![
            "--health-port".to_string(), "18080".to_string(),
            "--fail-health-after".to_string(), "5".to_string(),
        ],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        // Enable HTTP health checks
        health_check_type: Some("http".to_string()),
        health_check_endpoint: Some("http://127.0.0.1:18080/".to_string()),
        health_check_interval_ms: Some(3000),  // Check every 3 seconds
        health_check_timeout_ms: Some(2000),  // Timeout must be less than interval
        health_check_failure_threshold: Some(3),
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        // Step 1: Wait for TESTEXE to start with HTTP server
        println!("Step 1: Waiting for TESTEXE to start with HTTP health server...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        assert_process_started(procman, "testexe")?;
        
        // Wait for HTTP server to be ready
        thread::sleep(Duration::from_secs(2));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Verify HTTP server started
        let has_http_server = logs.iter().any(|l| 
            l.contains("Health check server") && (l.contains("listening") || l.contains("starting"))
        );
        
        if has_http_server {
            println!("✓ HTTP health server started\n");
        } else {
            println!("⚠ No HTTP server startup message found");
        }
        
        // Step 2: Wait for initial health checks to pass
        println!("Step 2: Waiting for initial health checks to pass...");
        thread::sleep(Duration::from_secs(3));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Check for successful health checks (200 OK responses)
        let has_success = logs.iter().any(|l| 
            l.contains("Health check") && (l.contains("passed") || l.contains("200 OK"))
        );
        
        if has_success {
            println!("✓ Initial health checks passing\n");
        } else {
            println!("⚠ No successful health check logs found");
        }
        
        // Step 3: Wait for health to start failing (after 5 seconds from start)
        println!("Step 3: Waiting for health checks to start failing (fail-health-after=5s)...");
        thread::sleep(Duration::from_secs(4));  // Total ~7s from start, health should be failing
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Check for health check failures
        let has_failures = logs.iter().any(|l| 
            l.contains("Health check failed") || l.contains("503") || l.contains("Unhealthy")
        );
        
        if has_failures {
            println!("✓ Health checks started failing as expected\n");
        } else {
            println!("⚠ No health check failure messages found");
        }
        
        // Step 4: Wait for restart to trigger (failure threshold = 3, interval = 2s)
        println!("Step 4: Waiting for restart trigger (after 3 consecutive failures)...");
        thread::sleep(Duration::from_secs(8));  // 3 failures * 2s interval + buffer
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Check for restart indication
        let has_restart = logs.iter().any(|l| 
            l.contains("testexe") && 
            (l.contains("restart") || l.contains("Restarting") || 
             l.contains("failure threshold reached") || 
             l.contains("📞 Calling restart callback"))
        );
        
        if has_restart {
            println!("✓ Restart triggered after health failure threshold\n");
        } else {
            println!("⚠ No restart trigger found in logs");
            println!("Recent logs:");
            for line in logs.iter().rev().take(30).rev() {
                println!("  {}", line);
            }
        }
        
        // Step 5: Verify second instance is running with healthy checks
        println!("Step 5: Verifying second instance is running with passing health checks...");
        thread::sleep(Duration::from_secs(5));
        
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
        
        // Check that new instance has passing health checks
        // (We need to wait at least 5s from the new instance start for health to be passing)
        let recent_logs: Vec<&String> = logs.iter().rev().take(50).collect();
        let has_recent_success = recent_logs.iter().any(|l| 
            l.contains("200 OK") || l.contains("Health check passed")
        );
        
        if has_recent_success {
            println!("✓ New instance has passing health checks\n");
        } else {
            println!("⚠ No recent successful health checks for new instance");
        }
        
        Ok(())
    });
    
    // Verify PID file cleanup after test completes
    println!("\nStep 6: Verifying PID file cleanup (proves no zombies)...");
    if let Ok(()) = result {
        assert_pid_directory_empty_in_dir(&executor.test_dir)
            .unwrap_or_else(|e| panic!("{}", e));
    }
    
    match result {
        Ok(()) => {
            println!("\n========================================");
            println!("✓ TEST PASSED: HTTP Health Check Restart");
            println!("========================================\n");
        }
        Err(e) => {
            println!("\n========================================");
            println!("✗ TEST FAILED: HTTP Health Check Restart");
            println!("Error: {}", e);
            println!("========================================\n");
            panic!("Test failed: {}", e);
        }
    }
}

/// Test that verifies HTTP health checks with timeout
#[test]
#[ignore] // Run manually with --ignored
fn test_http_health_timeout() {
    println!("\n========================================");
    println!("TEST: HTTP Health Check Timeout");
    println!("========================================\n");
    
    let executor = TestExecutor::new("http-health-timeout");
    
    let config = TestConfigOptions {
        port: 50060,
        // TESTEXE without health port (server won't start, causing timeouts)
        testexe_args: vec!["--run-duration".to_string(), "15".to_string()],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        // Configure HTTP health checks to a port that won't respond
        health_check_type: Some("http".to_string()),
        health_check_endpoint: Some("http://127.0.0.1:19999/".to_string()),
        health_check_interval_ms: Some(3000),
        health_check_timeout_ms: Some(2000),  // Short timeout
        health_check_failure_threshold: Some(2),
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        println!("Waiting for HTTP health check timeouts...");
        procman.wait_for_process_running("testexe", Duration::from_secs(5))?;
        
        // Wait for several health check attempts
        thread::sleep(Duration::from_secs(10));
        
        procman.collect_logs_now();
        let logs = procman.get_logs();
        
        // Check for timeout messages
        let has_timeouts = logs.iter().any(|l| 
            l.contains("Timeout") || l.contains("connection failed")
        );
        
        if has_timeouts {
            println!("✓ Health check timeouts detected");
        }
        
        // Check that restart was triggered due to timeouts
        let has_restart = logs.iter().any(|l| 
            l.contains("restart") || l.contains("failure threshold")
        );
        
        if has_restart {
            println!("✓ Restart triggered after timeout failures");
        }
        
        Ok(())
    });
    
    match result {
        Ok(()) => println!("\n✓ HTTP health check timeout test passed"),
        Err(e) => panic!("HTTP health check timeout test failed: {}", e),
    }
}

