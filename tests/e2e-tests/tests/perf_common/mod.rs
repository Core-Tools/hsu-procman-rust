//! Shared helpers for performance E2E tests.
//!
//! **Goals**:
//! - Deterministic, low-flake perf measurements for technical demos.
//! - No port collisions across parallel test runs.
//! - Stable, single-line human output + a single JSON line output (always).
//! - Percentile latency reporting (p50/p95/p99/max) from per-call samples.

use std::collections::HashMap;
#[cfg(feature = "procman-v1")]
use std::future::Future;
use std::path::{Path, PathBuf};
#[cfg(feature = "procman-v1")]
use std::pin::Pin;
use std::time::Duration;

use tokio::time::{sleep, Instant};
use tokio::time::timeout;

use hsu_process_management::{
    ExecutionConfig, ManagedProcessControlConfig, ProcessConfig, ProcessManagementConfig,
    ProcessManagementType, ProcessManager, ProcessManagerConfig, ProcessManagerOptions,
    StandardManagedProcessConfig,
};

// =============================================================================
// Constants
// =============================================================================

/// Default number of processes
#[allow(dead_code)]
pub const DEFAULT_N: usize = 32;
/// Maximum number of processes (to avoid resource abuse)
#[allow(dead_code)]
pub const MAX_N: usize = 64;
/// Default test window duration in ms
#[allow(dead_code)]
pub const DEFAULT_WINDOW_MS: u64 = 2000;
/// Default warmup duration in ms (not measured).
#[allow(dead_code)]
pub const DEFAULT_WARMUP_MS: u64 = 200;
/// Default query pacing (sleep between query attempts).
#[allow(dead_code)]
pub const DEFAULT_QUERY_PACE_MS: u64 = 10;
/// Default churn pacing (sleep between churn cycle attempts).
#[allow(dead_code)]
pub const DEFAULT_CHURN_PACE_MS: u64 = 30;
/// Heartbeat settle window (gives background tasks a moment to start).
#[allow(dead_code)]
pub const HEARTBEAT_SETTLE_MS: u64 = 200;
/// Poll cadence for keeping a pinned v1 stop future alive in `tokio::select!` loops.
#[allow(dead_code)]
pub const STOP_POLL_PACE_MS: u64 = 20;
/// Warmup pacing between calls (kept small to avoid busy spin).
#[allow(dead_code)]
pub const WARMUP_PACE_MS: u64 = 10;
/// Default per-call timeout for queries in ms
#[allow(dead_code)]
pub const DEFAULT_TIMEOUT_MS: u64 = 1500;
/// Default per-stop timeout in ms
#[allow(dead_code)]
pub const DEFAULT_STOP_TIMEOUT_MS: u64 = 10000;
/// Cancellation probe: strict timeout per probe query.
#[allow(dead_code)]
pub const DEFAULT_CANCEL_PROBE_TIMEOUT_MS: u64 = 250;
/// Cancellation probe: number of immediate post-abort queries.
#[allow(dead_code)]
pub const DEFAULT_CANCEL_PROBE_COUNT: u32 = 3;
/// Default concurrency for concurrent operations
#[allow(dead_code)]
pub const DEFAULT_CONCURRENCY: usize = 8;
/// Maximum concurrency
#[allow(dead_code)]
pub const MAX_CONCURRENCY: usize = 32;
/// Default number of reader tasks
#[allow(dead_code)]
pub const DEFAULT_READERS: usize = 8;
/// Maximum number of reader tasks
#[allow(dead_code)]
pub const MAX_READERS: usize = 32;
/// Default number of writer tasks
#[allow(dead_code)]
pub const DEFAULT_WRITERS: usize = 2;
/// Maximum number of writer tasks
#[allow(dead_code)]
pub const MAX_WRITERS: usize = 8;

/// v2 thresholds for query responsiveness
#[allow(dead_code)]
pub const V2_MAX_LATENCY_MS: u128 = 500;

// =============================================================================
// Configuration
// =============================================================================

/// Performance test configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct PerfConfig {
    #[allow(dead_code)]
    pub n: usize,
    #[allow(dead_code)]
    pub window_ms: u64,
    #[allow(dead_code)]
    pub warmup_ms: u64,
    #[allow(dead_code)]
    pub query_pace_ms: u64,
    #[allow(dead_code)]
    pub churn_pace_ms: u64,
    #[allow(dead_code)]
    pub timeout_ms: u64,
    #[allow(dead_code)]
    pub stop_timeout_ms: u64,
    #[allow(dead_code)]
    pub concurrency: usize,
    #[allow(dead_code)]
    pub readers: usize,
    #[allow(dead_code)]
    pub writers: usize,
    #[allow(dead_code)]
    pub cancel_probe_timeout_ms: u64,
    #[allow(dead_code)]
    pub cancel_probe_count: u32,
}

