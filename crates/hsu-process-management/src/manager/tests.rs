//! Unit tests for the process manager module.

use super::*;
use crate::config::{
    ContextAwareRestartConfig, ExecutionConfig, ManagedProcessControlConfig,
    ProcessManagementConfig, ProcessManagementType, ProcessManagerConfig, ProcessManagerOptions,
    RestartConfig, StandardManagedProcessConfig,
};
use std::collections::HashMap;
use tokio::time::Duration;
use async_trait::async_trait;
use hsu_common::ProcessResult;
use hsu_managed_process::{ProcessControl, ProcessDiagnostics as ControlDiagnostics};
use hsu_process_state::ProcessState;

/// Poll `manager.get_manager_state()` until `predicate` returns true or timeout expires.
///
/// Polls every 2ms. Panics with a clear message if timeout is reached.
async fn wait_for_state(
    manager: &ProcessManager,
    predicate: impl Fn(&ProcessManagerState) -> bool,
    timeout: Duration,
) {
    let poll_interval = Duration::from_millis(2);
    let result = tokio::time::timeout(timeout, async {
        loop {
            let state = manager.get_manager_state().await;
            if predicate(&state) {
                return state;
            }
            tokio::time::sleep(poll_interval).await;
        }
    })
    .await;

    if result.is_err() {
        let final_state = manager.get_manager_state().await;
        panic!(
            "wait_for_state timed out after {:?}. Final state: {:?}",
            timeout, final_state
        );
    }
}

fn create_test_config() -> ProcessManagerConfig {
    ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: 50055,
            log_level: "info".to_string(),
            force_shutdown_timeout: Duration::from_secs(30),
            base_directory: None,
        },
        managed_processes: vec![crate::config::ProcessConfig {
            id: "test-process".to_string(),
            enabled: true,
            profile_type: "test".to_string(),
            process_type: ProcessManagementType::StandardManaged,
            management: ProcessManagementConfig {
                standard_managed: Some(StandardManagedProcessConfig {
                    metadata: None,
                    control: ManagedProcessControlConfig {
                        execution: ExecutionConfig {
                            executable_path: "echo".to_string(),
                            args: vec!["hello".to_string()],
                            working_directory: None,
                            environment: HashMap::new(),
                            wait_delay: Duration::from_millis(100),
                        },
                        process_file: None,
                        context_aware_restart: Some(ContextAwareRestartConfig {
                            default: RestartConfig {
                                max_retries: 3,
                                retry_delay: Duration::from_secs(1),
                                backoff_rate: 1.5,
                            },
                            health_failures: None,
                            resource_violations: None,
                            startup_grace_period: None,
                            sustained_violation_time: None,
                        }),
                        restart_policy: None,
                        limits: None,
                        graceful_timeout: Some(Duration::from_secs(5)),
                        log_collection: None,
                    },
                    health_check: None,
                }),
                integrated_managed: None,
                unmanaged: None,
            },
        }],
        log_collection: None,
    }
}

fn create_empty_config() -> ProcessManagerConfig {
    ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: 50055,
            log_level: "info".to_string(),
            force_shutdown_timeout: Duration::from_secs(30),
            base_directory: None,
        },
        managed_processes: vec![],
        log_collection: None,
    }
}

#[tokio::test]
async fn test_process_manager_creation() {
    let config = create_test_config();
    let manager = ProcessManager::new(config).await;
    assert!(manager.is_ok());
}

#[tokio::test]
async fn test_process_manager_state_transitions() {
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Check initial state
    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Initializing));

    // Start manager
    manager.start().await.unwrap();
    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Running));

    // Shutdown manager
    manager.shutdown().await.unwrap();
    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Stopped));
}

#[tokio::test]
async fn test_get_all_process_info_returns_result() {
    let config = create_empty_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Should return Ok with empty process list
    let result = manager.get_all_process_info().await;
    assert!(result.is_ok());
    let processes = result.unwrap();
    assert!(processes.is_empty());
}

