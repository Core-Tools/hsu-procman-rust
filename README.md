# HSU Process Manager - Rust Implementation

---

## Overview

The Rust implementation provides a modular, type-safe process management system built on modern async Rust patterns. This implementation manages native processes with features including:

- ✅ **Lifecycle Management** - Process spawning, monitoring, graceful shutdown
- ✅ **Health Monitoring** - HTTP health checks, process liveness detection
- ✅ **Resource Management** - CPU/memory monitoring and limit enforcement
- ✅ **Automatic Restart** - Configurable restart policies with circuit breaker
- ✅ **Cross-Platform** - Windows and Linux support (macOS pending)
- ✅ **State Persistence** - Survive manager restarts via PID files

---

## Quick Start

### Prerequisites

- Rust 1.70+ (install from [rustup.rs](https://rustup.rs))
- C compiler (`gcc` on Linux, MSVC on Windows)
- Linux: `build-essential` package

### Build and Run

```bash
# Build everything
cargo build --workspace

# Build release version
cargo build --workspace --release

# Run process manager with config
cargo run --bin hsu-process-manager -- --config config-example.yaml

# Run tests
cargo test --workspace

# Run E2E tests
cargo test --package e2e-tests -- --nocapture --test-threads=1
```

---

## Project Structure

```
hsu-procman-rust/
├── Cargo.toml              # Workspace configuration
├── README.md               # This file
│
├── crates/                 # Library crates (reusable components)
│   ├── hsu-common/         # Shared types and errors
│   ├── hsu-process/        # Low-level process operations
│   ├── hsu-process-state/  # State machine
│   ├── hsu-process-file/   # PID file persistence
│   ├── hsu-monitoring/     # Health checks
│   ├── hsu-resource-limits/# Resource monitoring
│   ├── hsu-log-collection/ # Log aggregation
│   ├── hsu-managed-process/# Process types & traits
│   └── hsu-process-management/ # Main orchestration
│       ├── process_control_impl.rs  # Core implementation
│       ├── manager.rs               # ProcessManager
│       ├── lifecycle.rs             # Restart policies
│       └── config/                  # Configuration
│
├── bins/                   # Executable binaries
│   └── hsu-process-manager/
│       ├── src/main.rs
│       └── config-example.yaml
│
├── test-utils/             # Test utilities
│   └── testexe/            # Test executable for E2E tests
│       └── src/main.rs
│
└── tests/                  # E2E test suite
    └── e2e-tests/
        ├── src/            # Test framework
        │   ├── process_manager.rs  # PROCMAN wrapper
        │   ├── test_executor.rs    # Test orchestration
        │   ├── log_parser.rs       # Log analysis
        │   └── assertions.rs       # Custom assertions
        └── tests/          # Test scenarios
            ├── test_forced_stop.rs
            ├── test_graceful_stop.rs
            ├── test_health_restart.rs
            └── test_memory_limit.rs
```

---

## Configuration

### Minimal Configuration

```yaml
process_manager:
  port: 50055
  log_level: "info"

managed_processes:
  - id: "my-service"
    type: "standard_managed"
    enabled: true
    management:
      control:
        executable: "./my-service"
        arguments: ["--config", "config.yaml"]
```

### Full Configuration Example

See `bins/hsu-process-manager/config-example.yaml` for complete options including:

- Process types (standard_managed, integrated_managed, unmanaged)
- Health checks (HTTP, gRPC, process liveness)
- Resource limits (memory, CPU, file descriptors)
- Restart policies (never, on_failure, always)
- Logging configuration

---

## Building & Testing

### Build Commands

```bash
# Build workspace
cargo build --workspace

# Build specific crate
cargo build -p hsu-process-management

# Build release
cargo build --release --workspace

# Build binary only
cargo build -p hsu-process-manager

# Check without building
cargo check --workspace
```

### Unit Tests

```bash
# Run all tests
cargo test --workspace

# Test specific crate
cargo test -p hsu-process-state

# Test with output
cargo test -- --nocapture

# Run specific test
cargo test test_state_transitions -- --nocapture
```

### Code Quality

```bash
# Format code
cargo fmt --all

# Lint with Clippy
cargo clippy --workspace -- -D warnings

# Generate documentation
cargo doc --workspace --open
```

---

## End-to-End Testing

### Test Suite Overview

The E2E test suite validates complete process management workflows:

- ✅ **test_forced_stop** - Force kill after timeout
- ✅ **test_graceful_stop** - Graceful shutdown with SIGTERM
- ✅ **test_health_restart** - Automatic restart on process exit
- ✅ **test_memory_limit_violation** - Resource limit enforcement

### Running E2E Tests

**All Tests:**
```bash
cargo test --package e2e-tests -- --nocapture --test-threads=1
```

**Single Test:**
```bash
cargo test --package e2e-tests test_graceful_stop -- --nocapture
```

**Platform-Specific:**

**Linux/WSL2:**
```bash
# Ensure Rust environment
source ~/.cargo/env

# Run tests with timeout
timeout 60s cargo test --package e2e-tests -- --nocapture --test-threads=1
```

**Windows PowerShell:**
```powershell
cargo test --package e2e-tests -- --nocapture --test-threads=1
```

### Test Results

**Status:** ✅ **100% Passing (4/4 tests)**
- **Windows:** All tests passing (~30s total)
- **Linux (WSL2):** All tests passing (~32s total)
- **macOS:** Not yet tested

### Test Artifacts

Test runs preserve artifacts in timestamped directories:

```
target/tmp/
├── e2e-test-forced-stop/
│   └── run-1732363200/
│       ├── config.yaml      # Test configuration
│       └── procman.log      # Process manager logs
├── e2e-test-graceful-stop/
│   └── run-1732363201/
│       ├── config.yaml
│       └── procman.log
└── ...
```

**View artifacts:**
```bash
# List all test runs
ls target/tmp/

# View test config
cat target/tmp/e2e-test-graceful-stop/run-*/config.yaml

# View logs
cat target/tmp/e2e-test-graceful-stop/run-*/procman.log

# Search logs
grep "restart" target/tmp/e2e-test-health-restart/run-*/procman.log
```

**Cleanup:**
```bash
# Remove all test artifacts
rm -rf target/tmp/

# Remove old artifacts (7+ days)
find target/tmp/ -type d -name "run-*" -mtime +7 -exec rm -rf {} +
```

---

## Architecture Overview

### Layered Design

```
┌─────────────────────────────────────────┐
│  ProcessManager (Orchestration)         │  Layer 5
│  - Configuration                        │
│  - Process coordination                 │
│  - API exposure                         │
└──────────────┬──────────────────────────┘
               │ uses
┌──────────────▼──────────────────────────┐
│  ProcessControl Trait (Interface)       │  Layer 4
│  - start() / stop() / restart()         │
│  - get_state() / get_diagnostics()      │
└──────────────┬──────────────────────────┘
               │ implemented by
┌──────────────▼──────────────────────────┐
│  ProcessControlImpl (Implementation)    │  Layer 3
│  - Process spawning                     │
│  - Health monitoring integration        │
│  - Resource monitoring                  │
│  - Restart policy enforcement           │
└──────────────┬──────────────────────────┘
               │ uses
┌──────────────▼──────────────────────────┐
│  Cross-Cutting Concerns (Services)      │  Layer 2
│  - State Machine                        │
│  - Health Monitoring                    │
│  - Resource Limits                      │
│  - Log Collection                       │
└──────────────┬──────────────────────────┘
               │ uses
┌──────────────▼──────────────────────────┐
│  Platform Primitives (Low-level)        │  Layer 1
│  - process_spawn                        │
│  - process_kill                         │
│  - process_exists                       │
└─────────────────────────────────────────┘
```

### Process Types

| Type | Spawn | Monitor | Restart | Terminate | Use Case |
|------|-------|---------|---------|-----------|----------|
| **StandardManaged** | ✅ | ✅ | ✅ | ✅ | Custom apps, services |
| **IntegratedManaged** | ✅ | ✅ (gRPC) | ✅ | ✅ | HSU-aware services |
| **Unmanaged** | ❌ | ✅ | ❌ | ❌ | External systems |

### State Machine

```
     ┌──────────────────────────────────┐
     │                                  │
     ▼                                  │
  Stopped ──────► Starting ──────► Running
     ▲               │                │
     │               ▼                ▼
     │            Failed         Stopping
     │               │                │
     └───────────────┴────────────────┘
```

**Valid Transitions:**
- `Stopped` → `Starting`: Process spawn initiated
- `Starting` → `Running`: Process successfully started
- `Running` → `Stopping`: Graceful shutdown initiated
- `Stopping` → `Stopped`: Process terminated
- `Starting/Running` → `Failed`: Error occurred
- `Failed` → `Starting`: Restart policy triggered

---

## Platform Support

### Windows ✅
- **Status:** Fully tested and working
- **Process APIs:** CreateProcess, TerminateProcess
- **Signals:** Ctrl+C event handling
- **Resource Monitoring:** Windows APIs via sysinfo
- **Tests:** All 4/4 E2E tests passing

### Linux ✅
- **Status:** Fully tested and working (WSL2 Ubuntu 24.04)
- **Process APIs:** fork/exec, kill, waitpid
- **Signals:** SIGTERM, SIGKILL (full POSIX support)
- **Resource Monitoring:** procfs via sysinfo
- **Tests:** All 4/4 E2E tests passing

### macOS ⏳
- **Status:** Not yet tested
- **Expected:** Should work (BSD-based, similar to Linux)
- **Next Step:** Run E2E tests on macOS to verify

---

## Status Summary

**Production Readiness:** ✅ **READY**

| Component | Status | Notes |
|-----------|--------|-------|
| **Core Lifecycle** | ✅ Complete | Spawn, stop, restart |
| **Health Monitoring** | ✅ Complete | HTTP checks, liveness |
| **Resource Limits** | ✅ Complete | Memory, CPU monitoring |
| **Restart Policies** | ✅ Complete | All strategies + circuit breaker |
| **State Machine** | ✅ Complete | Full validation |
| **Configuration** | ✅ Complete | YAML with validation |
| **Cross-Platform** | ✅ Complete | Windows & Linux verified |
| **E2E Tests** | ✅ Complete | 4/4 passing (100%) |
| **Log Collection** | ⏳ Pending | Not yet tested |
| **gRPC Health** | ⏳ Pending | Proto compilation issues |
| **HTTP APIs** | ⏳ Pending | Planned |
