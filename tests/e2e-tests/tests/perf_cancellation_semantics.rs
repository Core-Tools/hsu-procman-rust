//! Performance E2E: caller cancellation of an in-flight stop does not wedge responsiveness.
//!
//! ### Architecture signal
//! - **Property**: aborting a *caller task* that is awaiting `stop_process()` must not wedge the manager.
//! - **Why v1 can be risky**: if a global lock is held across `await`, cancellation can strand work and block reads.
//! - **Why v2 should be safe**: actor messaging boundaries + out-of-band ops reduce lock coupling; cancellation
//!   should not poison global state.
//!
//! ## Run
//! - v2: `cargo test -p e2e-tests --test perf_cancellation_semantics -- --ignored --nocapture`
//! - v1: `cargo test -p e2e-tests --no-default-features --features procman-v1 --test perf_cancellation_semantics -- --ignored --nocapture`

#[path = "perf_common/mod.rs"]
mod perf_common;

use std::time::Duration;

use e2e_tests::{create_test_dir_guard, get_testexe_path};
#[cfg(feature = "procman-v2")]
use hsu_common::ProcessError;
use tokio::time::{timeout, Instant};

use perf_common::{
    collect_query_latencies, create_started_manager_unique_port, make_gated_args, make_std_managed_process,
    min_expected_samples, probe_queries, procman_impl_name, v2_passes_responsiveness, wait_for_file,
    write_text_file, PerfConfig, ScenarioReport, Verdict, V2_MAX_LATENCY_MS,
};

#[cfg(feature = "procman-v1")]
use std::sync::Arc;

