//! E2E Integration Tests for HSU Process Manager

// Phase 1: Basic Lifecycle Management
mod test_graceful_stop;
mod test_forced_stop;
mod test_log_collection;

// Phase 2: Health Monitoring & Restart
mod test_health_restart;
mod test_http_health;

// Phase 3: Resource Limits
mod test_memory_limit;

// Performance: procman-v1 vs procman-v2 comparison
mod perf_procman_v1_vs_v2;

