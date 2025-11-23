# TESTEXE - Test Executable for HSU Process Manager

A test executable designed for E2E testing of the HSU Process Manager.

## Features

### Core Features (Priority 1) ✅
- **Graceful shutdown handling** - Responds to SIGTERM/SIGINT, logs shutdown
- **Controlled runtime duration** - `--run-duration <seconds>` to exit after specified time
- **Memory allocation** - `--memory-mb <megabytes>` to allocate and hold memory
- **Exit code control** - `--exit-code <code>` for testing failure scenarios
- **Shutdown delay** - `--shutdown-delay <seconds>` to test timeout scenarios
- **Windows signal handling** - Proper Ctrl+C handling on Windows
- **Structured logging** - Uses tracing for consistent log output

### Enhanced Features (Priority 2)
- **Memory growth** - `--memory-growth-rate <mb/sec>` for gradual memory increase
- **PID file** - `--pid-file <path>` for unmanaged process testing
- **Startup delay** - `--startup-delay <seconds>` to test startup timeouts
- **Crash simulation** - `--crash-after <seconds>` to simulate panics
- **Health check endpoint** - `--health-port <port>` (to be implemented with TCP server)

### Advanced Features (Priority 3)
- **CPU stress** - `--cpu-percent <percent>` to generate CPU load

## Usage

```bash
# Basic run
testexe

# Run for 5 seconds then exit
testexe --run-duration 5

# Allocate 100MB of memory
testexe --memory-mb 100

# Exit with code 1 (failure)
testexe --exit-code 1

# Run for 3 seconds, allocate 50MB, delay shutdown by 2 seconds
testexe --run-duration 3 --memory-mb 50 --shutdown-delay 2

# Gradually increase memory (for testing resource limits)
testexe --memory-growth-rate 10  # Grows by 10MB per second

# Write PID file (for unmanaged process testing)
testexe --pid-file /tmp/testexe.pid

# Simulate crash after 5 seconds
testexe --crash-after 5

# Generate 80% CPU load
testexe --cpu-percent 80

# Health check endpoint (planned)
testexe --health-port 8080 --fail-health-after 10
```

## Expected Behavior

### Normal Operation
1. Starts up and logs "Starting testexe..."
2. Logs "Testexe is ready, starting managed processes..."
3. Logs "Testexe is fully operational"
4. Runs until shutdown signal or duration expires
5. Logs "Testexe received signal" (if signaled)
6. Logs "Testexe stopped"
7. Exits with specified exit code (default 0)

### Graceful Shutdown
When receiving SIGTERM/SIGINT:
1. Logs "Received SIGTERM" or "Received SIGINT"
2. Logs "Testexe received signal"
3. Waits for shutdown delay if configured
4. Cleans up PID file if created
5. Logs "Testexe stopped"
6. Exits with configured exit code

### Memory Allocation
- Allocates memory in 1MB chunks
- Touches memory to ensure it's actually allocated (not just virtual)
- Holds memory for the entire lifetime
- Can grow memory over time with `--memory-growth-rate`

### Signal Handling
- **Unix**: Handles SIGTERM and SIGINT
- **Windows**: Handles Ctrl+C and Ctrl+Break events

## Testing Scenarios

This executable is designed to test:

1. **Process Lifecycle** - Start, run, stop
2. **Graceful Shutdown** - Proper signal handling
3. **Forced Termination** - When graceful timeout expires
4. **Process Exit Detection** - When process exits on its own
5. **Resource Limits** - Memory and CPU monitoring
6. **Health Checks** - Process running check, HTTP checks
7. **Log Collection** - Stdout/stderr capture
8. **Restart Policies** - After failures, resource violations
9. **Startup/Shutdown Timeouts** - Slow processes
10. **Crash Handling** - Unexpected termination

## Build

```bash
cd hsu-core/rust/test-utils/testexe
cargo build --release
```

The executable will be at: `target/release/testexe` (or `testexe.exe` on Windows)

## Implementation Status

- [x] Core Features (Priority 1)
- [x] Enhanced Features (Priority 2) - except health HTTP server
- [x] Advanced Features (Priority 3)
- [ ] HTTP health check server (deferred to Phase 2)

