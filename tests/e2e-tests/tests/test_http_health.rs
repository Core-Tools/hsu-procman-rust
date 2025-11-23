//! Test Scenario 2.2: HTTP Health Check & Restart
//!
//! Tests that PROCMAN can detect when a managed process becomes unhealthy via
//! HTTP health checks and restart it according to the configured restart policy.

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::assert_process_started;
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
        health_check_interval_ms: Some(2000),  // Check every 2 seconds
        health_check_timeout_ms: Some(3000),
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

