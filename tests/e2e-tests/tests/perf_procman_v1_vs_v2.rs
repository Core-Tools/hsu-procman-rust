//! Performance E2E tests comparing procman-v1 vs procman-v2 behavior.
//!
//! These tests are **opt-in** and marked `#[ignore]` so they don't run in normal CI.
//!
//! ## Run Commands
//!
//! ### v2 (default):
//! ```bash
//! cargo test -p e2e-tests --test perf_procman_v1_vs_v2 -- --ignored --nocapture
//! ```
//!
//! ### v1:
//! ```bash
//! cargo test -p e2e-tests --no-default-features --features procman-v1 --test perf_procman_v1_vs_v2 -- --ignored --nocapture
//! ```
//!
//! ## Environment Variables
//!
//! - `HSU_E2E_PERF_N`: Number of processes (default: 32, max: 64)
//! - `HSU_E2E_PERF_WINDOW_MS`: Test window duration in ms (default: 2000)
//! - `HSU_E2E_PERF_TIMEOUT_MS`: Per-call timeout for queries in ms (default: 500)
//! - `HSU_E2E_PERF_STOP_TIMEOUT_MS`: Per-stop timeout for Scenario B in ms (default: 10000)
//! - `HSU_E2E_PERF_CONCURRENCY`: Max concurrent stop operations for v2 Scenario B (default: 8, max: 32)
//! - `HSU_E2E_PERF_JSON`: If "1", output results as JSON line
//!
//! ## Scenarios
//!
//! ### Scenario A: Gated Stop + Query Responsiveness
//!
//! Uses a "gated" process that blocks its shutdown until a gate file is created.
//! This keeps a StopProcess operation in-flight for a controlled duration.
//! - procman-v1: queries block because stop holds a global write lock across awaits
//! - procman-v2: queries remain responsive (actor-based, per-process serialization)
//!
//! ### Scenario B: Stop-All with Responsiveness Check
//!
//! Measures control-plane responsiveness while bulk stop-all is in-flight.
//! - procman-v2: runs concurrent stop-all (bounded concurrency) while continuously
//!   querying the manager. Asserts that queries stay responsive (no timeouts, low latency).
//!   Total stop-all time is printed as informational since OS shutdown cost dominates at high N.
//! - procman-v1: runs sequential stop-all (report-only, no strict assertions).
//!
//! ### Scenario C: Gated Stop + Churn
//!
//! Similar to Scenario A but with concurrent start/stop churn on non-gated processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "procman-v1")]
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
#[cfg(feature = "procman-v2")]
use std::sync::atomic::{AtomicBool, AtomicU32 as AtomicU32V2, Ordering as AtomicOrderingV2};
use std::sync::Arc;
use std::time::Duration;

use e2e_tests::{create_test_dir, get_testexe_path};
#[cfg(feature = "procman-v2")]
use futures::{stream, StreamExt};
use hsu_process_management::{
    ExecutionConfig, ManagedProcessControlConfig, ProcessConfig, ProcessManagementConfig,
    ProcessManagementType, ProcessManager, ProcessManagerConfig, ProcessManagerOptions,
    StandardManagedProcessConfig,
};
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Instant};

// =============================================================================
// Configuration & Environment
// =============================================================================

/// Default number of processes
const DEFAULT_N: usize = 32;
/// Maximum number of processes (to avoid resource abuse)
const MAX_N: usize = 64;
/// Default test window duration
const DEFAULT_WINDOW_MS: u64 = 2000;
/// Default per-call timeout for queries
const DEFAULT_TIMEOUT_MS: u64 = 500;
/// Default per-stop timeout for Scenario B
const DEFAULT_STOP_TIMEOUT_MS: u64 = 10000;
/// Default concurrency for v2 stop-all
const DEFAULT_CONCURRENCY: usize = 8;
/// Maximum concurrency for v2 stop-all
const MAX_CONCURRENCY: usize = 32;

/// v2 thresholds for query responsiveness (Scenarios A, B, C)
const V2_MAX_LATENCY_MS: u128 = 500;

struct PerfConfig {
    n: usize,
    window_ms: u64,
    timeout_ms: u64,
    stop_timeout_ms: u64,
    concurrency: usize,
    json_output: bool,
}

impl PerfConfig {
    fn from_env() -> Self {
        let n = std::env::var("HSU_E2E_PERF_N")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_N)
            .min(MAX_N);

