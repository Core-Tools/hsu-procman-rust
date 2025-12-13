//! Demonstrative E2E tests comparing procman-v1 vs procman-v2 behavior.
//!
//! These tests run against the *library* `hsu-process-management::ProcessManager`
//! (not the `hsu-process-manager` binary) so we can issue concurrent commands
//! deterministically and measure latency precisely while still spawning real
//! child processes (`testexe`).
//!
//! Run v2 (default):
//! - `cargo test -p e2e-tests`
//!
//! Run v1:
//! - `cargo test -p e2e-tests --no-default-features --features procman-v1`

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

const QUERY_DEADLINE: Duration = Duration::from_secs(5);

// Test A thresholds
const V2_QUERY_MAX_MS_A: u128 = 250;
const V1_QUERY_MIN_MS_A: u128 = 1500;

// Test B thresholds
const V2_QUERY_MAX_MS_B: u128 = 400;
const V1_QUERY_MIN_MS_B: u128 = 1200;

fn make_std_managed_process(id: &str, testexe: &Path, args: Vec<String>) -> ProcessConfig {
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
                        wait_delay: Duration::from_millis(100),
                    },
                    process_file: None,
                    context_aware_restart: None,
                    restart_policy: Some("on-failure".to_string()),
                    limits: None,
                    graceful_timeout: Some(Duration::from_secs(8)),
                    log_collection: None,
                },
                health_check: None,
            }),
            integrated_managed: None,
            unmanaged: None,
        },
    }
}

async fn wait_for_ready_file(path: &Path, timeout_dur: Duration) {
    let deadline = Instant::now() + timeout_dur;
    loop {
        if tokio::fs::metadata(path).await.is_ok() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("Ready file was not created in time: {}", path.display());
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn write_text_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("Failed to create parent directories");
    }
    tokio::fs::write(path, contents)
        .await
        .expect("Failed to write file");
}

fn procman_impl_name() -> &'static str {
    if cfg!(feature = "procman-v1") {
        "procman-v1"
    } else if cfg!(feature = "procman-v2") {
        "procman-v2"
    } else {
        "unknown"
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_stop_of_slow_process_should_not_delay_querying_fast_process() {
    let test_dir = create_test_dir("procman-v1-vs-v2-a");
    let testexe = get_testexe_path();

    let slow_ready = test_dir.join("slow.ready");
    let slow_ack = test_dir.join("slow.ack");
    let slow_gate = test_dir.join("slow.gate");
    let fast_ready = test_dir.join("fast.ready");

    let slow_args = vec![
        "--ready-file".to_string(),
        slow_ready.to_string_lossy().to_string(),
        "--signal-ack-file".to_string(),
        slow_ack.to_string_lossy().to_string(),
        "--shutdown-gate-file".to_string(),
        slow_gate.to_string_lossy().to_string(),
        "--stop-block-ms".to_string(),
        "2000".to_string(),
    ];
    let fast_args = vec![
        "--ready-file".to_string(),
        fast_ready.to_string_lossy().to_string(),
    ];

    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: 50055,
            log_level: "info".to_string(),
            force_shutdown_timeout: Duration::from_secs(30),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes: vec![
            make_std_managed_process("slow", &testexe, slow_args),
            make_std_managed_process("fast", &testexe, fast_args),
        ],
        log_collection: None,
    };

    let mut manager = ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager");
    manager.start().await.expect("Failed to start ProcessManager");

    wait_for_ready_file(&slow_ready, Duration::from_secs(10)).await;
    wait_for_ready_file(&fast_ready, Duration::from_secs(10)).await;

    // Start the slow stop operation "in the background", then wait until the child
    // acknowledges receiving the shutdown signal. Only then measure query latency.
    //
    // This avoids relying on timing assumptions about when the child actually
    // receives the signal (especially on Windows).
    let (query_result, elapsed) = {
        #[cfg(feature = "procman-v2")]
        {
            let mgr_for_stop = manager.clone();
            let stop_task = tokio::spawn(async move { mgr_for_stop.stop_process("slow").await });

            wait_for_ready_file(&slow_ack, Duration::from_secs(2)).await;

            let t0 = Instant::now();
            let query_result = timeout(QUERY_DEADLINE, manager.get_all_process_info()).await;
            let elapsed = t0.elapsed();

            // Open the shutdown gate so stop can complete.
            write_text_file(&slow_gate, "go\n").await;

            let _ = timeout(Duration::from_secs(20), stop_task).await;
            (query_result, elapsed)
        }

        #[cfg(feature = "procman-v1")]
        {
            // procman-v1's ProcessManager is not Clone and the stop future borrows `manager`,
            // so we can't `tokio::spawn` it. We still run it concurrently by keeping it pinned
            // while we wait for the child's ack file and issue the query.
            let stop_fut = manager.stop_process("slow");
            tokio::pin!(stop_fut);

            // Ensure the stop future starts executing (and in v1, is likely holding the lock).
            let _ = timeout(Duration::from_millis(10), &mut stop_fut).await;

            wait_for_ready_file(&slow_ack, Duration::from_secs(2)).await;

            let t0 = Instant::now();
            let query_result = timeout(QUERY_DEADLINE, manager.get_all_process_info()).await;
            let elapsed = t0.elapsed();

            // Open the shutdown gate so stop can complete.
            write_text_file(&slow_gate, "go\n").await;

            let _ = timeout(Duration::from_secs(20), &mut stop_fut).await;
            (query_result, elapsed)
        }
    };

    // Ensure cleanup before assertions (so failures don't leak child processes).
    let _ = timeout(Duration::from_secs(20), manager.shutdown()).await;

    println!(
        "[{}] get_all_process_info latency: {}ms (result: {:?})",
        procman_impl_name(),
        elapsed.as_millis(),
        query_result.as_ref().map(|r| r.as_ref().map(|_| ()))
    );

    match query_result {
        Ok(Ok(_infos)) => {
            if cfg!(feature = "procman-v2") {
                assert!(
                    elapsed.as_millis() < V2_QUERY_MAX_MS_A,
                    "procman-v2 should stay responsive (<{}ms), got {}ms",
                    V2_QUERY_MAX_MS_A,
                    elapsed.as_millis()
                );
            } else {
                assert!(
                    elapsed.as_millis() >= V1_QUERY_MIN_MS_A,
                    "procman-v1 should show lock-induced latency (>= {}ms), got {}ms",
                    V1_QUERY_MIN_MS_A,
                    elapsed.as_millis()
                );
            }
        }
        Ok(Err(e)) => panic!("get_all_process_info returned error: {e:?}"),
        Err(_elapsed_timeout) => {
            // v1 may completely block; v2 should not.
            assert!(
                cfg!(feature = "procman-v1"),
                "procman-v2 query timed out unexpectedly"
            );
        }
    }
}

