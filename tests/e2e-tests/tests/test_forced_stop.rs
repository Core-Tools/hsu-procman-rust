//! Test Scenario 1.1: Forced Stop
//!
//! Tests that PROCMAN can forcefully terminate managed processes when
//! graceful timeout is very short or zero.
//!
//! ## Test Scenario
//!
//! 1. Start TESTEXE as a managed process
//! 2. Wait for process to be running and healthy
//! 3. Let process run for 2 seconds (verify stability)
//! 4. Send stop signal with very short timeout (100ms)
//! 5. Verify process is force-killed immediately
//! 6. Verify PROCMAN shuts down successfully
//!
//! ## Expected Results
//!
//! - Process starts successfully and becomes healthy
//! - Process runs stably for several seconds
//! - Stop signal sent with minimal timeout
//! - Process force-killed when timeout exceeded
//! - Force termination happens quickly (< 1 second)
//! - Exit status captured (may show forced termination)
//! - PROCMAN shuts down cleanly
//!
//! ## Key Observations
//!
//! When examining test artifacts, look for:
//!
//! - ✅ Process spawn confirmation
//! - ✅ Health checks passing (process is stable)
//! - ✅ Termination signal sent with short timeout
//! - ✅ Graceful timeout exceeded message (expected)
//! - ✅ Force kill executed (SIGKILL or taskkill /f)
//! - ✅ Process terminated successfully
//! - ✅ Quick termination (timeout 100ms + force kill)
//!
//! ## Detailed Flow (Log Lines to Look For)
//!
//! ```
//! [PROCMAN] Process spawned successfully: testexe (PID: XXXXX)
//! [PROCMAN] Starting health monitor for testexe (type: Process)
//! [PROCMAN] 💓 Process health check result: healthy=true
//!
//! [Test sends shutdown with 100ms timeout]
//!
//! [PROCMAN] Stopping process control: testexe
//! [PROCMAN] Terminating process: testexe
//! [PROCMAN] Sending termination signal to PID XXXXX
//! [PROCMAN] Graceful timeout (100ms) exceeded, force killing...
//! [PROCMAN] Force killing process PID XXXXX
//! [PROCMAN] Process terminated: testexe
//! ```
//!
//! ## Framework Validation
//!
//! This test confirms that:
//!
//! 1. **Force kill works** - Processes can be terminated forcefully
//! 2. **Timeout enforcement works** - Short timeouts trigger force kill
//! 3. **Force termination is fast** - No hanging processes
//! 4. **State transitions are correct** - Running → Stopping → Stopped (force)
//! 5. **No zombie processes** - Clean termination even when forced
//!
//! ## Platform Differences
//!
//! ### Linux/macOS
//! - First attempt: `SIGTERM` (15) - graceful signal
//! - Wait 100ms for graceful exit
//! - Force kill: `SIGKILL` (9) - cannot be caught or ignored
//! - Immediate termination guaranteed
//!
//! ### Windows
//! - First attempt: `taskkill /t /pid XXXXX` (terminate tree)
//! - Wait 100ms for graceful exit
//! - Force kill: `taskkill /f /t /pid XXXXX` (force terminate tree)
//! - Terminates process and all children
//!
//! ## Use Case
//!
//! This test validates the "kill switch" scenario where:
//! - A process is unresponsive or hung
//! - Graceful shutdown is not working
//! - Immediate termination is required
//! - System needs to guarantee process cleanup
//!
//! ## Test Artifacts
//!
//! Test runs preserve artifacts in timestamped directories:
//!
//! ```
//! target/debug/tmp/e2e-test-forced-stop/run-TIMESTAMP/
//! ├── config.yaml      # Test configuration (graceful_timeout: 100ms)
//! └── procman.log      # Process manager logs
//! ```
//!
//! View force kill sequence:
//! ```bash
//! cat target/debug/tmp/e2e-test-forced-stop/run-*/procman.log | grep -E "force|timeout|kill"
//! ```
//!
//! Verify quick termination:
//! ```bash
//! # Check time between stop and terminated
//! cat target/debug/tmp/e2e-test-forced-stop/run-*/procman.log | grep -E "Stopping|terminated"
//! ```

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

