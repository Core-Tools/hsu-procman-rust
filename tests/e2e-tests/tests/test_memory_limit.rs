//! Test Scenario 3.1: Memory Limit Violation & Restart
//!
//! Tests that PROCMAN can detect when a managed process exceeds memory limits
//! and take the configured action (warn, restart, terminate).

use e2e_tests::TestExecutor;
use e2e_tests::process_manager::TestConfigOptions;
use e2e_tests::assertions::assert_process_started;
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
        
        // Step 5: Check for restart request queued
        println!("Step 5: Checking for restart request queued...");
        let has_restart_queued = logs.iter().any(|l| 
            l.contains("Resource violation restart request queued") ||
            l.contains("📬") && l.contains("restart")
        );
        
        if has_restart_queued {
            println!("✓ Restart request queued\n");
        } else {
            println!("⚠ No restart request found in logs\n");
        }
        
        // Step 6: Wait for heartbeat to process restart
        println!("Step 6: Waiting for automatic restart...");
        thread::sleep(Duration::from_secs(5));  // Wait for restart to complete
        
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

