use clap::Parser;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::{sleep, interval};
use tracing::{info, warn, error, debug};

/// Test executable for HSU Process Manager E2E testing
#[derive(Parser, Debug)]
#[command(name = "testexe")]
#[command(about = "Test executable for process manager testing", long_about = None)]
struct Args {
    /// Duration in seconds to run before exiting (0 = run indefinitely)
    #[arg(long, default_value = "0")]
    run_duration: u64,

    /// Memory in Megabytes to allocate and hold
    #[arg(long, default_value = "0")]
    memory_mb: usize,

    /// Exit code to return on shutdown (for testing failure scenarios)
    #[arg(long, default_value = "0")]
    exit_code: i32,

    /// Delay in seconds before responding to shutdown signal
    #[arg(long, default_value = "0")]
    shutdown_delay: u64,

    /// Memory growth rate in MB per second (for testing resource limits)
    #[arg(long, default_value = "0")]
    memory_growth_rate: usize,

    /// Port to listen on for HTTP health checks
    #[arg(long)]
    health_port: Option<u16>,

    /// Seconds after startup to start failing health checks
    #[arg(long)]
    fail_health_after: Option<u64>,

    /// PID file path to write process ID
    #[arg(long)]
    pid_file: Option<String>,

    /// Seconds to delay before considering startup complete
    #[arg(long, default_value = "0")]
    startup_delay: u64,

    /// Seconds after startup to crash/panic
    #[arg(long)]
    crash_after: Option<u64>,

    /// CPU percentage to use (0-100)
    #[arg(long, default_value = "0")]
    cpu_percent: u32,
}

#[tokio::main]
async fn main() {
    // Initialize tracing with structured logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_line_number(false)
        .with_file(false)
        .init();

    let args = Args::parse();
    info!("Starting testexe with args: {:?}", args);

    // Write PID file if requested
    if let Some(pid_file_path) = &args.pid_file {
        if let Err(e) = write_pid_file(pid_file_path) {
            error!("Failed to write PID file: {}", e);
            std::process::exit(1);
        }
        info!("Wrote PID to file: {}", pid_file_path);
    }

    // Setup shutdown signal handler
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    #[cfg(windows)]
    {
        if let Err(e) = setup_windows_signal_handler(shutdown_clone) {
            error!("Failed to setup signal handler: {}", e);
            std::process::exit(1);
        }
    }

    #[cfg(unix)]
    {
        tokio::spawn(async move {
            setup_unix_signal_handler(shutdown_clone).await;
        });
    }

    // Allocate initial memory if requested
    let mut memory_holder: Vec<Vec<u8>> = Vec::new();
    if args.memory_mb > 0 {
        info!("Allocating {} MB of memory", args.memory_mb);
        allocate_memory(&mut memory_holder, args.memory_mb);
        info!("Memory allocation complete. Current: {} MB", memory_holder.len());
    }

    // Startup delay if requested
    if args.startup_delay > 0 {
        info!("Startup delay: waiting {} seconds", args.startup_delay);
        sleep(Duration::from_secs(args.startup_delay)).await;
    }

    info!("Testexe is ready, starting managed processes...");
    sleep(Duration::from_millis(100)).await;
    info!("Testexe is fully operational");

    // Start health check server if requested
    let health_task = if let Some(port) = args.health_port {
        let fail_after = args.fail_health_after;
        Some(tokio::spawn(async move {
            run_health_server(port, fail_after).await;
        }))
    } else {
        None
    };

    // Start CPU stress if requested
    let cpu_task = if args.cpu_percent > 0 {
        info!("Starting CPU stress at {}%", args.cpu_percent);
        Some(tokio::spawn(async move {
            run_cpu_stress(args.cpu_percent).await;
        }))
    } else {
        None
    };

    // Start crash timer if requested
    if let Some(crash_after) = args.crash_after {
        let shutdown_clone = shutdown.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(crash_after)).await;
            if !shutdown_clone.load(Ordering::Relaxed) {
                error!("Testexe crashing after {} seconds as requested!", crash_after);
                panic!("Simulated crash");
            }
        });
    }

    // Main loop
    let mut ticker = interval(Duration::from_secs(2));
    let start_time = std::time::Instant::now();
    let run_duration_secs = args.run_duration;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Check if we should exit due to duration
                if run_duration_secs > 0 {
                    let elapsed = start_time.elapsed().as_secs();
                    if elapsed >= run_duration_secs {
                        info!("Run duration ({} seconds) reached, exiting", run_duration_secs);
                        break;
                    }
                    debug!("Running... ({}/{} seconds)", elapsed, run_duration_secs);
                }

                // Memory growth if requested
                if args.memory_growth_rate > 0 {
                    let elapsed = start_time.elapsed().as_secs();
                    let target_mb = args.memory_mb + (args.memory_growth_rate * elapsed as usize);
                    if memory_holder.len() < target_mb {
                        allocate_memory(&mut memory_holder, 1);
                        if memory_holder.len() % 10 == 0 {
                            info!("Memory usage growing: {} MB", memory_holder.len());
                        }
                    }
                }
            }
            _ = wait_for_shutdown(shutdown.clone()) => {
                info!("Testexe received signal");
                break;
            }
        }
    }

    // Shutdown delay if requested
    if args.shutdown_delay > 0 {
        info!("Shutdown delay: waiting {} seconds", args.shutdown_delay);
        sleep(Duration::from_secs(args.shutdown_delay)).await;
    }

    // Cleanup
    if let Some(task) = health_task {
        task.abort();
    }
    if let Some(task) = cpu_task {
        task.abort();
    }

    // Clean up PID file
    if let Some(pid_file_path) = &args.pid_file {
        if let Err(e) = std::fs::remove_file(pid_file_path) {
            warn!("Failed to remove PID file: {}", e);
        }
    }

    info!("Testexe stopped");
    std::process::exit(args.exit_code);
}