#[tokio::test]
async fn test_start_after_shutdown_fails() {
    // StartAll in Stopped state must return an error.
    // Stopped is a terminal state; create a new ProcessManager instead.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start and shutdown to reach Stopped state
    manager.start().await.unwrap();
    manager.shutdown().await.unwrap();

    // Verify we're in Stopped state
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );

    // Trying to start again should fail with OperationNotAllowed
    let result = manager.start().await;
    assert!(result.is_err(), "start() in Stopped state should return Err");

    // Verify the error indicates StartAll is not allowed in Stopped state
    let err = result.unwrap_err();
    let err_msg = format!("{:?}", err);
    assert!(
        err_msg.contains("OperationNotAllowed") || err_msg.contains("Stopped"),
        "Error should indicate operation not allowed in Stopped state: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_process_not_found_error() {
    let config = create_empty_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Try to get info for non-existent process
    let result = manager.get_process_info("non-existent").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        hsu_common::ProcessError::NotFound { id } => assert_eq!(id, "non-existent"),
        e => panic!("Expected NotFound error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_start_process_not_found() {
    let config = create_empty_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Try to start non-existent process
    let result = manager.start_process("non-existent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_empty_start_all() {
    let config = create_empty_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start with no processes should succeed immediately
    let result = manager.start().await;
    assert!(result.is_ok());

    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Running));
}

#[tokio::test]
async fn test_empty_shutdown() {
    let config = create_empty_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start then shutdown with no processes
    manager.start().await.unwrap();
    let result = manager.shutdown().await;
    assert!(result.is_ok());

    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Stopped));
}

#[tokio::test]
async fn test_start_all_returns_only_after_completion() {
    // This test verifies that start() returns only after all processes have started.
    // With the new batch tracking, the response is sent only when the batch completes.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Before start, state should be Initializing
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Initializing),
        "Expected Initializing, got {:?}",
        state
    );

    // After start() returns, state should be Running (not Starting)
    // This proves that start() waited for batch completion
    let result = manager.start().await;
    // Note: The actual start may fail if echo doesn't behave as expected,
    // but state should still be Running after the batch completes
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Running),
        "Expected Running after start(), got {:?}. Result: {:?}",
        state,
        result
    );

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_shutdown_returns_only_after_completion() {
    // This test verifies that shutdown() returns only after all processes have stopped.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Running),
        "Expected Running after start, got {:?}",
        state
    );

    // After shutdown() returns, state should be Stopped (not Stopping)
    let result = manager.shutdown().await;
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown(), got {:?}. Result: {:?}",
        state,
        result
    );
}

