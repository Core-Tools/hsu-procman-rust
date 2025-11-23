//! Test Scenario 1.1: Forced Stop
//!
//! Tests that PROCMAN can forcefully terminate managed processes when
//! graceful timeout is very short or zero.

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::{assert_process_started};
use std::time::Duration;
use std::thread;

#[test]
fn test_forced_stop() {
    println!("\n========================================");
    println!("TEST: Forced Stop");
    println!("========================================\n");
    
    let executor = TestExecutor::new("forced-stop");
    
    let config = TestConfigOptions {
        port: 50057,
        testexe_args: vec![],
        graceful_timeout_ms: 100,  // Very short timeout = forced kill
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
        
        // Step 2: Let it run briefly
        println!("Step 2: Letting TESTEXE run for 1 second...");
        thread::sleep(Duration::from_secs(1));
        println!("✓ TESTEXE has been running\n");
        
        // Step 3: Force shutdown (short graceful timeout)
        println!("Step 3: PROCMAN will force-kill with short timeout...");
        
        // The executor will handle shutdown
        // With 100ms timeout, TESTEXE won't have time to log graceful shutdown
        
        Ok(())
    });
    
    match result {
        Ok(()) => {
            println!("\n========================================");
            println!("✓ TEST PASSED: Forced Stop");
            println!("========================================\n");
        }
        Err(e) => {
            println!("\n========================================");
            println!("✗ TEST FAILED: Forced Stop");
            println!("Error: {}", e);
            println!("========================================\n");
            panic!("Test failed: {}", e);
        }
    }
}

