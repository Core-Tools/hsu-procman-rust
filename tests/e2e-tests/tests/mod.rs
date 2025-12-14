//! E2E Integration Tests for HSU Process Manager

// Phase 1: Basic Lifecycle Management
mod test_graceful_stop;
mod test_forced_stop;
mod test_log_collection;
mod test_stop_waits_for_exit;

// Phase 2: Health Monitoring & Restart
mod test_health_restart;
mod test_http_health;

// Phase 3: Resource Limits
mod test_memory_limit;

// Note: Performance tests are run as separate binaries via --test flag:
//   cargo test -p e2e-tests --test perf_procman_v1_vs_v2 -- --ignored --nocapture
//   cargo test -p e2e-tests --test perf_concurrent_ops_isolation -- --ignored --nocapture
//   cargo test -p e2e-tests --test perf_hot_process_isolation -- --ignored --nocapture
//   cargo test -p e2e-tests --test perf_read_heavy_write_heavy -- --ignored --nocapture
//   cargo test -p e2e-tests --test perf_cancellation_semantics -- --ignored --nocapture