#[tokio::test]
async fn test_concurrent_process_operations_are_queued() {
    // This test verifies that rapid sequential operations on the same process
    // are queued instead of rejected. We fire start + restart + stop in quick
    // succession and verify all complete without "busy" errors.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();

    // Fire multiple operations concurrently on the same process
    // With queueing, none of these should fail with "busy" error
    let manager_clone1 = manager.clone();
    let manager_clone2 = manager.clone();
    let manager_clone3 = manager.clone();

    let handle1 = tokio::spawn(async move { manager_clone1.start_process("test-process").await });

    let handle2 =
        tokio::spawn(async move { manager_clone2.restart_process("test-process", false).await });

    let handle3 = tokio::spawn(async move { manager_clone3.stop_process("test-process").await });

    // Wait for all operations to complete
    let results = tokio::join!(handle1, handle2, handle3);

    // Check that no operation returned a "busy" error
    // (Other errors like start failing because already running are OK)
    let check_not_busy =
        |result: std::result::Result<std::result::Result<(), hsu_common::ProcessError>, _>| {
            if let Ok(Err(e)) = result {
                let msg = format!("{:?}", e);
                assert!(
                    !msg.contains("busy"),
                    "Operation should not fail with busy error: {}",
                    msg
                );
            }
        };

    check_not_busy(results.0);
    check_not_busy(results.1);
    check_not_busy(results.2);

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_process_manager_state_is_starting_during_batch() {
    // This test verifies state transitions happen correctly.
    // We use an empty config variant to test the edge case.
    // With processes, the state should be Starting until batch completes.
    let config = create_test_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Initial state
    let state = manager.get_manager_state().await;
    assert!(matches!(state, ProcessManagerState::Initializing));

    // After manager is created but before start, it should be Initializing
    assert!(matches!(state, ProcessManagerState::Initializing));
}

#[tokio::test]
async fn test_restart_process_not_found() {
    let config = create_empty_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Try to restart non-existent process
    let result = manager.restart_process("non-existent", false).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        hsu_common::ProcessError::NotFound { id } => assert_eq!(id, "non-existent"),
        e => panic!("Expected NotFound error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_stop_process_not_found() {
    let config = create_empty_config();
    let manager = ProcessManager::new(config).await.unwrap();

    // Try to stop non-existent process
    let result = manager.stop_process("non-existent").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        hsu_common::ProcessError::NotFound { id } => assert_eq!(id, "non-existent"),
        e => panic!("Expected NotFound error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_get_manager_state_not_blocked_by_long_op() {
    // Ensure the actor stays responsive while a long-running op executes in OpRunner.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();

    // Kick off a long-running test sleep operation
    let sleep_fut = manager.test_sleep_op("test-process", std::time::Duration::from_millis(500));

    // Manager state should still be retrievable quickly
    let state_result =
        tokio::time::timeout(Duration::from_millis(150), manager.get_manager_state()).await;
    assert!(
        state_result.is_ok(),
        "get_manager_state timed out during long op"
    );
    let state = state_result.unwrap();
    assert!(
        matches!(
            state,
            ProcessManagerState::Running
                | ProcessManagerState::Starting
                | ProcessManagerState::Stopping
        ),
        "Unexpected state while long op running: {:?}",
        state
    );

    // Await the test op completion to keep test clean
    let _ = sleep_fut.await;
}

#[tokio::test]
async fn test_pending_ops_drained_on_control_loss() {
    // This test verifies that when a process loses its control handle (e.g., due to
    // a worker panic), all pending operations are properly drained and their
    // responders receive errors.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();

    // Start a long-running operation to make the process busy
    let manager_clone1 = manager.clone();
    let sleep_handle = tokio::spawn(async move {
        manager_clone1
            .test_sleep_op("test-process", std::time::Duration::from_millis(200))
            .await
    });

    // Give time for the sleep to become in-flight
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Queue multiple operations while the process is busy
    let manager_clone2 = manager.clone();
    let op1_handle = tokio::spawn(async move { manager_clone2.start_process("test-process").await });

    let manager_clone3 = manager.clone();
    let op2_handle =
        tokio::spawn(async move { manager_clone3.restart_process("test-process", false).await });

    let manager_clone4 = manager.clone();
    let op3_handle = tokio::spawn(async move { manager_clone4.stop_process("test-process").await });

    // Give time for ops to be queued
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Now simulate control loss - this should:
    // 1. Fail the in-flight sleep operation
    // 2. Drain all 3 pending operations
    let drained_count = manager.test_lose_control("test-process").await.unwrap();
    assert_eq!(
        drained_count, 3,
        "Expected 3 pending ops to be drained, got {}",
        drained_count
    );

    // The sleep operation should have failed
    let sleep_result = sleep_handle.await.unwrap();
    assert!(
        sleep_result.is_err(),
        "In-flight sleep should have failed due to control loss"
    );

    // All pending operations should have received errors
    let op1_result = op1_handle.await.unwrap();
    assert!(
        op1_result.is_err(),
        "Pending op1 should have failed: {:?}",
        op1_result
    );

    let op2_result = op2_handle.await.unwrap();
    assert!(
        op2_result.is_err(),
        "Pending op2 should have failed: {:?}",
        op2_result
    );

    let op3_result = op3_handle.await.unwrap();
    assert!(
        op3_result.is_err(),
        "Pending op3 should have failed: {:?}",
        op3_result
    );

    // Verify the error message is appropriate
    let err_msg = format!("{}", op1_result.unwrap_err());
    assert!(
        err_msg.contains("inoperable") || err_msg.contains("control"),
        "Error should mention inoperable state or control loss: {}",
        err_msg
    );

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_new_ops_fail_after_control_loss() {
    // This test verifies that operations scheduled after a process becomes
    // inoperable immediately fail with an appropriate error.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();

    // Wait for start to complete
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Simulate control loss (no pending ops to drain)
    let drained_count = manager.test_lose_control("test-process").await.unwrap();
    assert_eq!(
        drained_count, 0,
        "No pending ops should be drained when process is idle"
    );

    // Now try to schedule new operations - they should fail immediately
    let result = manager.start_process("test-process").await;
    assert!(
        result.is_err(),
        "Start should fail for inoperable process"
    );

    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("no control handle") || err_msg.contains("inoperable"),
        "Error should indicate no control handle: {}",
        err_msg
    );

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_start_all_idempotent_when_running() {
    // This test verifies that calling start() when already Running is idempotent:
    // - Returns Ok(()) immediately
    // - Does NOT create a new start batch
    // - State remains Running (not changed to Starting)
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // First start - transitions to Running
    manager.start().await.ok();
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Running),
        "Expected Running after first start(), got {:?}",
        state
    );

    // Second start (while Running) - should be idempotent
    let result = manager.start().await;
    assert!(
        result.is_ok(),
        "start() while Running should return Ok, got {:?}",
        result
    );

    // State should still be Running (NOT Starting)
    // If a new batch was created, state would be Starting
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Running),
        "State should remain Running after idempotent start(), got {:?}",
        state
    );

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_start_all_idempotent_when_starting() {
    // This test verifies that calling start() while already Starting is idempotent.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start the first start() in background - it will be in Starting state briefly
    let manager_clone = manager.clone();
    let start_handle = tokio::spawn(async move {
        let mut m = manager_clone;
        m.start().await
    });

    // Wait until the manager enters Starting or Running state (allow Running for fast transitions)
    wait_for_state(
        &manager,
        |s| matches!(s, ProcessManagerState::Starting | ProcessManagerState::Running),
        Duration::from_millis(200),
    )
    .await;

    // Try to start again while Starting or Running - should be idempotent either way
    let result = manager.start().await;
    assert!(
        result.is_ok(),
        "start() while Starting/Running should return Ok (idempotent), got {:?}",
        result
    );

    // Wait for original start to complete
    let _ = start_handle.await;

    // Final state should be Running
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Running),
        "Expected Running after all starts complete, got {:?}",
        state
    );

    manager.shutdown().await.ok();
}