fn allocate_memory(holder: &mut Vec<Vec<u8>>, megabytes: usize) {
    for _ in 0..megabytes {
        let mut chunk = vec![0u8; 1024 * 1024]; // 1 MB
        // Touch the memory to ensure it's actually allocated
        for i in (0..chunk.len()).step_by(4096) {
            chunk[i] = 42;
        }
        holder.push(chunk);
    }
}

fn write_pid_file(path: &str) -> std::io::Result<()> {
    let pid = std::process::id();
    std::fs::write(path, pid.to_string())
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(unix)]
async fn setup_unix_signal_handler(shutdown: Arc<AtomicBool>) {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to setup SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to setup SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("Received SIGTERM");
            shutdown.store(true, Ordering::Relaxed);
        }
        _ = sigint.recv() => {
            info!("Received SIGINT");
            shutdown.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(windows)]
fn setup_windows_signal_handler(shutdown: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::TRUE;
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT, CTRL_BREAK_EVENT};

    // Global shutdown flag for Windows signal handler
    static SHUTDOWN_FLAG: once_cell::sync::Lazy<Mutex<Option<Arc<AtomicBool>>>> = 
        once_cell::sync::Lazy::new(|| Mutex::new(None));

    *SHUTDOWN_FLAG.lock().unwrap() = Some(shutdown);

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        match ctrl_type {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => {
                if let Some(shutdown) = SHUTDOWN_FLAG.lock().unwrap().as_ref() {
                    info!("Received Windows console control signal");
                    shutdown.store(true, Ordering::Relaxed);
                }
                TRUE
            }
            _ => 0,
        }
    }

    unsafe {
        if SetConsoleCtrlHandler(Some(handler), TRUE) == 0 {
            return Err("Failed to set console control handler".into());
        }
    }

    Ok(())
}

async fn run_health_server(port: u16, fail_after: Option<u64>) {
    let start_time = std::time::Instant::now();
    let addr = format!("127.0.0.1:{}", port);
    
    info!("Health check server listening on http://{}", addr);
    
    // Simple HTTP server without hyper dependency
    // For now, just log that we would start it
    // In Phase 2, we'll implement this with a simple TCP server
    warn!("Health check server not fully implemented yet - would listen on {}", addr);
    
    // Keep alive and simulate health status
    loop {
        let elapsed = start_time.elapsed().as_secs();
        let should_fail = fail_after.map_or(false, |secs| elapsed >= secs);
        
        if should_fail {
            debug!("Health check would return: 503 Unhealthy");
        } else {
            debug!("Health check would return: 200 OK");
        }
        
        sleep(Duration::from_secs(5)).await;
    }
}

async fn run_cpu_stress(target_percent: u32) {
    let target_percent = target_percent.min(100);
    
    // Calculate work/sleep ratio
    // For X% CPU: work for X ms, sleep for (100-X) ms per 100ms window
    let work_ms = target_percent as u64;
    let sleep_ms = 100 - work_ms;
    
    info!("CPU stress: working {}ms, sleeping {}ms per 100ms window", work_ms, sleep_ms);
    
    loop {
        // CPU work
        let work_until = std::time::Instant::now() + Duration::from_millis(work_ms);
        while std::time::Instant::now() < work_until {
            // Busy work
            let mut _x = 0u64;
            for i in 0..1000 {
                _x = _x.wrapping_add(i);
            }
        }
        
        // Sleep
        if sleep_ms > 0 {
            sleep(Duration::from_millis(sleep_ms)).await;
        }
    }
}

