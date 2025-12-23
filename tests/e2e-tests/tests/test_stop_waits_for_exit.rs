//! E2E: ProcessControlImpl::stop() waits for actual exit (deterministic).
//!
//! Uses `testexe` shutdown gate so the child will not exit until we open the gate.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use e2e_tests::{create_test_dir, get_testexe_path};
use hsu_process_management::{
    ExecutionConfig, ManagedProcessControlConfig, ProcessConfig, ProcessManagementConfig,
    ProcessManagementType, ProcessManager, ProcessManagerConfig, ProcessManagerOptions,
    StandardManagedProcessConfig,
};
use tokio::time::{sleep, timeout, Instant};

fn make_std_managed_process(id: &str, testexe: &Path, args: Vec<String>, graceful_timeout: Duration) -> ProcessConfig {
    ProcessConfig {
        id: id.to_string(),
        process_type: ProcessManagementType::StandardManaged,
        profile_type: "test".to_string(),
        enabled: true,
        management: ProcessManagementConfig {
            standard_managed: Some(StandardManagedProcessConfig {
                metadata: None,
                control: ManagedProcessControlConfig {
                    execution: ExecutionConfig {
                        executable_path: testexe.to_string_lossy().to_string(),
                        args,
                        working_directory: None,
                        environment: HashMap::new(),
                        wait_delay: Duration::from_millis(50),
                    },
                    process_file: None,
                    context_aware_restart: None,
                    restart_policy: Some("on-failure".to_string()),
                    limits: None,
                    graceful_timeout: Some(graceful_timeout),
                    log_collection: None,
                },
                health_check: None,
            }),
            integrated_managed: None,
            unmanaged: None,
        },
    }
}

async fn wait_for_file(path: &Path, timeout_dur: Duration) {
    let deadline = Instant::now() + timeout_dur;
    loop {
        if tokio::fs::metadata(path).await.is_ok() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("File was not created in time: {}", path.display());
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn write_text(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.unwrap();
    }
    tokio::fs::write(path, contents).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_waits_until_child_exits_or_gate_opens() {
    let test_dir = create_test_dir("stop-waits-for-exit");
    let testexe = get_testexe_path();

    let ready = test_dir.join("p.ready");
    let ack = test_dir.join("p.ack");
    let gate = test_dir.join("p.gate");

    let args = vec![
        "--ready-file".to_string(),
        ready.to_string_lossy().to_string(),
        "--signal-ack-file".to_string(),
        ack.to_string_lossy().to_string(),
        "--shutdown-gate-file".to_string(),
        gate.to_string_lossy().to_string(),
    ];

    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: 50123,
            log_level: "info".to_string(),
            force_shutdown_timeout: Duration::from_secs(30),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes: vec![make_std_managed_process(
            "p",
            &testexe,
            args,
            Duration::from_secs(3),
        )],
        log_collection: None,
    };

    let mut manager = ProcessManager::new(config).await.unwrap();
    manager.start().await.unwrap();

    wait_for_file(&ready, Duration::from_secs(10)).await;

    // Keep the stop future in a scope so the immutable borrow ends before shutdown()
    // (procman-v1 shutdown requires &mut self).
    {
        // Start stop, but keep it from completing by keeping the gate closed.
        let mut stop_fut = Box::pin(manager.stop_process("p"));

        // Kick it so the signal is sent.
        let _ = timeout(Duration::from_millis(10), stop_fut.as_mut()).await;

        // Wait for deterministic signal ack.
        wait_for_file(&ack, Duration::from_secs(2)).await;

        // Stop should NOT complete while the gate is closed (short timeout).
        assert!(
            timeout(Duration::from_millis(200), stop_fut.as_mut()).await.is_err(),
            "stop() completed even though shutdown gate was closed"
        );

        // Open the gate; now stop should complete quickly (well before graceful timeout).
        write_text(&gate, "go\n").await;
        timeout(Duration::from_secs(2), stop_fut.as_mut())
            .await
            .expect("stop() did not complete after gate opened")
            .expect("stop() returned error after gate opened");
    }

    manager.shutdown().await.unwrap();
}