struct QueryStats {
    max_latency: Duration,
    timeouts: u32,
    samples: u32,
}

async fn query_loop(manager: &ProcessManager, deadline: Instant) -> QueryStats {
    let mut max_latency = Duration::ZERO;
    let mut timeouts_cnt = 0u32;
    let mut samples = 0u32;

    while Instant::now() < deadline {
        let t0 = Instant::now();
        let res = timeout(Duration::from_millis(2000), async {
            let _ = manager.get_all_process_info().await.unwrap();
        })
        .await;

        match res {
            Ok(()) => {
                let elapsed = t0.elapsed();
                if elapsed > max_latency {
                    max_latency = elapsed;
                }
                samples += 1;
            }
            Err(_) => {
                timeouts_cnt += 1;
            }
        }

        sleep(Duration::from_millis(25)).await;
    }

    QueryStats {
        max_latency,
        timeouts: timeouts_cnt,
        samples,
    }
}

async fn restart_fast_loop(manager: &ProcessManager, deadline: Instant, fast_ids: &[String]) {
    let mut idx = 0usize;
    while Instant::now() < deadline {
        let id = &fast_ids[idx % fast_ids.len()];
        idx += 1;

        let _ = timeout(Duration::from_secs(10), manager.restart_process(id, false)).await;
        sleep(Duration::from_millis(80)).await;
    }
}