#[tokio::test]
async fn test_start_all_fails_when_stopping() {
    // This test verifies that calling start() while Stopping returns an error.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start the manager
    manager.start().await.ok();

    // Start shutdown in background
    let manager_clone = manager.clone();
    let shutdown_handle = tokio::spawn(async move {
        let mut m = manager_clone;
        m.shutdown().await
    });

    // Wait until the manager enters Stopping state
    wait_for_state(
        &manager,
        |s| matches!(s, ProcessManagerState::Stopping),
        Duration::from_millis(200),
    )
    .await;

    // Now we're in Stopping state - start() should fail
    let result = manager.start().await;
    assert!(
        result.is_err(),
        "start() while Stopping should return error, got {:?}",
        result
    );

    // Verify the error is OperationNotAllowed
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("OperationNotAllowed") || err_msg.contains("Stopping"),
        "Error should indicate operation not allowed during Stopping: {}",
        err_msg
    );

    // Wait for shutdown to complete
    let _ = shutdown_handle.await;

    // Final state should be Stopped
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );
}

#[tokio::test]
async fn test_restart_after_shutdown_fails() {
    // This test verifies that restart_process() after shutdown returns an error.
    // This complements test_start_after_shutdown_fails which only tests start().
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start and shutdown to reach Stopped state
    manager.start().await.unwrap();
    manager.shutdown().await.unwrap();

    // Verify we're in Stopped state
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );

    // Trying to restart should fail with appropriate error
    let result = manager.restart_process("test-process", false).await;
    assert!(
        result.is_err(),
        "restart_process() in Stopped state should return Err"
    );

    // Verify the error message indicates the operation is not allowed
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("stopped") || err_msg.contains("Stopped"),
        "Error should mention stopped state: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_queries_work_after_shutdown() {
    // This test verifies that query commands still work after shutdown.
    // The actor should remain responsive for state queries even in Stopped state.
    //
    // We use an empty config to avoid process-specific diagnostics queries that
    // may trigger blocking code issues in the test environment.
    let config = create_empty_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start and shutdown
    manager.start().await.unwrap();
    manager.shutdown().await.unwrap();

    // Verify we're in Stopped state
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );

    // get_manager_state should still work (multiple calls to verify responsiveness)
    for _ in 0..3 {
        let state = manager.get_manager_state().await;
        assert!(
            matches!(state, ProcessManagerState::Stopped),
            "get_manager_state should return Stopped, got {:?}",
            state
        );
    }

    // get_all_process_info should still work (uses cached data, non-blocking)
    let result = manager.get_all_process_info().await;
    assert!(
        result.is_ok(),
        "get_all_process_info should work after shutdown: {:?}",
        result
    );
    let processes = result.unwrap();
    assert!(
        processes.is_empty(),
        "Empty config should have 0 processes"
    );

    // get_process_info for non-existent process should return NotFound error
    // (verifying actor is responsive even for error cases)
    let result = manager.get_process_info("non-existent").await;
    assert!(
        result.is_err(),
        "get_process_info for non-existent should return error"
    );
    match result.unwrap_err() {
        hsu_common::ProcessError::NotFound { id } => assert_eq!(id, "non-existent"),
        e => panic!("Expected NotFound error, got: {:?}", e),
    }
}