#[cfg(feature = "procman-v2")]
fn is_expected_stop_state_error(err: &ProcessError) -> bool {
    matches!(
        err,
        ProcessError::InvalidState { .. } | ProcessError::OperationNotAllowed { .. }
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn perf_cancellation_semantics() {
    let cfg = PerfConfig::from_env_with_defaults(1, 1000);
    let (test_dir, _test_dir_guard) = create_test_dir_guard("perf-cancellation-semantics");
    let testexe = get_testexe_path();

    let scenario = "caller_cancellation_semantics";
    let min_samples = min_expected_samples(cfg.window_ms, cfg.query_pace_ms);

    // Single gated process.
    let gated_id = "proc-gated".to_string();
    let gated_ready = test_dir.join("gated.ready");
    let gated_ack = test_dir.join("gated.ack");
    let gated_gate = test_dir.join("gated.gate");

    let managed_processes = vec![make_std_managed_process(
        &gated_id,
        &testexe,
        make_gated_args(&gated_ready, &gated_ack, &gated_gate),
        Duration::from_secs(60),
    )];

    let started = create_started_manager_unique_port(&test_dir, 3, managed_processes).await;
    let port = started.port;
    let port_mode = started.port_mode;
    let port0_error = started.port0_error.clone();

    // Start manager already done; wait ready.
    wait_for_file(&gated_ready, Duration::from_secs(10)).await;

    let per_call_timeout = Duration::from_millis(cfg.timeout_ms);
    let stop_timeout = Duration::from_millis(cfg.stop_timeout_ms);

    let (probe, query, second_stop_accepted, total_ms, manager_for_shutdown) = {
        #[cfg(feature = "procman-v2")]
        {
            let manager = started.manager;

            // Start a stop that will block on the gate.
            let mgr_for_stop = manager.clone();
            let gated_id_clone = gated_id.clone();
            let stop_handle = tokio::spawn(async move { mgr_for_stop.stop_process(&gated_id_clone).await });

            // Wait for ack, then abort (caller cancellation).
            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            stop_handle.abort();

            // Immediate probes with strict timeout (pre-warmup) to avoid hiding short wedges.
            let probe = probe_queries(
                cfg.cancel_probe_count,
                cfg.cancel_probe_timeout_ms,
                || manager.get_all_process_info(),
            )
            .await;

            // Warmup (not measured) and then main window measurement.
            perf_common::warmup_loop(
                Duration::from_millis(cfg.warmup_ms),
                per_call_timeout,
                || manager.get_all_process_info(),
            )
            .await;

            let start = Instant::now();
            let query = collect_query_latencies(
                cfg.window_ms,
                cfg.query_pace_ms,
                cfg.timeout_ms,
                || manager.get_all_process_info(),
            )
            .await;

            // Open gate and ensure process can be stopped (or is already stopping/stopped).
            write_text_file(&gated_gate, "go\n").await;
            let second = timeout(stop_timeout, manager.stop_process(&gated_id)).await;
            let accepted = match second {
                Ok(Ok(())) => true,
                Ok(Err(ref e)) => is_expected_stop_state_error(e),
                Err(_) => false,
            };

            // Ensure the aborted caller task is fully torn down (avoid leaks impacting shutdown).
            let _ = timeout(Duration::from_secs(5), stop_handle).await;

            (probe, query, accepted, start.elapsed().as_millis(), manager)
        }

        #[cfg(feature = "procman-v1")]
        {
            // v1: run the same caller-cancellation experiment. We do NOT require "good" latencies; we only
            // require the manager does not completely wedge and that the process can be stopped after opening
            // the gate.
            let manager = Arc::new(started.manager);

            // Start a stop that will block on the gate.
            let mgr_for_stop = manager.clone();
            let gated_id_clone = gated_id.clone();
            let stop_handle = tokio::spawn(async move { mgr_for_stop.stop_process(&gated_id_clone).await });

            // Wait for ack, then abort (caller cancellation).
            wait_for_file(&gated_ack, Duration::from_secs(5)).await;
            stop_handle.abort();

            let probe = probe_queries(
                cfg.cancel_probe_count,
                cfg.cancel_probe_timeout_ms,
                || manager.get_all_process_info(),
            )
            .await;

            perf_common::warmup_loop(
                Duration::from_millis(cfg.warmup_ms),
                per_call_timeout,
                || manager.get_all_process_info(),
            )
            .await;

            let start = Instant::now();
            let query = collect_query_latencies(
                cfg.window_ms,
                cfg.query_pace_ms,
                cfg.timeout_ms,
                || manager.get_all_process_info(),
            )
            .await;

            write_text_file(&gated_gate, "go\n").await;
            let accepted = timeout(stop_timeout, manager.stop_process(&gated_id)).await.is_ok();

            // Ensure the aborted caller task is fully torn down (avoid leaks impacting shutdown).
            let _ = timeout(Duration::from_secs(5), stop_handle).await;

            // Unwrap Arc for shutdown.
            let manager = match Arc::try_unwrap(manager) {
                Ok(m) => m,
                Err(_) => panic!("manager Arc still has outstanding refs (task leak?)"),
            };

            (probe, query, accepted, start.elapsed().as_millis(), manager)
        }
    };

    // Cleanup.
    if tokio::fs::metadata(&gated_gate).await.is_err() {
        write_text_file(&gated_gate, "go\n").await;
    }
    let _ = timeout(Duration::from_secs(120), async {
        let mut m = manager_for_shutdown;
        m.shutdown().await
    })
    .await;

    // Verdict.
    let mut verdict = Verdict::Pass;
    let mut reason = "ok".to_string();

    if cfg!(feature = "procman-v2") {
        if probe.timeouts > 0 {
            verdict = Verdict::Fail;
            reason = format!("probe timeouts immediately after abort: {}", probe.timeouts);
        } else if let Err(e) = v2_passes_responsiveness(&query, V2_MAX_LATENCY_MS) {
            verdict = Verdict::Fail;
            reason = e;
        } else if query.samples < min_samples {
            verdict = Verdict::Fail;
            reason = format!("insufficient query samples: got {} expected {}", query.samples, min_samples);
        } else if !second_stop_accepted {
            verdict = Verdict::Fail;
            reason = "second stop not accepted after opening gate".to_string();
        }
    } else {
        // For v1 we just require "no complete wedge" + successful cleanup.
        if probe.timeouts >= cfg.cancel_probe_count {
            verdict = Verdict::Fail;
            reason = format!(
                "v1 appears wedged immediately after abort: probe_timeouts={} probe_count={}",
                probe.timeouts, cfg.cancel_probe_count
            );
        } else if query.samples == 0 {
            verdict = Verdict::Fail;
            reason = "v1 made no query progress during measurement window".to_string();
        } else if !second_stop_accepted {
            verdict = Verdict::Fail;
            reason = "v1 second stop not accepted after opening gate".to_string();
        }
    }

    let mut report = ScenarioReport::new(procman_impl_name(), scenario)
        .with_config(&cfg)
        .with_gate_hold_ms(0)
        .with_min_expected_query_samples(min_samples)
        .with_query_stats(query)
        .with_cancel_probe(probe)
        .with_total_elapsed_ms(total_ms)
        .push_note(format!("second_stop_accepted={}", second_stop_accepted))
        .with_verdict(verdict, reason.clone());

    report.port = port;
    report.port_mode = port_mode;
    report.port0_error = port0_error;
    if let Some(ref e) = report.port0_error {
        report.notes.push(format!("port0_failed={}", e.replace('"', "'")));
    }

    report.print_final();

    if cfg!(feature = "procman-v2") {
        assert_eq!(probe.timeouts, 0);
        assert_eq!(query.timeouts, 0);
        assert!((query.p99_ms() as u128) <= V2_MAX_LATENCY_MS);
        assert!(query.samples >= min_samples);
        assert!(second_stop_accepted);
    }
}

 