        let window_ms = std::env::var("HSU_E2E_PERF_WINDOW_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_WINDOW_MS);

        let timeout_ms = std::env::var("HSU_E2E_PERF_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS);

        let stop_timeout_ms = std::env::var("HSU_E2E_PERF_STOP_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_STOP_TIMEOUT_MS);

        let concurrency = std::env::var("HSU_E2E_PERF_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_CONCURRENCY)
            .min(MAX_CONCURRENCY)
            .max(1);

        let json_output = std::env::var("HSU_E2E_PERF_JSON")
            .map(|s| s == "1")
            .unwrap_or(false);

        Self {
            n,
            window_ms,
            timeout_ms,
            stop_timeout_ms,
            concurrency,
            json_output,
        }
    }

    /// Calculate gate hold duration: keep the gate closed for most of the window
    fn gate_hold_ms(&self) -> u64 {
        // Hold gate for ~75% of window, but at least 500ms and leave 200ms margin
        let min_hold = 500u64;
        let max_hold = self.window_ms.saturating_sub(200);
        let target = (self.window_ms * 3) / 4;
        target.clamp(min_hold, max_hold)
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn procman_impl_name() -> &'static str {
    if cfg!(feature = "procman-v1") {
        "procman-v1"
    } else if cfg!(feature = "procman-v2") {
        "procman-v2"
    } else {
        "unknown"
    }
}

fn make_std_managed_process(
    id: &str,
    testexe: &Path,
    args: Vec<String>,
    graceful_timeout: Duration,
) -> ProcessConfig {
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
                    restart_policy: Some("never".to_string()),
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

/// Write text to a file, creating parent directories if needed
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

/// Wait for a file to exist, with timeout
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

async fn wait_for_all_ready(ready_files: &[PathBuf], timeout_dur: Duration) {
    for rf in ready_files {
        wait_for_file(rf, timeout_dur).await;
    }
}

// =============================================================================
// Stats Collection
// =============================================================================

#[derive(Debug, Clone, Default)]
struct QueryStats {
    samples: u32,
    max_latency_ms: u128,
    timeouts: u32,
}

/// Stats for stop-all operations (Scenario B)
#[derive(Debug, Clone, Default)]
struct StopAllStats {
    total_ms: u128,
    successes: u32,
    timeouts: u32,
    errors: u32,
    concurrency: usize,
}

/// Combined stats for Scenario B (stop-all + query responsiveness)
#[derive(Debug, Clone, Default)]
struct ScenarioBStats {
    stop_stats: StopAllStats,
    query_stats: QueryStats,
}

struct PerfResults {
    impl_name: &'static str,
    scenario: String,
    n: usize,
    window_ms: u64,
    gate_hold_ms: u64,
    samples: u32,
    max_latency_ms: u128,
    timeouts: u32,
    total_ms: u128,
}

impl PerfResults {
    fn print_summary(&self) {
        println!(
            "[{}] Scenario: {} | N={} | window={}ms | gate_hold={}ms | samples={} | max_latency={}ms | timeouts={} | total={}ms",
            self.impl_name,
            self.scenario,
            self.n,
            self.window_ms,
            self.gate_hold_ms,
            self.samples,
            self.max_latency_ms,
            self.timeouts,
            self.total_ms
        );
    }

    fn print_json(&self) {
        println!(
            r#"{{"impl":"{}","scenario":"{}","n":{},"window_ms":{},"gate_hold_ms":{},"samples":{},"max_latency_ms":{},"timeouts":{},"total_ms":{}}}"#,
            self.impl_name,
            self.scenario,
            self.n,
            self.window_ms,
            self.gate_hold_ms,
            self.samples,
            self.max_latency_ms,
            self.timeouts,
            self.total_ms
        );
    }

    fn print(&self, json_output: bool) {
        if json_output {
            self.print_json();
        } else {
            self.print_summary();
        }
    }
}

impl ScenarioBStats {
    fn print_summary(&self, impl_name: &str, n: usize) {
        println!(
            "[{}] Scenario: stop_all_responsiveness | N={} | concurrency={} | \
             stops: successes={} timeouts={} errors={} total={}ms | \
             queries: samples={} max_latency={}ms timeouts={}",
            impl_name,
            n,
            self.stop_stats.concurrency,
            self.stop_stats.successes,
            self.stop_stats.timeouts,
            self.stop_stats.errors,
            self.stop_stats.total_ms,
            self.query_stats.samples,
            self.query_stats.max_latency_ms,
            self.query_stats.timeouts
        );
    }

    fn print_json(&self, impl_name: &str, n: usize) {
        println!(
            r#"{{"impl":"{}","scenario":"stop_all_responsiveness","n":{},"concurrency":{},"stop_successes":{},"stop_timeouts":{},"stop_errors":{},"stop_total_ms":{},"query_samples":{},"query_max_latency_ms":{},"query_timeouts":{}}}"#,
            impl_name,
            n,
            self.stop_stats.concurrency,
            self.stop_stats.successes,
            self.stop_stats.timeouts,
            self.stop_stats.errors,
            self.stop_stats.total_ms,
            self.query_stats.samples,
            self.query_stats.max_latency_ms,
            self.query_stats.timeouts
        );
    }

    fn print(&self, impl_name: &str, n: usize, json_output: bool) {
        if json_output {
            self.print_json(impl_name, n);
        } else {
            self.print_summary(impl_name, n);
        }
    }
}

// =============================================================================
// Scenario A: Gated Stop + Query Responsiveness
// =============================================================================

/// Query loop: repeatedly call get_all_process_info() and collect stats
#[cfg(feature = "procman-v2")]
async fn query_loop(
    manager: &ProcessManager,
    deadline: Instant,
    per_call_timeout: Duration,
    stats: Arc<Mutex<QueryStats>>,
) {
    while Instant::now() < deadline {
        let t0 = Instant::now();
        let res = timeout(per_call_timeout, manager.get_all_process_info()).await;

        let mut stats = stats.lock().await;
        match res {
            Ok(Ok(_)) | Ok(Err(_)) => {
                let elapsed_ms = t0.elapsed().as_millis();
                stats.samples += 1;
                if elapsed_ms > stats.max_latency_ms {
                    stats.max_latency_ms = elapsed_ms;
                }
            }
            Err(_) => {
                stats.timeouts += 1;
            }
        }
        drop(stats);

        sleep(Duration::from_millis(10)).await;
    }
}

/// Query loop that runs until a stop flag is set
#[cfg(feature = "procman-v2")]
async fn query_loop_until_done(
    manager: &ProcessManager,
    stop_flag: Arc<AtomicBool>,
    per_call_timeout: Duration,
    stats: Arc<Mutex<QueryStats>>,
) {
    while !stop_flag.load(AtomicOrderingV2::Relaxed) {
        let t0 = Instant::now();
        let res = timeout(per_call_timeout, manager.get_all_process_info()).await;

        let mut stats = stats.lock().await;
        match res {
            Ok(Ok(_)) | Ok(Err(_)) => {
                let elapsed_ms = t0.elapsed().as_millis();
                stats.samples += 1;
                if elapsed_ms > stats.max_latency_ms {
                    stats.max_latency_ms = elapsed_ms;
                }
            }
            Err(_) => {
                stats.timeouts += 1;
            }
        }
        drop(stats);

        sleep(Duration::from_millis(10)).await;
    }
}

/// Stop storm loop: repeatedly call stop_process for non-gated process IDs
#[cfg(feature = "procman-v2")]
async fn stop_storm_loop(
    manager: &ProcessManager,
    deadline: Instant,
    process_ids: &[String],
    per_call_timeout: Duration,
) {
    let mut idx = 0usize;
    while Instant::now() < deadline {
        let id = &process_ids[idx % process_ids.len()];
        idx += 1;
        let _ = timeout(per_call_timeout, manager.stop_process(id)).await;
        sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore] // Opt-in: run with `-- --ignored`
async fn perf_scenario_a_stop_storm_responsiveness() {
    let cfg = PerfConfig::from_env();
    let test_dir = create_test_dir("perf-scenario-a");
    let testexe = get_testexe_path();
    let gate_hold_ms = cfg.gate_hold_ms();

    println!(
        "\n=== Scenario A: Gated Stop + Query Responsiveness (N={}, window={}ms, gate_hold={}ms) ===",
        cfg.n, cfg.window_ms, gate_hold_ms
    );

    // Gated process setup
    let gated_id = "proc-gated".to_string();
    let gated_ready = test_dir.join("gated.ready");
    let gated_ack = test_dir.join("gated.ack");
    let gated_gate = test_dir.join("gated.gate");

    let gated_args = vec![
        "--ready-file".to_string(),
        gated_ready.to_string_lossy().to_string(),
        "--signal-ack-file".to_string(),
        gated_ack.to_string_lossy().to_string(),
        "--shutdown-gate-file".to_string(),
        gated_gate.to_string_lossy().to_string(),
        "--shutdown-delay".to_string(),
        "0".to_string(),
    ];

    // Build N-1 regular processes + 1 gated process
    let mut managed_processes = Vec::new();
    let mut non_gated_ids = Vec::new();
    let mut ready_files = Vec::new();

    // Add gated process first
    managed_processes.push(make_std_managed_process(
        &gated_id,
        &testexe,
        gated_args,
        Duration::from_secs(30), // Long graceful timeout for gated process
    ));
    ready_files.push(gated_ready.clone());

    // Add regular processes
    let port_base = 51000;
    for i in 0..(cfg.n.saturating_sub(1)) {
        let id = format!("proc-{i}");
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        non_gated_ids.push(id.clone());

        let args = vec![
            "--ready-file".to_string(),
            ready.to_string_lossy().to_string(),
            "--shutdown-delay".to_string(),
            "0".to_string(),
        ];

        managed_processes.push(make_std_managed_process(
            &id,
            &testexe,
            args,
            Duration::from_secs(5),
        ));
    }

    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: port_base,
            log_level: "warn".to_string(),
            force_shutdown_timeout: Duration::from_secs(60),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes,
        log_collection: None,
    };

    // Create and start manager
    let mut manager = ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager");
    manager
        .start()
        .await
        .expect("Failed to start ProcessManager");

    // Wait for all ready files
    wait_for_all_ready(&ready_files, Duration::from_secs(30)).await;
    println!("All {} processes ready (including gated)", cfg.n);

    let window = Duration::from_millis(cfg.window_ms);
    let per_call_timeout = Duration::from_millis(cfg.timeout_ms);

    // Start the gated stop and run measurement
    let (final_stats, total_elapsed) = {
        #[cfg(feature = "procman-v2")]
        {
            let stats = Arc::new(Mutex::new(QueryStats::default()));

            // v2: spawn the gated stop, then run concurrent query + storm tasks
            let mgr_for_stop = manager.clone();
            let gated_id_clone = gated_id.clone();
            let stop_task = tokio::spawn(async move {
                mgr_for_stop.stop_process(&gated_id_clone).await
            });

            // Wait for the gated process to acknowledge the shutdown signal
            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            println!("Gated process acknowledged shutdown, stop is now in-flight");

            // Now run the measurement window
            let start = Instant::now();
            let deadline = start + window;

            let mgr_for_query = manager.clone();
            let mgr_for_storm = manager.clone();
            let stats_clone = stats.clone();
            let non_gated_ids_clone = non_gated_ids.clone();
            let gate_path = gated_gate.clone();
            let gate_hold = Duration::from_millis(gate_hold_ms);

            // Gate opener task: open gate after gate_hold_ms
            let gate_opener = tokio::spawn(async move {
                sleep(gate_hold).await;
                write_text_file(&gate_path, "go\n").await;
            });

            let query_task = tokio::spawn(async move {
                query_loop(&mgr_for_query, deadline, per_call_timeout, stats_clone).await
            });

            let storm_task = tokio::spawn(async move {
                stop_storm_loop(&mgr_for_storm, deadline, &non_gated_ids_clone, per_call_timeout).await
            });

            let _ = tokio::join!(query_task, storm_task, gate_opener);

            // Ensure gate is open
            if tokio::fs::metadata(&gated_gate).await.is_err() {
                write_text_file(&gated_gate, "go\n").await;
            }

            // Wait for gated stop to complete
            let _ = timeout(Duration::from_secs(30), stop_task).await;

            let total_elapsed = start.elapsed();
            let final_stats = stats.lock().await.clone();
            (final_stats, total_elapsed)
        }

        #[cfg(feature = "procman-v1")]
        {
            // v1: pin the gated stop future, then run query loop while it's in-flight
            // The gated stop holds the global write lock, blocking queries

            let stop_fut = manager.stop_process(&gated_id);
            tokio::pin!(stop_fut);

            // Kick the stop future to start execution (it will acquire the lock)
            let _ = timeout(Duration::from_millis(10), &mut stop_fut).await;

            // Wait for the gated process to acknowledge shutdown
            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            println!("Gated process acknowledged shutdown, stop is now in-flight (holding lock)");

            let start = Instant::now();
            let deadline = start + window;

            let stats_samples = Arc::new(AtomicU32::new(0));
            let stats_timeouts = Arc::new(AtomicU32::new(0));
            let stats_max_latency = Arc::new(Mutex::new(0u128));

            let gate_open_time = start + Duration::from_millis(gate_hold_ms);
            let mut gate_opened = false;

            // Query loop: attempt queries while the stop is in-flight
            while Instant::now() < deadline {
                // Open gate after gate_hold_ms
                if !gate_opened && Instant::now() >= gate_open_time {
                    write_text_file(&gated_gate, "go\n").await;
                    gate_opened = true;
                    println!("Gate opened after {}ms", gate_hold_ms);
                }

                // Attempt a query (this should block or timeout due to lock contention)
                let t0 = Instant::now();
                let res = timeout(per_call_timeout, manager.get_all_process_info()).await;

                match res {
                    Ok(Ok(_)) | Ok(Err(_)) => {
                        let elapsed_ms = t0.elapsed().as_millis();
                        stats_samples.fetch_add(1, AtomicOrdering::Relaxed);
                        let mut max = stats_max_latency.lock().await;
                        if elapsed_ms > *max {
                            *max = elapsed_ms;
                        }
                    }
                    Err(_) => {
                        stats_timeouts.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                }

                sleep(Duration::from_millis(10)).await;
            }

            // Ensure gate is open
            if !gate_opened {
                write_text_file(&gated_gate, "go\n").await;
            }

            // Complete the gated stop
            let _ = timeout(Duration::from_secs(30), &mut stop_fut).await;

            let total_elapsed = start.elapsed();
            let final_stats = QueryStats {
                samples: stats_samples.load(AtomicOrdering::Relaxed),
                timeouts: stats_timeouts.load(AtomicOrdering::Relaxed),
                max_latency_ms: *stats_max_latency.lock().await,
            };
            (final_stats, total_elapsed)
        }
    };

    // Cleanup
    let _ = timeout(Duration::from_secs(60), manager.shutdown()).await;

    let results = PerfResults {
        impl_name: procman_impl_name(),
        scenario: "gated_stop_responsiveness".to_string(),
        n: cfg.n,
        window_ms: cfg.window_ms,
        gate_hold_ms,
        samples: final_stats.samples,
        max_latency_ms: final_stats.max_latency_ms,
        timeouts: final_stats.timeouts,
        total_ms: total_elapsed.as_millis(),
    };

    results.print(cfg.json_output);

    // Assertions
    if cfg!(feature = "procman-v2") {
        assert_eq!(
            final_stats.timeouts, 0,
            "procman-v2 should have zero query timeouts while gated stop is in-flight"
        );
        assert!(
            final_stats.max_latency_ms < V2_MAX_LATENCY_MS,
            "procman-v2 max query latency should be <{}ms, got {}ms",
            V2_MAX_LATENCY_MS,
            final_stats.max_latency_ms
        );
    } else {
        // v1: must exhibit contention during the gate hold period
        // Either timeouts > 0 OR max_latency >= gate_hold_ms OR total significantly exceeds window
        let exhibits_contention = final_stats.timeouts > 0
            || final_stats.max_latency_ms >= (gate_hold_ms as u128)
            || total_elapsed.as_millis() > (cfg.window_ms as u128 + gate_hold_ms as u128);

        assert!(
            exhibits_contention,
            "procman-v1 should exhibit lock contention while gated stop is in-flight. \
             Expected: timeouts > 0 OR max_latency >= {}ms OR total > {}ms. \
             Got: max_latency={}ms, timeouts={}, total={}ms",
            gate_hold_ms,
            cfg.window_ms + gate_hold_ms,
            final_stats.max_latency_ms,
            final_stats.timeouts,
            total_elapsed.as_millis()
        );
    }
}

// =============================================================================
// Scenario B: Stop-All with Responsiveness Check
// =============================================================================

/// Build test processes for Scenario B (no gating needed)
fn build_scenario_b_processes(
    cfg: &PerfConfig,
    test_dir: &Path,
    testexe: &Path,
) -> (Vec<ProcessConfig>, Vec<String>, Vec<PathBuf>) {
    let mut managed_processes = Vec::new();
    let mut process_ids = Vec::new();
    let mut ready_files = Vec::new();

    for i in 0..cfg.n {
        let id = format!("proc-{i}");
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        process_ids.push(id.clone());

        let args = vec![
            "--ready-file".to_string(),
            ready.to_string_lossy().to_string(),
            "--shutdown-delay".to_string(),
            "0".to_string(),
        ];

        managed_processes.push(make_std_managed_process(
            &id,
            testexe,
            args,
            Duration::from_secs(5),
        ));
    }

    (managed_processes, process_ids, ready_files)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore] // Opt-in: run with `-- --ignored`
async fn perf_scenario_b_stop_all_responsiveness() {
    let cfg = PerfConfig::from_env();
    let test_dir = create_test_dir("perf-scenario-b");
    let testexe = get_testexe_path();

    let mode = if cfg!(feature = "procman-v2") {
        format!("concurrent, concurrency={}", cfg.concurrency)
    } else {
        "sequential".to_string()
    };

    println!(
        "\n=== Scenario B: Stop-All with Responsiveness ({}) (N={}) ===",
        mode, cfg.n
    );

    let (managed_processes, process_ids, ready_files) =
        build_scenario_b_processes(&cfg, &test_dir, &testexe);

    let port_base = 52000;
    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: port_base,
            log_level: "warn".to_string(),
            force_shutdown_timeout: Duration::from_secs(120),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes,
        log_collection: None,
    };

    // Create and start manager
    let mut manager = ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager");
    manager
        .start()
        .await
        .expect("Failed to start ProcessManager");

    // Wait for all ready files
    wait_for_all_ready(&ready_files, Duration::from_secs(60)).await;
    println!("All {} processes ready", cfg.n);

    let per_stop_timeout = Duration::from_millis(cfg.stop_timeout_ms);
    #[allow(unused_variables)]
    let per_query_timeout = Duration::from_millis(cfg.timeout_ms);

    // Measure stop-all with query responsiveness
    let stats = {
        #[cfg(feature = "procman-v2")]
        {
            let query_stats = Arc::new(Mutex::new(QueryStats::default()));
            let stop_done = Arc::new(AtomicBool::new(false));

            // Spawn query loop that runs until stop-all completes
            let mgr_for_query = manager.clone();
            let query_stats_clone = query_stats.clone();
            let stop_done_clone = stop_done.clone();
            let query_task = tokio::spawn(async move {
                query_loop_until_done(
                    &mgr_for_query,
                    stop_done_clone,
                    per_query_timeout,
                    query_stats_clone,
                )
                .await
            });

            // Run concurrent stop-all
            let start = Instant::now();

            let successes = Arc::new(AtomicU32V2::new(0));
            let timeouts = Arc::new(AtomicU32V2::new(0));
            let errors = Arc::new(AtomicU32V2::new(0));

            let _results: Vec<_> = stream::iter(process_ids.iter().cloned())
                .map(|id| {
                    let mgr = manager.clone();
                    let successes = successes.clone();
                    let timeouts = timeouts.clone();
                    let errors = errors.clone();
                    async move {
                        match timeout(per_stop_timeout, mgr.stop_process(&id)).await {
                            Ok(Ok(())) => {
                                successes.fetch_add(1, AtomicOrderingV2::Relaxed);
                            }
                            Ok(Err(_)) => {
                                errors.fetch_add(1, AtomicOrderingV2::Relaxed);
                            }
                            Err(_) => {
                                timeouts.fetch_add(1, AtomicOrderingV2::Relaxed);
                            }
                        }
                    }
                })
                .buffer_unordered(cfg.concurrency)
                .collect()
                .await;

            let total_ms = start.elapsed().as_millis();

            // Signal query loop to stop
            stop_done.store(true, AtomicOrderingV2::Relaxed);
            let _ = query_task.await;

            let final_query_stats = query_stats.lock().await.clone();

            ScenarioBStats {
                stop_stats: StopAllStats {
                    total_ms,
                    successes: successes.load(AtomicOrderingV2::Relaxed),
                    timeouts: timeouts.load(AtomicOrderingV2::Relaxed),
                    errors: errors.load(AtomicOrderingV2::Relaxed),
                    concurrency: cfg.concurrency,
                },
                query_stats: final_query_stats,
            }
        }

        #[cfg(feature = "procman-v1")]
        {
            // v1: sequential stop (serialized by global lock anyway)
            // No concurrent query loop since v1 would block anyway
            let start = Instant::now();
            let mut successes = 0u32;
            let mut timeouts_cnt = 0u32;
            let mut errors_cnt = 0u32;

            for id in &process_ids {
                match timeout(per_stop_timeout, manager.stop_process(id)).await {
                    Ok(Ok(())) => {
                        successes += 1;
                    }
                    Ok(Err(_)) => {
                        errors_cnt += 1;
                    }
                    Err(_) => {
                        timeouts_cnt += 1;
                    }
                }
            }

            let total_ms = start.elapsed().as_millis();
            ScenarioBStats {
                stop_stats: StopAllStats {
                    total_ms,
                    successes,
                    timeouts: timeouts_cnt,
                    errors: errors_cnt,
                    concurrency: 1, // Sequential
                },
                query_stats: QueryStats::default(), // Not measured for v1
            }
        }
    };

    // Cleanup
    let _ = timeout(Duration::from_secs(60), manager.shutdown()).await;

    // Print results
    stats.print(procman_impl_name(), cfg.n, cfg.json_output);

    // Assertions
    if cfg!(feature = "procman-v2") {
        // Assert stop-all made progress without per-stop timeouts
        assert_eq!(
            stats.stop_stats.timeouts, 0,
            "procman-v2 concurrent stop-all should have zero per-stop timeouts"
        );

        // Assert queries remained responsive during stop-all
        assert_eq!(
            stats.query_stats.timeouts, 0,
            "procman-v2 should have zero query timeouts during concurrent stop-all"
        );
        assert!(
            stats.query_stats.max_latency_ms < V2_MAX_LATENCY_MS,
            "procman-v2 max query latency should be <{}ms during stop-all, got {}ms",
            V2_MAX_LATENCY_MS,
            stats.query_stats.max_latency_ms
        );

        // Informational: print total time (no strict assertion)
        println!(
            "[procman-v2] Stop-all completed in {:.2}s (informational, OS overhead varies)",
            stats.stop_stats.total_ms as f64 / 1000.0
        );
    } else {
        // v1: report only, no strict threshold
        println!(
            "[procman-v1] Sequential stop-all: {} processes in {:.2}s (report only)",
            cfg.n,
            stats.stop_stats.total_ms as f64 / 1000.0
        );
    }
}

// =============================================================================
// Scenario C: Gated Stop + Churn
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore] // Opt-in: run with `-- --ignored`
async fn perf_scenario_c_query_under_churn() {
    let cfg = PerfConfig::from_env();
    let test_dir = create_test_dir("perf-scenario-c");
    let testexe = get_testexe_path();
    let gate_hold_ms = cfg.gate_hold_ms();

    // Use fewer non-gated processes for churn
    let n_non_gated = cfg.n.min(16).saturating_sub(1);
    let total_n = n_non_gated + 1; // +1 for gated process

    println!(
        "\n=== Scenario C: Gated Stop + Churn (N={}, window={}ms, gate_hold={}ms) ===",
        total_n, cfg.window_ms, gate_hold_ms
    );

    // Gated process setup
    let gated_id = "proc-gated".to_string();
    let gated_ready = test_dir.join("gated.ready");
    let gated_ack = test_dir.join("gated.ack");
    let gated_gate = test_dir.join("gated.gate");

    let gated_args = vec![
        "--ready-file".to_string(),
        gated_ready.to_string_lossy().to_string(),
        "--signal-ack-file".to_string(),
        gated_ack.to_string_lossy().to_string(),
        "--shutdown-gate-file".to_string(),
        gated_gate.to_string_lossy().to_string(),
        "--shutdown-delay".to_string(),
        "0".to_string(),
    ];

    let mut managed_processes = Vec::new();
    let mut non_gated_ids = Vec::new();
    let mut ready_files = Vec::new();

    // Add gated process
    managed_processes.push(make_std_managed_process(
        &gated_id,
        &testexe,
        gated_args,
        Duration::from_secs(30),
    ));
    ready_files.push(gated_ready.clone());

    // Add non-gated processes for churn
    let port_base = 53000;
    for i in 0..n_non_gated {
        let id = format!("proc-{i}");
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        non_gated_ids.push(id.clone());

        let args = vec![
            "--ready-file".to_string(),
            ready.to_string_lossy().to_string(),
            "--shutdown-delay".to_string(),
            "0".to_string(),
        ];

        managed_processes.push(make_std_managed_process(
            &id,
            &testexe,
            args,
            Duration::from_secs(3),
        ));
    }

    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port: port_base,
            log_level: "warn".to_string(),
            force_shutdown_timeout: Duration::from_secs(60),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes,
        log_collection: None,
    };

    // Create and start manager
    let mut manager = ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager");
    manager
        .start()
        .await
        .expect("Failed to start ProcessManager");

    // Wait for all ready files
    wait_for_all_ready(&ready_files, Duration::from_secs(30)).await;
    println!("All {} processes ready (including gated)", total_n);

    let window = Duration::from_millis(cfg.window_ms);
    let per_call_timeout = Duration::from_millis(cfg.timeout_ms);

    let (final_stats, total_elapsed) = {
        #[cfg(feature = "procman-v2")]
        {
            let stats = Arc::new(Mutex::new(QueryStats::default()));

            // v2: spawn gated stop, then run query + churn + gate opener concurrently
            let mgr_for_stop = manager.clone();
            let gated_id_clone = gated_id.clone();
            let stop_task = tokio::spawn(async move {
                mgr_for_stop.stop_process(&gated_id_clone).await
            });

            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            println!("Gated process acknowledged shutdown, stop is now in-flight");

            let start = Instant::now();
            let deadline = start + window;

            let mgr_for_query = manager.clone();
            let mgr_for_churn = manager.clone();
            let stats_clone = stats.clone();
            let non_gated_ids_clone = non_gated_ids.clone();
            let gate_path = gated_gate.clone();
            let gate_hold = Duration::from_millis(gate_hold_ms);

            let gate_opener = tokio::spawn(async move {
                sleep(gate_hold).await;
                write_text_file(&gate_path, "go\n").await;
            });

            let query_task = tokio::spawn(async move {
                query_loop(&mgr_for_query, deadline, per_call_timeout, stats_clone).await
            });

            let churn_task = tokio::spawn(async move {
                let mut idx = 0usize;
                while Instant::now() < deadline {
                    if non_gated_ids_clone.is_empty() {
                        sleep(Duration::from_millis(50)).await;
                        continue;
                    }
                    let id = &non_gated_ids_clone[idx % non_gated_ids_clone.len()];
                    idx += 1;
                    let _ = timeout(per_call_timeout, mgr_for_churn.stop_process(id)).await;
                    let _ = timeout(per_call_timeout, mgr_for_churn.start_process(id)).await;
                    sleep(Duration::from_millis(30)).await;
                }
            });

            let _ = tokio::join!(query_task, churn_task, gate_opener);

            if tokio::fs::metadata(&gated_gate).await.is_err() {
                write_text_file(&gated_gate, "go\n").await;
            }

            let _ = timeout(Duration::from_secs(30), stop_task).await;

            let total_elapsed = start.elapsed();
            let final_stats = stats.lock().await.clone();
            (final_stats, total_elapsed)
        }

        #[cfg(feature = "procman-v1")]
        {
            // v1: pin gated stop, run query loop while it holds the lock
            let stop_fut = manager.stop_process(&gated_id);
            tokio::pin!(stop_fut);

            let _ = timeout(Duration::from_millis(10), &mut stop_fut).await;

            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            println!("Gated process acknowledged shutdown, stop is now in-flight (holding lock)");

            let start = Instant::now();
            let deadline = start + window;

            let stats_samples = Arc::new(AtomicU32::new(0));
            let stats_timeouts = Arc::new(AtomicU32::new(0));
            let stats_max_latency = Arc::new(Mutex::new(0u128));

            let gate_open_time = start + Duration::from_millis(gate_hold_ms);
            let mut gate_opened = false;
            let mut idx = 0usize;

            while Instant::now() < deadline {
                if !gate_opened && Instant::now() >= gate_open_time {
                    write_text_file(&gated_gate, "go\n").await;
                    gate_opened = true;
                    println!("Gate opened after {}ms", gate_hold_ms);
                }

                // Attempt a query
                let t0 = Instant::now();
                let res = timeout(per_call_timeout, manager.get_all_process_info()).await;

                match res {
                    Ok(Ok(_)) | Ok(Err(_)) => {
                        let elapsed_ms = t0.elapsed().as_millis();
                        stats_samples.fetch_add(1, AtomicOrdering::Relaxed);
                        let mut max = stats_max_latency.lock().await;
                        if elapsed_ms > *max {
                            *max = elapsed_ms;
                        }
                    }
                    Err(_) => {
                        stats_timeouts.fetch_add(1, AtomicOrdering::Relaxed);
                    }
                }

                // Churn on non-gated processes (will block until stop completes in v1)
                if !non_gated_ids.is_empty() {
                    let id = &non_gated_ids[idx % non_gated_ids.len()];
                    idx += 1;
                    let _ = timeout(per_call_timeout, manager.stop_process(id)).await;
                    let _ = timeout(per_call_timeout, manager.start_process(id)).await;
                }

                sleep(Duration::from_millis(20)).await;
            }

            if !gate_opened {
                write_text_file(&gated_gate, "go\n").await;
            }

            let _ = timeout(Duration::from_secs(30), &mut stop_fut).await;

            let total_elapsed = start.elapsed();
            let final_stats = QueryStats {
                samples: stats_samples.load(AtomicOrdering::Relaxed),
                timeouts: stats_timeouts.load(AtomicOrdering::Relaxed),
                max_latency_ms: *stats_max_latency.lock().await,
            };
            (final_stats, total_elapsed)
        }
    };

    // Cleanup
    let _ = timeout(Duration::from_secs(60), manager.shutdown()).await;

    let results = PerfResults {
        impl_name: procman_impl_name(),
        scenario: "gated_stop_churn".to_string(),
        n: total_n,
        window_ms: cfg.window_ms,
        gate_hold_ms,
        samples: final_stats.samples,
        max_latency_ms: final_stats.max_latency_ms,
        timeouts: final_stats.timeouts,
        total_ms: total_elapsed.as_millis(),
    };

    results.print(cfg.json_output);

    // Assertions
    if cfg!(feature = "procman-v2") {
        assert_eq!(
            final_stats.timeouts, 0,
            "procman-v2 should have zero query timeouts under gated stop + churn"
        );
        assert!(
            final_stats.max_latency_ms < V2_MAX_LATENCY_MS,
            "procman-v2 max query latency should be <{}ms under churn, got {}ms",
            V2_MAX_LATENCY_MS,
            final_stats.max_latency_ms
        );
    } else {
        // v1: must exhibit contention during the gate hold period
        let exhibits_contention = final_stats.timeouts > 0
            || final_stats.max_latency_ms >= (gate_hold_ms as u128)
            || total_elapsed.as_millis() > (cfg.window_ms as u128 + gate_hold_ms as u128);

        assert!(
            exhibits_contention,
            "procman-v1 should exhibit lock contention under gated stop + churn. \
             Expected: timeouts > 0 OR max_latency >= {}ms OR total > {}ms. \
             Got: max_latency={}ms, timeouts={}, total={}ms",
            gate_hold_ms,
            cfg.window_ms + gate_hold_ms,
            final_stats.max_latency_ms,
            final_stats.timeouts,
            total_elapsed.as_millis()
        );
    }
}
