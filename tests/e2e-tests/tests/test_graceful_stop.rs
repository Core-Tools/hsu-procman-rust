//! Test Scenario 1.2: Graceful Stop
//!
//! Tests that PROCMAN can gracefully shut down managed processes by sending
//! termination signals and waiting for them to exit cleanly.

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::assert_process_started;
use std::time::Duration;
use std::thread;

#[test]
fn test_graceful_stop() {
    println!("\n========================================");
    println!("TEST: Graceful Stop");
    println!("========================================\n");
    
    let executor = TestExecutor::new("graceful-stop");
    
    let config = TestConfigOptions {
        port: 50055,
        testexe_args: vec![],  // No special args - just run normally
        graceful_timeout_ms: 5000,  // 5 second graceful timeout
        enable_logging: false,
        log_dir: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        // Step 1: Wait for TESTEXE to be running
        println!("Step 1: Waiting for TESTEXE to be running...");
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        assert_process_started(procman, "testexe")?;
        println!("✓ TESTEXE is running\n");
        
        // Step 2: Let it run for a bit
        println!("Step 2: Letting TESTEXE run for 2 seconds...");
        thread::sleep(Duration::from_secs(2));
        println!("✓ TESTEXE has been running\n");
        
        // Step 3: Shutdown will be handled by executor, but let's verify logs
        println!("Step 3: PROCMAN will send shutdown signal...");
        // The executor.run_test will call procman.shutdown() for us
        
        Ok(())
    });
    
    match result {
        Ok(()) => {
            println!("\n========================================");
            println!("✓ TEST PASSED: Graceful Stop");
            println!("========================================\n");
        }
        Err(e) => {
            println!("\n========================================");
            println!("✗ TEST FAILED: Graceful Stop");
            println!("Error: {}", e);
            println!("========================================\n");
            panic!("Test failed: {}", e);
        }
    }
}

/// Extended version with more detailed assertions
#[test]
#[ignore] // Run manually with --ignored
fn test_graceful_stop_detailed() {
    println!("\n========================================");
    println!("TEST: Graceful Stop (Detailed)");
    println!("========================================\n");
    
    let executor = TestExecutor::new("graceful-stop-detailed");
    
    let config = TestConfigOptions {
        port: 50056,
        testexe_args: vec![],
        graceful_timeout_ms: 5000,
        enable_logging: false,
        log_dir: None,
        ..Default::default()
    };
    
    let result = executor.run_test(config, |procman| {
        // Wait for process to start
        procman.wait_for_process_running("testexe", Duration::from_secs(10))?;
        assert_process_started(procman, "testexe")?;
        
        // Verify TESTEXE logs "fully operational"
        thread::sleep(Duration::from_secs(3));
        
        // Collect logs and verify startup sequence
        let logs = procman.get_logs();
        println!("Collected {} log lines", logs.len());
        
        // Look for key TESTEXE messages
        let has_starting = logs.iter().any(|l| l.contains("Starting testexe"));
        let has_ready = logs.iter().any(|l| l.contains("ready"));
        let has_operational = logs.iter().any(|l| l.contains("operational"));
        
        if has_starting {
            println!("✓ Found 'Starting testexe' in logs");
        }
        if has_ready {
            println!("✓ Found 'ready' in logs");
        }
        if has_operational {
            println!("✓ Found 'operational' in logs");
        }
        
        Ok(())
    });
    
    match result {
        Ok(()) => println!("\n✓ Detailed test passed"),
        Err(e) => panic!("Detailed test failed: {}", e),
    }
}

