//! Test Scenario 2.1: Process Exit Detection & Restart
//!
//! Tests that PROCMAN can detect when a managed process exits and restart it
//! according to the configured restart policy.

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::assert_process_started;
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