impl PerfConfig {
    /// Load configuration from environment variables using global defaults.
    #[allow(dead_code)]
    pub fn from_env() -> Self {
        Self::from_env_with_defaults(DEFAULT_N, DEFAULT_WINDOW_MS)
    }

    /// Load configuration from environment variables with per-test documented defaults.
    ///
    /// Environment variables (when set) always override the provided defaults:
    /// - `HSU_E2E_PERF_N`
    /// - `HSU_E2E_PERF_WINDOW_MS`
    /// - `HSU_E2E_PERF_WARMUP_MS`
    /// - `HSU_E2E_PERF_TIMEOUT_MS`
    /// - `HSU_E2E_PERF_STOP_TIMEOUT_MS`
    /// - `HSU_E2E_PERF_CONCURRENCY`
    /// - `HSU_E2E_PERF_READERS`
    /// - `HSU_E2E_PERF_WRITERS`
    /// - `HSU_E2E_PERF_CANCEL_PROBE_TIMEOUT_MS`
    /// - `HSU_E2E_PERF_CANCEL_PROBE_COUNT`
    #[allow(dead_code)]
    pub fn from_env_with_defaults(default_n: usize, default_window_ms: u64) -> Self {
        let n = std::env::var("HSU_E2E_PERF_N")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(default_n)
            .min(MAX_N)
            .max(1);

        let window_ms = std::env::var("HSU_E2E_PERF_WINDOW_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(default_window_ms)
            .max(1);

        let warmup_ms = std::env::var("HSU_E2E_PERF_WARMUP_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_WARMUP_MS)
            .max(0);

        let query_pace_ms = std::env::var("HSU_E2E_PERF_QUERY_PACE_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_QUERY_PACE_MS)
            .max(1);

        let churn_pace_ms = std::env::var("HSU_E2E_PERF_CHURN_PACE_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CHURN_PACE_MS)
            .max(1);

        let timeout_ms = std::env::var("HSU_E2E_PERF_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1);

        let stop_timeout_ms = std::env::var("HSU_E2E_PERF_STOP_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_STOP_TIMEOUT_MS)
            .max(1);

        let concurrency = std::env::var("HSU_E2E_PERF_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_CONCURRENCY)
            .min(MAX_CONCURRENCY)
            .max(1);

        let readers = std::env::var("HSU_E2E_PERF_READERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_READERS)
            .min(MAX_READERS)
            .max(1);

        let writers = std::env::var("HSU_E2E_PERF_WRITERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(DEFAULT_WRITERS)
            .min(MAX_WRITERS)
            .max(1);

        let cancel_probe_timeout_ms = std::env::var("HSU_E2E_PERF_CANCEL_PROBE_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CANCEL_PROBE_TIMEOUT_MS)
            .max(1);

        let cancel_probe_count = std::env::var("HSU_E2E_PERF_CANCEL_PROBE_COUNT")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(DEFAULT_CANCEL_PROBE_COUNT)
            .max(1);

        Self {
            n,
            window_ms,
            warmup_ms,
            query_pace_ms,
            churn_pace_ms,
            timeout_ms,
            stop_timeout_ms,
            concurrency,
            readers,
            writers,
            cancel_probe_timeout_ms,
            cancel_probe_count,
        }
    }

    /// Calculate gate hold duration: keep the gate closed for most of the window
    pub fn gate_hold_ms(&self) -> u64 {
        // IMPORTANT: must never panic (clamp panics if max < min).
        // - If the window is too small, return a safe hold within the window.
        // - Otherwise, hold gate for ~75% of window, but at least 500ms and leave 200ms margin.
        let window_ms = self.window_ms.max(1);
        if window_ms <= 700 {
            return window_ms.saturating_sub(200).max(1);
        }

        let min_hold = 500u64;
        let max_hold = window_ms.saturating_sub(200);
        let target = (window_ms * 3) / 4;
        target.clamp(min_hold, max_hold).min(window_ms).max(1)
    }
}

// =============================================================================
// Measurement + Percentiles
// =============================================================================

/// Minimum expected samples, derived from `window_ms` and query pacing.
///
/// Ideal samples \(\approx window_ms / pace_ms\).
/// We require **at least 25%** of that ideal count, minimum 5.
pub fn min_expected_samples(window_ms: u64, pace_ms: u64) -> u32 {
    let window_ms = window_ms.max(1);
    let pace_ms = pace_ms.max(1);
    let ideal = (window_ms / pace_ms) as u32;
    let min = (ideal / 4).max(5);
    min
}

/// Latency stats with computed percentiles (no external crates).
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyStats {
    pub samples: u32,
    pub timeouts: u32,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

impl LatencyStats {
    pub fn p50_ms(&self) -> u64 {
        self.p50_us / 1000
    }
    pub fn p95_ms(&self) -> u64 {
        self.p95_us / 1000
    }
    pub fn p99_ms(&self) -> u64 {
        self.p99_us / 1000
    }
    pub fn max_ms(&self) -> u64 {
        self.max_us / 1000
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LatencyCollector {
    timeouts: u32,
    latencies_us: Vec<u64>,
}

impl LatencyCollector {
    pub fn record_completed(&mut self, elapsed: Duration) {
        self.latencies_us.push(elapsed.as_micros() as u64);
    }

    pub fn record_timeout(&mut self) {
        self.timeouts += 1;
    }

    #[allow(dead_code)]
    pub fn merge_from(&mut self, other: LatencyCollector) {
        self.timeouts += other.timeouts;
        self.latencies_us.extend(other.latencies_us);
    }

    pub fn finish(&self) -> LatencyStats {
        let mut v = self.latencies_us.clone();
        if v.is_empty() {
            return LatencyStats {
                samples: 0,
                timeouts: self.timeouts,
                ..LatencyStats::default()
            };
        }
        v.sort_unstable();
        let max = *v.last().unwrap_or(&0);
        LatencyStats {
            samples: v.len() as u32,
            timeouts: self.timeouts,
            p50_us: percentile_sorted_us(&v, 50),
            p95_us: percentile_sorted_us(&v, 95),
            p99_us: percentile_sorted_us(&v, 99),
            max_us: max,
        }
    }
}

fn percentile_sorted_us(sorted_us: &[u64], p: u64) -> u64 {
    if sorted_us.is_empty() {
        return 0;
    }
    if sorted_us.len() == 1 {
        return sorted_us[0];
    }
    let n_minus_1 = (sorted_us.len() - 1) as u64;
    // percentile index = ceil(p * (n-1))
    // p is in [0,100].
    let idx = ((p * n_minus_1) + 99) / 100;
    sorted_us[idx as usize]
}

// =============================================================================
// Helpers
// =============================================================================

/// Returns the name of the current procman implementation
#[allow(dead_code)]
pub fn procman_impl_name() -> &'static str {
    if cfg!(feature = "procman-v1") {
        "procman-v1"
    } else if cfg!(feature = "procman-v2") {
        "procman-v2"
    } else {
        "unknown"
    }
}

// =============================================================================
// Port selection (no collisions)
// =============================================================================

/// Deterministically derive a "likely free" high port for this test process.
///
/// This is a fallback for environments where binding to port 0 is not supported by procman.
#[allow(dead_code)]
pub fn derive_unique_port(base: u16, per_test_offset: u16) -> u16 {
    // Keep in a safe high range.
    const MIN_PORT: u16 = 40000;
    const MAX_PORT: u16 = 65000;

    let base = base.clamp(MIN_PORT, MAX_PORT);
    let pid = std::process::id();
    // Spread per process in steps to reduce collisions between parallel test processes.
    let pid_bucket = (pid % 900) as u16; // 0..899
    let mut port = base.saturating_add(pid_bucket.saturating_mul(10)).saturating_add(per_test_offset);

    if port < MIN_PORT {
        port = MIN_PORT;
    }
    if port > MAX_PORT {
        // Wrap back into range deterministically.
        let span = (MAX_PORT - MIN_PORT) as u32;
        let wrapped = MIN_PORT as u32 + ((port as u32 - MIN_PORT as u32) % span);
        port = wrapped as u16;
    }
    port
}

// =============================================================================
// procman-v1 stop polling helper
// =============================================================================

#[cfg(feature = "procman-v1")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum V1StopPhase {
    Acked,
    StopCompletedEarly,
    TimedOutWaitingAck,
}

#[cfg(feature = "procman-v1")]
#[allow(dead_code)]
pub async fn v1_poll_stop_until_ack_or_done<F>(
    mut stop_fut: Pin<&mut F>,
    ack_path: &Path,
    ack_timeout: Duration,
) -> V1StopPhase
where
    F: Future,
{
    let deadline = Instant::now() + ack_timeout;
    loop {
        if tokio::fs::metadata(ack_path).await.is_ok() {
            return V1StopPhase::Acked;
        }
        if Instant::now() >= deadline {
            return V1StopPhase::TimedOutWaitingAck;
        }
        tokio::select! {
            _ = stop_fut.as_mut() => {
                return V1StopPhase::StopCompletedEarly;
            }
            _ = sleep(Duration::from_millis(25)) => {}
        }
    }
}

/// Create a standard managed process configuration
#[allow(dead_code)]
pub fn make_std_managed_process(
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
#[allow(dead_code)]
pub async fn write_text_file(path: &Path, contents: &str) {
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
#[allow(dead_code)]
pub async fn wait_for_file(path: &Path, timeout_dur: Duration) {
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

/// Wait for multiple files to exist
#[allow(dead_code)]
pub async fn wait_for_all_ready(ready_files: &[PathBuf], timeout_dur: Duration) {
    for rf in ready_files {
        wait_for_file(rf, timeout_dur).await;
    }
}

// =============================================================================
// Query loop helpers
// =============================================================================

/// Run an unmeasured warmup phase to avoid first-call overhead skewing percentiles.
#[allow(dead_code)]
pub async fn warmup_loop<F, Fut>(warmup: Duration, per_call_timeout: Duration, mut call: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future,
{
    let deadline = Instant::now() + warmup;
    while Instant::now() < deadline {
        let _ = timeout(per_call_timeout, call()).await;
        sleep(Duration::from_millis(WARMUP_PACE_MS)).await;
    }
}

/// v1-only warmup loop that keeps a pinned stop future alive while performing the same
/// query warmup attempts as `warmup_loop` (same pacing + per-call timeout).
#[cfg(feature = "procman-v1")]
#[allow(dead_code)]
pub async fn v1_warmup_loop_polling_stop<F, Fut, SFut>(
    warmup: Duration,
    per_call_timeout: Duration,
    mut stop_fut: Pin<&mut SFut>,
    stop_done: &mut bool,
    mut call: F,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future,
    SFut: Future,
{
    let deadline = Instant::now() + warmup;
    while Instant::now() < deadline {
        tokio::select! {
            _ = stop_fut.as_mut(), if !*stop_done => {
                *stop_done = true;
            }
            _ = timeout(per_call_timeout, call()) => {}
        }
        sleep(Duration::from_millis(WARMUP_PACE_MS)).await;
    }
}

/// Churn stats for measuring stop/start operations
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ChurnStats {
    pub attempted_cycles: u32,
    pub successful_cycles: u32,
    pub failed_cycles: u32,
    pub stop_errors: u32,
    pub start_errors: u32,
}

impl ChurnStats {
    // Intentionally no helpers here: keep the surface area small.
}

// =============================================================================
// Query Loop Helpers (all impls)
// =============================================================================

/// Collect query latencies for a fixed window with pacing.
///
/// - Records per-call latency (microseconds) for calls that finish before `timeout_ms`.
/// - Counts timeouts separately.
#[allow(dead_code)]
pub async fn collect_query_latencies<F, Fut>(
    window_ms: u64,
    pace_ms: u64,
    timeout_ms: u64,
    mut query_fn: F,
) -> LatencyStats
where
    F: FnMut() -> Fut,
    Fut: std::future::Future,
{
    let window = Duration::from_millis(window_ms.max(1));
    let pace = Duration::from_millis(pace_ms.max(1));
    let per_call_timeout = Duration::from_millis(timeout_ms.max(1));
    let deadline = Instant::now() + window;

    let mut c = LatencyCollector::default();
    while Instant::now() < deadline {
        let t0 = Instant::now();
        match timeout(per_call_timeout, query_fn()).await {
            Ok(_any) => c.record_completed(t0.elapsed()),
            Err(_) => c.record_timeout(),
        }
        sleep(pace).await;
    }

    c.finish()
}

/// Collect query latencies into a raw collector so callers can merge samples and compute correct
/// percentiles across multiple concurrent query loops.
#[allow(dead_code)]
pub async fn collect_query_latencies_collector<F, Fut>(
    window_ms: u64,
    pace_ms: u64,
    timeout_ms: u64,
    mut query_fn: F,
) -> LatencyCollector
where
    F: FnMut() -> Fut,
    Fut: std::future::Future,
{
    let window = Duration::from_millis(window_ms.max(1));
    let pace = Duration::from_millis(pace_ms.max(1));
    let per_call_timeout = Duration::from_millis(timeout_ms.max(1));
    let deadline = Instant::now() + window;

    let mut c = LatencyCollector::default();
    while Instant::now() < deadline {
        let t0 = Instant::now();
        match timeout(per_call_timeout, query_fn()).await {
            Ok(_any) => c.record_completed(t0.elapsed()),
            Err(_) => c.record_timeout(),
        }
        sleep(pace).await;
    }
    c
}

/// Run N strict probe queries (no pacing) and capture their latencies/timeouts.
#[allow(dead_code)]
pub async fn probe_queries<F, Fut>(count: u32, timeout_ms: u64, mut query_fn: F) -> LatencyStats
where
    F: FnMut() -> Fut,
    Fut: std::future::Future,
{
    let per_call_timeout = Duration::from_millis(timeout_ms.max(1));
    let mut c = LatencyCollector::default();
    for _ in 0..count.max(1) {
        let t0 = Instant::now();
        match timeout(per_call_timeout, query_fn()).await {
            Ok(_any) => c.record_completed(t0.elapsed()),
            Err(_) => c.record_timeout(),
        }
    }
    c.finish()
}

// =============================================================================
// Process Manager Setup Helpers
// =============================================================================

/// Create a ProcessManager with the given configuration
#[allow(dead_code)]
pub async fn create_manager(
    test_dir: &Path,
    port: u16,
    managed_processes: Vec<ProcessConfig>,
) -> ProcessManager {
    let config = ProcessManagerConfig {
        process_manager: ProcessManagerOptions {
            port,
            log_level: "warn".to_string(),
            force_shutdown_timeout: Duration::from_secs(60),
            base_directory: Some(test_dir.to_string_lossy().to_string()),
        },
        managed_processes,
        log_collection: None,
    };

    ProcessManager::new(config)
        .await
        .expect("Failed to create ProcessManager")
}

pub struct StartedManager {
    pub manager: ProcessManager,
    pub port: u16,
    pub port_mode: &'static str,
    pub port0_error: Option<String>,
}

/// Create + start a ProcessManager with a test-unique port.
///
/// Prefers `port=0` (ephemeral) if procman supports it; otherwise falls back to a
/// deterministic derived high port that is unique per test process.
#[allow(dead_code)]
pub async fn create_started_manager_unique_port(
    test_dir: &Path,
    per_test_offset: u16,
    managed_processes: Vec<ProcessConfig>,
) -> StartedManager {
    // Attempt port 0 first (best-effort).
    {
        let mut mgr = create_manager(test_dir, 0, managed_processes.clone()).await;
        match mgr.start().await {
            Ok(()) => {
                return StartedManager {
                    manager: mgr,
                    port: 0,
                    port_mode: "ephemeral",
                    port0_error: None,
                };
            }
            Err(e) => {
                // Fall through to deterministic derived port.
                let port = derive_unique_port(54000, per_test_offset);
                let mut mgr2 = create_manager(test_dir, port, managed_processes).await;
                mgr2.start().await.unwrap_or_else(|e2| {
                    panic!("Failed to start ProcessManager (port0 failed: {e:?}) and derived port {port} failed: {e2:?}")
                });
                return StartedManager {
                    manager: mgr2,
                    port,
                    port_mode: "derived",
                    port0_error: Some(format!("{e:?}")),
                };
            }
        }
    }
}

#[allow(dead_code)]
pub async fn shutdown_manager_best_effort(mut manager: ProcessManager, timeout_dur: Duration) {
    let _ = timeout(timeout_dur, manager.shutdown()).await;
}

// =============================================================================
// Assertion helpers (fair + robust)
// =============================================================================

#[allow(dead_code)]
pub fn v2_passes_responsiveness(q: &LatencyStats, p99_threshold_ms: u128) -> Result<(), String> {
    if q.timeouts != 0 {
        return Err(format!("query timeouts: {}", q.timeouts));
    }
    if (q.p99_ms() as u128) > p99_threshold_ms {
        return Err(format!(
            "p99 latency too high: {}ms (threshold {}ms)",
            q.p99_ms(),
            p99_threshold_ms
        ));
    }
    Ok(())
}

/// For v1 demos we want a deterministic "strong signal" of contention/blocking that doesn't
/// depend on absolute machine speed.
#[allow(dead_code)]
pub fn v1_exhibits_contention(
    q: &LatencyStats,
    gate_hold_ms: u64,
    min_expected_samples: u32,
) -> Result<(), String> {
    if q.timeouts > 0 {
        return Ok(());
    }

    // If we have a gate window, we can use it as a deterministic time-scale signal.
    if gate_hold_ms > 0 {
        let gate_ms = gate_hold_ms as u64;
        // "Near gate" means in the same order of magnitude; be generous to avoid flake.
        let near_gate_ms = gate_ms.saturating_sub((gate_ms / 5).max(50)); // ~80% of gate, at least -50ms
        if q.p95_ms() >= near_gate_ms || q.p99_ms() >= near_gate_ms {
            return Ok(());
        }
    }

    // Extremely low progress compared to expectation is also a strong signal.
    let expected = min_expected_samples.max(1);
    if q.samples < (expected / 4).max(1) {
        return Ok(());
    }
    Err("no strong contention signal observed".to_string())
}

/// Create arguments for a gated process (with shutdown gate)
#[allow(dead_code)]
pub fn make_gated_args(
    ready_file: &Path,
    ack_file: &Path,
    gate_file: &Path,
) -> Vec<String> {
    vec![
        "--ready-file".to_string(),
        ready_file.to_string_lossy().to_string(),
        "--signal-ack-file".to_string(),
        ack_file.to_string_lossy().to_string(),
        "--shutdown-gate-file".to_string(),
        gate_file.to_string_lossy().to_string(),
        "--shutdown-delay".to_string(),
        "0".to_string(),
    ]
}

/// Create arguments for a simple process (no gating)
#[allow(dead_code)]
pub fn make_simple_args(ready_file: &Path) -> Vec<String> {
    vec![
        "--ready-file".to_string(),
        ready_file.to_string_lossy().to_string(),
        "--shutdown-delay".to_string(),
        "0".to_string(),
    ]
}

// =============================================================================
// Reporting (single-line human + optional JSON)
// =============================================================================

#[derive(Debug, Clone, Copy)]
pub enum Verdict {
    Pass,
    Fail,
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict::Fail
    }
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScenarioReport {
    pub impl_name: &'static str,
    pub scenario: &'static str,

    pub n: usize,
    pub window_ms: u64,
    pub warmup_ms: u64,
    pub gate_hold_ms: u64,

    pub query_pace_ms: u64,
    pub churn_pace_ms: u64,
    pub query_timeout_ms: u64,
    pub stop_timeout_ms: u64,
    pub concurrency: usize,
    pub readers: usize,
    pub writers: usize,
    pub cancel_probe_timeout_ms: u64,
    pub cancel_probe_count: u32,

    pub port: u16,
    pub port_mode: &'static str,
    pub port0_error: Option<String>,

    pub min_expected_query_samples: u32,

    pub query: LatencyStats,
    pub cancel_probe: Option<LatencyStats>,

    pub churn: Option<ChurnStats>,

    pub total_elapsed_ms: u128,

    pub notes: Vec<String>,
    pub verdict: Verdict,
    pub reason: String,
}

impl ScenarioReport {
    #[allow(dead_code)]
    pub fn new(impl_name: &'static str, scenario: &'static str) -> Self {
        Self {
            impl_name,
            scenario,
            verdict: Verdict::Fail,
            reason: "uninitialized".to_string(),
            ..Default::default()
        }
    }

    #[allow(dead_code)]
    pub fn with_config(mut self, cfg: &PerfConfig) -> Self {
        self.n = cfg.n;
        self.window_ms = cfg.window_ms;
        self.warmup_ms = cfg.warmup_ms;
        self.query_pace_ms = cfg.query_pace_ms;
        self.churn_pace_ms = cfg.churn_pace_ms;
        self.query_timeout_ms = cfg.timeout_ms;
        self.stop_timeout_ms = cfg.stop_timeout_ms;
        self.concurrency = cfg.concurrency;
        self.readers = cfg.readers;
        self.writers = cfg.writers;
        self.cancel_probe_timeout_ms = cfg.cancel_probe_timeout_ms;
        self.cancel_probe_count = cfg.cancel_probe_count;
        self
    }

    #[allow(dead_code)]
    pub fn with_started_manager(mut self, started: &StartedManager) -> Self {
        self.port = started.port;
        self.port_mode = started.port_mode;
        self.port0_error = started.port0_error.clone();
        if let Some(ref e) = started.port0_error {
            self.notes.push(format!("port0_failed={}", e.replace('"', "'")));
        }
        self
    }

    #[allow(dead_code)]
    pub fn with_gate_hold_ms(mut self, gate_hold_ms: u64) -> Self {
        self.gate_hold_ms = gate_hold_ms;
        self
    }

    #[allow(dead_code)]
    pub fn with_min_expected_query_samples(mut self, min_expected_samples: u32) -> Self {
        self.min_expected_query_samples = min_expected_samples;
        self
    }

    #[allow(dead_code)]
    pub fn with_query_stats(mut self, q: LatencyStats) -> Self {
        self.query = q;
        self
    }

    #[allow(dead_code)]
    pub fn with_churn_stats(mut self, cs: ChurnStats) -> Self {
        self.churn = Some(cs);
        self
    }

    #[allow(dead_code)]
    pub fn with_cancel_probe(mut self, probe: LatencyStats) -> Self {
        self.cancel_probe = Some(probe);
        self
    }

    #[allow(dead_code)]
    pub fn with_total_elapsed_ms(mut self, total_elapsed_ms: u128) -> Self {
        self.total_elapsed_ms = total_elapsed_ms;
        self
    }

    #[allow(dead_code)]
    pub fn push_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    #[allow(dead_code)]
    pub fn with_verdict(mut self, verdict: Verdict, reason: impl Into<String>) -> Self {
        self.verdict = verdict;
        self.reason = reason.into();
        self
    }

    fn flags(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.query.samples == 0 {
            flags.push("NO_PROGRESS");
        }
        if self.query.timeouts > 0 {
            flags.push("TIMEOUTS");
        }
        flags
    }

    #[allow(dead_code)]
    pub fn print_final(&self) {
        // Human readable (single line).
        let flags = self.flags();
        let flags_joined = if flags.is_empty() {
            "none".to_string()
        } else {
            flags.join(",")
        };
        let notes_joined = if self.notes.is_empty() {
            "".to_string()
        } else {
            format!(" notes=\"{}\"", self.notes.join(";"))
        };

        let probe = self.cancel_probe.unwrap_or_default();

        println!(
            "[{impl_name}] scenario={scenario} n={n} window_ms={window_ms} warmup_ms={warmup_ms} gate_hold_ms={gate_hold_ms} \
query_pace_ms={query_pace_ms} churn_pace_ms={churn_pace_ms} query_timeout_ms={query_timeout_ms} stop_timeout_ms={stop_timeout_ms} \
concurrency={concurrency} readers={readers} writers={writers} cancel_probe_timeout_ms={cancel_probe_timeout_ms} cancel_probe_count={cancel_probe_count} \
port={port} port_mode={port_mode} \
q_samples={q_samples} q_timeouts={q_timeouts} q_p50_ms={q_p50_ms} q_p95_ms={q_p95_ms} q_p99_ms={q_p99_ms} q_max_ms={q_max_ms} \
probe_samples={probe_samples} probe_timeouts={probe_timeouts} probe_p50_ms={probe_p50_ms} probe_p95_ms={probe_p95_ms} probe_p99_ms={probe_p99_ms} probe_max_ms={probe_max_ms} \
churn_attempted_cycles={churn_attempted} churn_successful_cycles={churn_successful} churn_failed_cycles={churn_failed} \
total_ms={total_ms} verdict={verdict} flags={flags}{notes} reason=\"{reason}\"",
            impl_name = self.impl_name,
            scenario = self.scenario,
            n = self.n,
            window_ms = self.window_ms,
            warmup_ms = self.warmup_ms,
            gate_hold_ms = self.gate_hold_ms,
            query_pace_ms = self.query_pace_ms,
            churn_pace_ms = self.churn_pace_ms,
            query_timeout_ms = self.query_timeout_ms,
            stop_timeout_ms = self.stop_timeout_ms,
            concurrency = self.concurrency,
            readers = self.readers,
            writers = self.writers,
            cancel_probe_timeout_ms = self.cancel_probe_timeout_ms,
            cancel_probe_count = self.cancel_probe_count,
            port = self.port,
            port_mode = self.port_mode,
            q_samples = self.query.samples,
            q_timeouts = self.query.timeouts,
            q_p50_ms = self.query.p50_ms(),
            q_p95_ms = self.query.p95_ms(),
            q_p99_ms = self.query.p99_ms(),
            q_max_ms = self.query.max_ms(),
            probe_samples = probe.samples,
            probe_timeouts = probe.timeouts,
            probe_p50_ms = probe.p50_ms(),
            probe_p95_ms = probe.p95_ms(),
            probe_p99_ms = probe.p99_ms(),
            probe_max_ms = probe.max_ms(),
            churn_attempted = self.churn.as_ref().map(|c| c.attempted_cycles).unwrap_or(0),
            churn_successful = self.churn.as_ref().map(|c| c.successful_cycles).unwrap_or(0),
            churn_failed = self.churn.as_ref().map(|c| c.failed_cycles).unwrap_or(0),
            total_ms = self.total_elapsed_ms,
            verdict = self.verdict.as_str(),
            flags = flags_joined.as_str(),
            notes = notes_joined,
            reason = self.reason.replace('"', "'"),
        );

        // JSON line (single line, flat keys).
        let flags_json = self.flags().join(",");
        let notes_json = self.notes.join(";");
        println!(
            r#"{{"impl":"{impl_name}","scenario":"{scenario}","n":{n},"window_ms":{window_ms},"warmup_ms":{warmup_ms},"gate_hold_ms":{gate_hold_ms},"query_pace_ms":{query_pace_ms},"churn_pace_ms":{churn_pace_ms},"query_timeout_ms":{query_timeout_ms},"stop_timeout_ms":{stop_timeout_ms},"concurrency":{concurrency},"readers":{readers},"writers":{writers},"cancel_probe_timeout_ms":{cancel_probe_timeout_ms},"cancel_probe_count":{cancel_probe_count},"port":{port},"port_mode":"{port_mode}","port0_error":"{port0_error}","min_expected_query_samples":{min_expected_query_samples},"query_samples":{q_samples},"query_timeouts":{q_timeouts},"query_p50_us":{q_p50_us},"query_p95_us":{q_p95_us},"query_p99_us":{q_p99_us},"query_max_us":{q_max_us},"query_p50_ms":{q_p50_ms},"query_p95_ms":{q_p95_ms},"query_p99_ms":{q_p99_ms},"query_max_ms":{q_max_ms},"cancel_probe_samples":{probe_samples},"cancel_probe_timeouts":{probe_timeouts},"cancel_probe_p50_us":{probe_p50_us},"cancel_probe_p95_us":{probe_p95_us},"cancel_probe_p99_us":{probe_p99_us},"cancel_probe_max_us":{probe_max_us},"churn_attempted_cycles":{churn_attempted_cycles},"churn_successful_cycles":{churn_successful_cycles},"churn_failed_cycles":{churn_failed_cycles},"churn_stop_errors":{churn_stop_errors},"churn_start_errors":{churn_start_errors},"total_ms":{total_ms},"flags":"{flags}","verdict":"{verdict}","reason":"{reason}","notes":"{notes}"}}"#,
            impl_name = self.impl_name,
            scenario = self.scenario,
            n = self.n,
            window_ms = self.window_ms,
            warmup_ms = self.warmup_ms,
            gate_hold_ms = self.gate_hold_ms,
            query_pace_ms = self.query_pace_ms,
            churn_pace_ms = self.churn_pace_ms,
            query_timeout_ms = self.query_timeout_ms,
            stop_timeout_ms = self.stop_timeout_ms,
            concurrency = self.concurrency,
            readers = self.readers,
            writers = self.writers,
            cancel_probe_timeout_ms = self.cancel_probe_timeout_ms,
            cancel_probe_count = self.cancel_probe_count,
            port = self.port,
            port_mode = self.port_mode,
            port0_error = self.port0_error.clone().unwrap_or_default().replace('"', "'"),
            min_expected_query_samples = self.min_expected_query_samples,
            q_samples = self.query.samples,
            q_timeouts = self.query.timeouts,
            q_p50_us = self.query.p50_us,
            q_p95_us = self.query.p95_us,
            q_p99_us = self.query.p99_us,
            q_max_us = self.query.max_us,
            q_p50_ms = self.query.p50_ms(),
            q_p95_ms = self.query.p95_ms(),
            q_p99_ms = self.query.p99_ms(),
            q_max_ms = self.query.max_ms(),
            probe_samples = self.cancel_probe.map(|p| p.samples).unwrap_or(0),
            probe_timeouts = self.cancel_probe.map(|p| p.timeouts).unwrap_or(0),
            probe_p50_us = self.cancel_probe.map(|p| p.p50_us).unwrap_or(0),
            probe_p95_us = self.cancel_probe.map(|p| p.p95_us).unwrap_or(0),
            probe_p99_us = self.cancel_probe.map(|p| p.p99_us).unwrap_or(0),
            probe_max_us = self.cancel_probe.map(|p| p.max_us).unwrap_or(0),
            churn_attempted_cycles = self.churn.as_ref().map(|c| c.attempted_cycles).unwrap_or(0),
            churn_successful_cycles = self.churn.as_ref().map(|c| c.successful_cycles).unwrap_or(0),
            churn_failed_cycles = self.churn.as_ref().map(|c| c.failed_cycles).unwrap_or(0),
            churn_stop_errors = self.churn.as_ref().map(|c| c.stop_errors).unwrap_or(0),
            churn_start_errors = self.churn.as_ref().map(|c| c.start_errors).unwrap_or(0),
            total_ms = self.total_elapsed_ms,
            flags = flags_json,
            verdict = self.verdict.as_str(),
            reason = self.reason.replace('"', "'"),
            notes = notes_json.replace('"', "'"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{min_expected_samples, percentile_sorted_us, PerfConfig};

    #[test]
    fn gate_hold_ms_never_panics_and_is_in_range() {
        for window_ms in 0u64..=1000u64 {
            let window_ms = window_ms.max(1);
            let cfg = PerfConfig {
                n: 1,
                window_ms,
                warmup_ms: 0,
                query_pace_ms: 10,
                churn_pace_ms: 10,
                timeout_ms: 1,
                stop_timeout_ms: 1,
                concurrency: 1,
                readers: 1,
                writers: 1,
                cancel_probe_timeout_ms: 1,
                cancel_probe_count: 1,
            };
            let hold = cfg.gate_hold_ms();
            assert!(hold > 0);
            assert!(hold <= window_ms);
        }
    }

    #[test]
    fn percentile_index_matches_ceil_definition() {
        let v = vec![10u64, 20, 30, 40, 50];
        // n=5, n-1=4.
        // p50: ceil(0.5*4)=2 -> 30
        assert_eq!(percentile_sorted_us(&v, 50), 30);
        // p90: ceil(0.9*4)=4 -> 50
        assert_eq!(percentile_sorted_us(&v, 90), 50);
        // p99: ceil(0.99*4)=4 -> 50
        assert_eq!(percentile_sorted_us(&v, 99), 50);
    }

    #[test]
    fn min_expected_samples_derived_from_pace() {
        // 1000ms window, 10ms pace => ideal 100 => min 25
        assert_eq!(min_expected_samples(1000, 10), 25);
        // tiny window still min 5
        assert_eq!(min_expected_samples(10, 10), 5);
    }
}