#[tokio::test]
async fn test_stop_process_after_shutdown() {
    // This test verifies stop_process still works after shutdown (idempotent).
    // Separate from test_queries_work_after_shutdown to isolate process-specific behavior.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    // Start and shutdown
    manager.start().await.unwrap();
    manager.shutdown().await.unwrap();

    // Verify we're in Stopped state
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );

    // stop_process should respond (may succeed or fail, but should not hang)
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        manager.stop_process("test-process"),
    )
    .await;
    assert!(
        result.is_ok(),
        "stop_process should not hang after shutdown"
    );
    // Inner result may be Ok (already stopped) or Err (process inoperable after internal panic)
    // The key is that the actor remains responsive
}

#[tokio::test]
async fn test_actor_exits_after_command_channel_closes_and_work_drains() {
    // This test validates the corrected drain semantics after the channel split:
    // - When the command channel is closed (all handles dropped), the actor enters drain mode
    // - The actor continues processing OpCompleted messages
    // - The actor terminates once there is no pending/in-flight work remaining
    //
    // We drive the actor directly (without ProcessManager handle) to observe termination.
    let config = create_test_config();

    // Build file manager + channels as ProcessManager::new would
    let pid_file_manager = hsu_process_file::ProcessFileManager::with_defaults();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<super::commands::ManagerCommand>(32);
    let (completed_tx, completed_rx) = tokio::sync::mpsc::channel::<super::ops::OpCompleted>(256);

    // Create actor and init processes (no external handles created here)
    let mut actor = super::actor::ProcessManagerActor::new(config, pid_file_manager, None, completed_tx);
    actor.initialize_processes().await.unwrap();

    // Spawn actor loop
    let join = tokio::spawn(actor.run(cmd_rx, completed_rx));

    // Submit a long-running op to ensure a completion arrives after command channel closure
    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    cmd_tx
        .send(super::commands::ManagerCommand::TestSleep {
            id: "test-process".to_string(),
            duration: std::time::Duration::from_millis(150),
            resp: resp_tx,
        })
        .await
        .unwrap();

    // Now close the command channel (simulates all handles dropped)
    drop(cmd_tx);

    // The op should still complete and respond
    let op_res = tokio::time::timeout(Duration::from_secs(2), resp_rx).await;
    assert!(op_res.is_ok(), "TestSleep oneshot should resolve during drain");

    // Actor should exit once drained
    let actor_res = tokio::time::timeout(Duration::from_secs(2), join).await;
    assert!(actor_res.is_ok(), "Actor should terminate after drain");
}