async fn stop_start_slow_loop(manager: &ProcessManager, deadline: Instant, slow_ids: &[String]) {
    let mut idx = 0usize;
    while Instant::now() < deadline {
        let id = &slow_ids[idx % slow_ids.len()];
        idx += 1;

        let _ = timeout(Duration::from_secs(10), manager.stop_process(id)).await;
        let _ = timeout(Duration::from_secs(10), manager.start_process(id)).await;
        sleep(Duration::from_millis(120)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_concurrent_command_throughput_under_mixed_operations() {
    let test_dir = create_test_dir("procman-v1-vs-v2-b");
    let testexe = get_testexe_path();

    // Deterministic set: 8 processes (4 slow, 4 fast)
    let mut managed_processes = Vec::new();
    let mut slow_ids = Vec::new();
    let mut fast_ids = Vec::new();
    let mut ready_files = Vec::new();

    // One slow process is held in a deterministic "shutdown gate" to keep StopProcess in-flight.
    let gate_id = "slow-gate-0".to_string();
    let gate_ack = test_dir.join("slow-gate-0.ack");
    let gate_gate = test_dir.join("slow-gate-0.gate");

    for i in 0..4 {
        let id = if i == 0 { gate_id.clone() } else { format!("slow-{i}") };
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        if id != gate_id {
            slow_ids.push(id.clone());
        }

        let args = if id == gate_id {
            vec![
                "--ready-file".to_string(),
                ready.to_string_lossy().to_string(),
                "--signal-ack-file".to_string(),
                gate_ack.to_string_lossy().to_string(),
                "--shutdown-gate-file".to_string(),
                gate_gate.to_string_lossy().to_string(),
                "--stop-block-ms".to_string(),
                "500".to_string(),
            ]
        } else {
            vec![
                "--ready-file".to_string(),
                ready.to_string_lossy().to_string(),
                "--stop-block-ms".to_string(),
                "500".to_string(),
            ]
        };
        managed_processes.push(make_std_managed_process(&id, &testexe, args));
    }

    for i in 0..4 {
        let id = format!("fast-{i}");
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        fast_ids.push(id.clone());

        let args = vec![
            "--ready-file".to_string(),
            ready.to_string_lossy().to_string(),
        ];
        managed_processes.push(make_std_managed_process(&id, &testexe, args));
    }

    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: 50056,
            log_level: "info".to_string(),
            force_shutdown_timeout: Duration::from_secs(30),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes,
        log_collection: None,
    };

    let mut manager = ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager");
    manager.start().await.expect("Failed to start ProcessManager");

    for rf in &ready_files {
        wait_for_ready_file(rf, Duration::from_secs(15)).await;
    }

    // Start a StopProcess on the gated slow process and keep it blocked for ~1500ms of the 2s window.
    // This makes lock contention (v1) vs actor responsiveness (v2) deterministic.
    let ack_timeout = Duration::from_secs(2);
    let gate_hold = Duration::from_millis(1500);
    let window = Duration::from_secs(2);

    #[cfg(feature = "procman-v2")]
    let query_stats = {
        let mgr_for_stop = manager.clone();
        let gate_id_for_stop = gate_id.clone();
        let stop_task = tokio::spawn(async move { mgr_for_stop.stop_process(&gate_id_for_stop).await });

        wait_for_ready_file(&gate_ack, ack_timeout).await;

        let start = Instant::now();
        let deadline = start + window;

        // Gate opener: keep gate closed for ~1500ms.
        let gate_opener = async {
            sleep(gate_hold).await;
            write_text_file(&gate_gate, "go\n").await;
        };

        let (query_stats, _, _, _) = tokio::join!(
            query_loop(&manager, deadline),
            restart_fast_loop(&manager, deadline, &fast_ids),
            stop_start_slow_loop(&manager, deadline, &slow_ids),
            gate_opener,
        );

        // Ensure the gate is open even if the gate opener was delayed.
        if tokio::fs::metadata(&gate_gate).await.is_err() {
            write_text_file(&gate_gate, "go\n").await;
        }

        let _ = timeout(Duration::from_secs(20), stop_task).await;
        query_stats
    };

    #[cfg(feature = "procman-v1")]
    let query_stats = {
        let stop_fut = manager.stop_process(&gate_id);
        tokio::pin!(stop_fut);

        // Ensure the stop future starts executing (and is likely holding the lock).
        let _ = timeout(Duration::from_millis(10), &mut stop_fut).await;

        wait_for_ready_file(&gate_ack, ack_timeout).await;

        let start = Instant::now();
        let deadline = start + window;

        let gate_opener = async {
            sleep(gate_hold).await;
            write_text_file(&gate_gate, "go\n").await;
        };

        let (query_stats, _, _, _) = tokio::join!(
            query_loop(&manager, deadline),
            restart_fast_loop(&manager, deadline, &fast_ids),
            stop_start_slow_loop(&manager, deadline, &slow_ids),
            gate_opener,
        );

        if tokio::fs::metadata(&gate_gate).await.is_err() {
            write_text_file(&gate_gate, "go\n").await;
        }

        let _ = timeout(Duration::from_secs(20), &mut stop_fut).await;
        query_stats
    };

    // Cleanup before assertions.
    let _ = timeout(Duration::from_secs(30), manager.shutdown()).await;

    println!(
        "[{}] query samples={} timeouts={} max_latency={}ms",
        procman_impl_name(),
        query_stats.samples,
        query_stats.timeouts,
        query_stats.max_latency.as_millis()
    );

    if cfg!(feature = "procman-v2") {
        assert_eq!(
            query_stats.timeouts, 0,
            "procman-v2 should have zero query timeouts"
        );
        assert!(
            query_stats.max_latency.as_millis() < V2_QUERY_MAX_MS_B,
            "procman-v2 max query latency should be <{}ms, got {}ms",
            V2_QUERY_MAX_MS_B,
            query_stats.max_latency.as_millis()
        );
    } else {
        assert!(
            query_stats.timeouts > 0 || query_stats.max_latency.as_millis() > V1_QUERY_MIN_MS_B,
            "procman-v1 should exhibit timeouts or high latency (>{}ms). Got timeouts={}, max_latency={}ms",
            V1_QUERY_MIN_MS_B,
            query_stats.timeouts,
            query_stats.max_latency.as_millis()
        );
    }
}


