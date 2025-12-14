//! Performance E2E: long stop on a single “hot” process while other processes churn.
//!
//! ### Architecture signal
//! - **Property**: a long stop on one process should not wedge the manager’s *query* APIs.
//! - **Why v1 degrades**: global write-lock across `await` in stop/start blocks reads -> latency spikes/timeouts.
//! - **Why v2 stays responsive**: actor + per-process queuing + cached query snapshots.
//!
//! ## Run
//! - v2: `cargo test -p e2e-tests --test perf_hot_process_isolation -- --ignored --nocapture`
//! - v1: `cargo test -p e2e-tests --no-default-features --features procman-v1 --test perf_hot_process_isolation -- --ignored --nocapture`

#[path = "perf_common/mod.rs"]
mod perf_common;

#[cfg(feature = "procman-v1")]
use std::sync::Arc;
use std::time::Duration;

use e2e_tests::{create_test_dir_guard, get_testexe_path};
use tokio::time::{sleep, timeout, Instant};

use perf_common::{
    collect_query_latencies, create_started_manager_unique_port, make_gated_args, make_simple_args,
    make_std_managed_process, min_expected_samples, procman_impl_name, v1_exhibits_contention,
    v2_passes_responsiveness, wait_for_all_ready, write_text_file, ChurnStats, PerfConfig,
    ScenarioReport, Verdict, V2_MAX_LATENCY_MS,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn perf_hot_process_isolation() {
    let cfg = PerfConfig::from_env_with_defaults(16, 3000);
    let (test_dir, _test_dir_guard) = create_test_dir_guard("perf-hot-process-isolation");
    let testexe = get_testexe_path();

    let gate_hold_ms = cfg.gate_hold_ms();
    let scenario = "hot_process_isolation";
    let min_samples = min_expected_samples(cfg.window_ms, cfg.query_pace_ms);

    // Hot gated process.
    let hot_id = "proc-hot".to_string();
    let hot_ready = test_dir.join("hot.ready");
    let hot_ack = test_dir.join("hot.ack");
    let hot_gate = test_dir.join("hot.gate");

    // Cold processes (targets for churn cycles).
    let n_cold = cfg.n.min(32).saturating_sub(1);
    let mut cold_ids = Vec::new();

    let mut managed_processes = Vec::new();
    let mut ready_files = Vec::new();

    managed_processes.push(make_std_managed_process(
        &hot_id,
        &testexe,
        make_gated_args(&hot_ready, &hot_ack, &hot_gate),
        Duration::from_secs(60),
    ));
    ready_files.push(hot_ready.clone());

    for i in 0..n_cold {
        let id = format!("proc-cold-{i}");
        let ready = test_dir.join(format!("{id}.ready"));
        ready_files.push(ready.clone());
        cold_ids.push(id.clone());
        managed_processes.push(make_std_managed_process(
            &id,
            &testexe,
            make_simple_args(&ready),
            Duration::from_secs(5),
        ));
    }

    let started = create_started_manager_unique_port(&test_dir, 2, managed_processes).await;
    let port = started.port;
    let port_mode = started.port_mode;
    let port0_error = started.port0_error.clone();

    #[cfg(feature = "procman-v2")]
    let (query, churn, total_ms, manager_for_shutdown) = {
        let manager = started.manager;
        wait_for_all_ready(&ready_files, Duration::from_secs(30)).await;

        // Start hot stop and wait for ack.
        let mgr_for_hot = manager.clone();
        let hot_id_clone = hot_id.clone();
        let hot_stop_task = tokio::spawn(async move { mgr_for_hot.stop_process(&hot_id_clone).await });
        perf_common::wait_for_file(&hot_ack, Duration::from_secs(5)).await;

        // Warmup (not measured).
        perf_common::warmup_loop(
            Duration::from_millis(cfg.warmup_ms),
            Duration::from_millis(cfg.timeout_ms),
            || manager.get_all_process_info(),
        )
        .await;

        let start = Instant::now();
        let deadline = start + Duration::from_millis(cfg.window_ms);

        // Open hot gate after hold.
        let gate_path = hot_gate.clone();
        let gate_hold = Duration::from_millis(gate_hold_ms);
        let gate_opener = tokio::spawn(async move {
            sleep(gate_hold).await;
            write_text_file(&gate_path, "go\n").await;
        });

        // Query task.
        let mgr_for_query = manager.clone();
        let query_task = tokio::spawn(async move {
            collect_query_latencies(
                cfg.window_ms,
                cfg.query_pace_ms,
                cfg.timeout_ms,
                || mgr_for_query.get_all_process_info(),
            )
            .await
        });

        // Cold churn cycles.
        let churn_task_count = cfg.concurrency.min(4).max(1);
        let mut churn_handles = Vec::new();
        for task_idx in 0..churn_task_count {
            let mgr = manager.clone();
            let ids = cold_ids.clone();
            let stop_timeout_ms = cfg.stop_timeout_ms;
            let churn_pace_ms = cfg.churn_pace_ms;
            churn_handles.push(tokio::spawn(async move {
                let mut stats = ChurnStats::default();
                let mut idx = task_idx;
                while Instant::now() < deadline {
                    if ids.is_empty() {
                        sleep(Duration::from_millis(churn_pace_ms)).await;
                        continue;
                    }
                    let id = &ids[idx % ids.len()];
                    idx += churn_task_count;

                    stats.attempted_cycles += 1;
                    let stop_ok = timeout(Duration::from_millis(stop_timeout_ms), mgr.stop_process(id))
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);
                    if !stop_ok {
                        stats.stop_errors += 1;
                        stats.failed_cycles += 1;
                        sleep(Duration::from_millis(churn_pace_ms)).await;
                        continue;
                    }
                    let start_ok = timeout(Duration::from_millis(stop_timeout_ms), mgr.start_process(id))
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);
                    if !start_ok {
                        stats.start_errors += 1;
                        stats.failed_cycles += 1;
                    } else {
                        stats.successful_cycles += 1;
                    }
                    sleep(Duration::from_millis(churn_pace_ms)).await;
                }
                stats
            }));
        }

        let query = query_task.await.expect("query join failed");
        let mut churn = ChurnStats::default();
        for h in churn_handles {
            let s = h.await.expect("churn join failed");
            churn.attempted_cycles += s.attempted_cycles;
            churn.successful_cycles += s.successful_cycles;
            churn.failed_cycles += s.failed_cycles;
            churn.stop_errors += s.stop_errors;
            churn.start_errors += s.start_errors;
        }

        let _ = gate_opener.await;
        if tokio::fs::metadata(&hot_gate).await.is_err() {
            write_text_file(&hot_gate, "go\n").await;
        }
        let _ = timeout(Duration::from_secs(30), hot_stop_task).await;

        (query, churn, start.elapsed().as_millis(), manager)
    };

    #[cfg(feature = "procman-v1")]
    let (query, churn, total_ms, manager_for_shutdown) = {
        let manager = Arc::new(started.manager);
        wait_for_all_ready(&ready_files, Duration::from_secs(30)).await;

        // Start hot stop (keep future alive). Use a cloned Arc so we can later unwrap.
        let mgr_for_stop = manager.clone();
        let mut stop_fut = Box::pin(async move { mgr_for_stop.stop_process(&hot_id).await });
        let phase = perf_common::v1_poll_stop_until_ack_or_done(
            stop_fut.as_mut(),
            &hot_ack,
            Duration::from_secs(5),
        )
        .await;
        let mut stop_done = phase == perf_common::V1StopPhase::StopCompletedEarly;
        match phase {
            perf_common::V1StopPhase::Acked => {}
            perf_common::V1StopPhase::StopCompletedEarly => {
                write_text_file(&hot_gate, "go\n").await;
                panic!("v1 stop completed before ack; cannot measure contention");
            }
            perf_common::V1StopPhase::TimedOutWaitingAck => {
                write_text_file(&hot_gate, "go\n").await;
                let _ = timeout(Duration::from_secs(30), stop_fut.as_mut()).await;
                panic!("v1 stop did not ack within timeout");
            }
        }

        // Warmup (not measured).
        perf_common::v1_warmup_loop_polling_stop(
            Duration::from_millis(cfg.warmup_ms),
            Duration::from_millis(cfg.timeout_ms),
            stop_fut.as_mut(),
            &mut stop_done,
            || manager.get_all_process_info(),
        )
        .await;

        let start = Instant::now();
        let deadline = start + Duration::from_millis(cfg.window_ms);

        // Open gate after hold.
        let gate_path = hot_gate.clone();
        let gate_hold = Duration::from_millis(gate_hold_ms);
        let gate_opener = tokio::spawn(async move {
            sleep(gate_hold).await;
            write_text_file(&gate_path, "go\n").await;
        });

        // Query task.
        let mgr_for_query = manager.clone();
        let query_task = tokio::spawn(async move {
            collect_query_latencies(
                cfg.window_ms,
                cfg.query_pace_ms,
                cfg.timeout_ms,
                || mgr_for_query.get_all_process_info(),
            )
            .await
        });

        // Cold churn cycles.
        let churn_task_count = cfg.concurrency.min(4).max(1);
        let mut churn_handles = Vec::new();
        for task_idx in 0..churn_task_count {
            let mgr = manager.clone();
            let ids = cold_ids.clone();
            let stop_timeout_ms = cfg.stop_timeout_ms;
            let churn_pace_ms = cfg.churn_pace_ms;
            churn_handles.push(tokio::spawn(async move {
                let mut stats = ChurnStats::default();
                let mut idx = task_idx;
                while Instant::now() < deadline {
                    if ids.is_empty() {
                        sleep(Duration::from_millis(churn_pace_ms)).await;
                        continue;
                    }
                    let id = &ids[idx % ids.len()];
                    idx += churn_task_count;

                    stats.attempted_cycles += 1;
                    let stop_ok = timeout(Duration::from_millis(stop_timeout_ms), mgr.stop_process(id))
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);
                    if !stop_ok {
                        stats.stop_errors += 1;
                        stats.failed_cycles += 1;
                        sleep(Duration::from_millis(churn_pace_ms)).await;
                        continue;
                    }
                    let start_ok = timeout(Duration::from_millis(stop_timeout_ms), mgr.start_process(id))
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);
                    if !start_ok {
                        stats.start_errors += 1;
                        stats.failed_cycles += 1;
                    } else {
                        stats.successful_cycles += 1;
                    }
                    sleep(Duration::from_millis(churn_pace_ms)).await;
                }
                stats
            }));
        }

        // Keep stop future polled.
        while Instant::now() < deadline {
            tokio::select! {
                _ = stop_fut.as_mut(), if !stop_done => { stop_done = true; }
                _ = sleep(Duration::from_millis(perf_common::STOP_POLL_PACE_MS)) => {}
            }
        }

        let query = query_task.await.expect("query join failed");
        let mut churn = ChurnStats::default();
        for h in churn_handles {
            let s = h.await.expect("churn join failed");
            churn.attempted_cycles += s.attempted_cycles;
            churn.successful_cycles += s.successful_cycles;
            churn.failed_cycles += s.failed_cycles;
            churn.stop_errors += s.stop_errors;
            churn.start_errors += s.start_errors;
        }

        let _ = gate_opener.await;
        if tokio::fs::metadata(&hot_gate).await.is_err() {
            write_text_file(&hot_gate, "go\n").await;
        }
        if !stop_done {
            let _ = timeout(Duration::from_secs(30), stop_fut.as_mut()).await;
        }

        drop(stop_fut);
        let manager_for_shutdown = match Arc::try_unwrap(manager) {
            Ok(m) => m,
            Err(_) => panic!("manager Arc still has outstanding refs (task leak?)"),
        };
        (query, churn, start.elapsed().as_millis(), manager_for_shutdown)
    };

    // Cleanup.
    if tokio::fs::metadata(&hot_gate).await.is_err() {
        write_text_file(&hot_gate, "go\n").await;
    }
    let _ = timeout(Duration::from_secs(120), async {
        let mut m = manager_for_shutdown;
        m.shutdown().await
    })
    .await;

    // Verdict + report.
    let mut verdict = Verdict::Pass;
    let mut reason = "ok".to_string();

    if cfg!(feature = "procman-v2") {
        if let Err(e) = v2_passes_responsiveness(&query, V2_MAX_LATENCY_MS) {
            verdict = Verdict::Fail;
            reason = e;
        } else if query.samples < min_samples {
            verdict = Verdict::Fail;
            reason = format!("insufficient query samples: got {} expected {}", query.samples, min_samples);
        } else if churn.successful_cycles == 0 {
            verdict = Verdict::Fail;
            reason = "cold churn made no successful cycles".to_string();
        }
    } else {
        match v1_exhibits_contention(&query, gate_hold_ms, min_samples) {
            Ok(()) => {}
            Err(e) => {
                verdict = Verdict::Fail;
                reason = e;
            }
        }
    }

    let mut report = ScenarioReport::new(procman_impl_name(), scenario)
        .with_config(&cfg)
        .with_gate_hold_ms(gate_hold_ms)
        .with_min_expected_query_samples(min_samples)
        .with_query_stats(query)
        .with_churn_stats(churn)
        .with_total_elapsed_ms(total_ms)
        .with_verdict(verdict, reason.clone());

    report.port = port;
    report.port_mode = port_mode;
    report.port0_error = port0_error;
    if let Some(ref e) = report.port0_error {
        report.notes.push(format!("port0_failed={}", e.replace('"', "'")));
    }

    report.print_final();

    if cfg!(feature = "procman-v2") {
        assert_eq!(query.timeouts, 0);
        assert!((query.p99_ms() as u128) <= V2_MAX_LATENCY_MS);
        assert!(query.samples >= min_samples);
        assert!(report.churn.as_ref().unwrap().successful_cycles > 0);
    } else {
        assert!(matches!(verdict, Verdict::Pass), "{reason}");
    }
}

 