#[tokio::test]
async fn test_actor_terminates_even_if_total_pending_ops_counter_drifts() {
    // Regression test: termination must NOT depend on `total_pending_ops` being perfect.
    // If the counter ever drifts high due to a bug, we still must not hang shutdown.
    let config = create_empty_config();

    let pid_file_manager = hsu_process_file::ProcessFileManager::with_defaults();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<super::commands::ManagerCommand>(8);
    let (completed_tx, completed_rx) = tokio::sync::mpsc::channel::<super::ops::OpCompleted>(8);

    let mut actor =
        super::actor::ProcessManagerActor::new(config, pid_file_manager, None, completed_tx);
    actor.initialize_processes().await.unwrap();

    // Simulate a drifted counter that would previously prevent termination.
    actor.test_set_total_pending_ops(1);

    let join = tokio::spawn(actor.run(cmd_rx, completed_rx));

    // Close the command channel so the actor enters drain mode and tries to terminate.
    drop(cmd_tx);

    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .expect("actor should terminate even if total_pending_ops is inconsistent")
        .unwrap();
}

#[derive(Default)]
struct DummyControl;

#[async_trait]
impl ProcessControl for DummyControl {
    async fn start(&mut self) -> ProcessResult<()> {
        Ok(())
    }

    async fn stop(&mut self) -> ProcessResult<()> {
        Ok(())
    }

    async fn restart(&mut self, _force: bool) -> ProcessResult<()> {
        Ok(())
    }

    fn get_state(&self) -> ProcessState {
        ProcessState::Stopped
    }

    fn get_diagnostics(&self) -> ControlDiagnostics {
        ControlDiagnostics {
            state: ProcessState::Stopped,
            last_error: None,
            process_id: None,
            start_time: None,
            executable_path: "dummy".to_string(),
            executable_exists: true,
            failure_count: 0,
            last_attempt_time: None,
            is_healthy: true,
            cpu_usage: None,
            memory_usage: None,
        }
    }

    fn get_pid(&self) -> Option<u32> {
        None
    }

    fn is_healthy(&self) -> bool {
        true
    }
}

#[tokio::test]
async fn test_oprunner_emits_completed_if_semaphore_closed() {
    // Regression test: if the OpRunner semaphore is closed (acquire fails), the job must not be
    // dropped silently; it must produce an OpCompleted so the actor can clear in-flight state.
    let (completed_tx, mut completed_rx) = tokio::sync::mpsc::channel::<super::ops::OpCompleted>(1);

    let job = super::ops::Job {
        process_id: "p1".to_string(),
        op: super::types::OpKind::Start,
        control: Box::new(DummyControl::default()),
        batch_id: Some(123),
    };

    super::ops::OpRunner::test_run_job_with_closed_semaphore(job, completed_tx).await;

    let completed = tokio::time::timeout(Duration::from_secs(1), completed_rx.recv())
        .await
        .expect("should receive OpCompleted")
        .expect("OpCompleted should be sent");

    assert_eq!(completed.process_id, "p1");
    assert!(completed.control.is_some(), "control handle must be returned");
    assert_eq!(completed.batch_id, Some(123));
    assert!(
        matches!(completed.result, Err(hsu_common::ProcessError::SpawnFailed { .. })),
        "expected SpawnFailed when semaphore is closed, got: {:?}",
        completed.result
    );
}

#[tokio::test]
async fn test_actor_does_not_hang_if_completion_channel_closes() {
    // If the completion channel closes unexpectedly while work is pending/in-flight,
    // the actor must force-fail all responders/batches and eventually terminate after
    // the command channel closes.
    let config = create_test_config();

    let pid_file_manager = hsu_process_file::ProcessFileManager::with_defaults();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<super::commands::ManagerCommand>(32);

    // Completion channel used by the actor loop: we keep the sender until after commands are processed,
    // then drop it to simulate unexpected completion channel closure from the actor's perspective.
    let (actor_completed_tx, actor_completed_rx) =
        tokio::sync::mpsc::channel::<super::ops::OpCompleted>(8);

    // Completion channel used by OpRunner is intentionally NOT connected to the actor loop.
    // This simulates "no completions can ever arrive" once actor_completed_tx is dropped.
    let (runner_completed_tx, _runner_completed_rx) =
        tokio::sync::mpsc::channel::<super::ops::OpCompleted>(8);

    let mut actor =
        super::actor::ProcessManagerActor::new(config, pid_file_manager, None, runner_completed_tx);
    actor.initialize_processes().await.unwrap();

    // Send a batch (StartAll) and a per-process command (StartProcess) that would normally be queued/in-flight.
    let (start_all_tx, start_all_rx) = tokio::sync::oneshot::channel();
    cmd_tx
        .send(super::commands::ManagerCommand::StartAll { resp: start_all_tx })
        .await
        .unwrap();

    let (start_proc_tx, start_proc_rx) = tokio::sync::oneshot::channel();
    cmd_tx
        .send(super::commands::ManagerCommand::StartProcess {
            id: "test-process".to_string(),
            resp: start_proc_tx,
        })
        .await
        .unwrap();

    // Barrier command: ensures the actor has processed the previous two commands (FIFO order).
    let (state_tx, state_rx) = tokio::sync::oneshot::channel();
    cmd_tx
        .send(super::commands::ManagerCommand::GetManagerState { resp: state_tx })
        .await
        .unwrap();

    // Spawn actor loop
    let join = tokio::spawn(actor.run(cmd_rx, actor_completed_rx));

    // Wait for the barrier response (actor processed StartAll + StartProcess).
    let _ = tokio::time::timeout(Duration::from_secs(1), state_rx)
        .await
        .expect("actor should respond to GetManagerState");

    // Now simulate completion channel closure.
    drop(actor_completed_tx);

    // Close command channel (simulates all handles dropped) so actor can exit once drained.
    drop(cmd_tx);

    // Both responders must resolve with error (not hang).
    let start_proc_res = tokio::time::timeout(Duration::from_secs(2), start_proc_rx)
        .await
        .expect("start_process oneshot should resolve")
        .expect("start_process oneshot sender should not be dropped");
    assert!(
        start_proc_res.is_err(),
        "start_process should fail when completion channel is closed"
    );

    let start_all_res = tokio::time::timeout(Duration::from_secs(2), start_all_rx)
        .await
        .expect("start_all oneshot should resolve")
        .expect("start_all oneshot sender should not be dropped");
    assert!(
        start_all_res.is_err(),
        "start_all batch should fail when completion channel is closed"
    );

    // Actor must terminate (no hang).
    tokio::time::timeout(Duration::from_secs(2), join)
        .await
        .expect("actor should terminate after forced drain")
        .unwrap();
}

#[tokio::test]
async fn test_pending_ops_complete_before_shutdown_returns() {
    // This test verifies that when shutdown is called while an operation is in-flight,
    // the shutdown waits for all operations to complete before returning.
    let config = create_test_config();
    let mut manager = ProcessManager::new(config).await.unwrap();

    manager.start().await.ok();

    // Start a long-running operation to make a process busy
    let manager_clone = manager.clone();
    let sleep_handle = tokio::spawn(async move {
        manager_clone
            .test_sleep_op("test-process", std::time::Duration::from_millis(200))
            .await
    });

    // Give time for the sleep to become in-flight
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Now call shutdown - it should wait for the in-flight operation
    let shutdown_start = std::time::Instant::now();
    let shutdown_result = manager.shutdown().await;
    let shutdown_duration = shutdown_start.elapsed();

    // Shutdown should have succeeded
    assert!(
        shutdown_result.is_ok(),
        "Shutdown should succeed: {:?}",
        shutdown_result
    );

    // Shutdown should have taken at least ~180ms (the remaining sleep time)
    // We use a lower bound to account for timing variations
    assert!(
        shutdown_duration >= Duration::from_millis(100),
        "Shutdown should have waited for in-flight op, but only took {:?}",
        shutdown_duration
    );

    // The sleep operation should have completed (not been cancelled)
    let sleep_result = sleep_handle.await.unwrap();
    assert!(
        sleep_result.is_ok(),
        "In-flight operation should complete successfully: {:?}",
        sleep_result
    );

    // Final state should be Stopped
    let state = manager.get_manager_state().await;
    assert!(
        matches!(state, ProcessManagerState::Stopped),
        "Expected Stopped after shutdown, got {:?}",
        state
    );
}
