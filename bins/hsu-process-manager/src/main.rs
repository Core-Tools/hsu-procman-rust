use anyhow::Result;
use clap::Parser;
use tracing::{error, info};
use tracing_subscriber;

use hsu_process_management::{ProcessManagerConfig, ProcessManager};

/// HSU Process Manager - Rust implementation
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Configuration file path (YAML)
    #[arg(short, long, value_name = "FILE")]
    config: String,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Port to listen on (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Run duration in seconds (for testing)
    #[arg(long)]
    run_duration: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    initialize_logging(args.debug)?;

    info!("Starting HSU Process Manager (Rust implementation)");
    info!("Config file: {}", args.config);

    // Load configuration
    let mut config = ProcessManagerConfig::load_from_file(&args.config)?;
    
    // Override port if specified
    if let Some(port) = args.port {
        config.process_manager.port = port;
    }

    info!("Loaded configuration for {} processes", config.managed_processes.len());

    // Create and start process manager
    let mut process_manager = ProcessManager::new(config).await?;
    
    // Set up signal handlers for graceful shutdown
    let shutdown_signal = setup_signal_handlers();

    // Start the process manager
    match process_manager.start().await {
        Ok(_) => {
            info!("Process manager started successfully");
            
            // Wait for shutdown signal or run duration
            if let Some(duration) = args.run_duration {
                info!("Running for {} seconds (test mode)", duration);
                tokio::time::sleep(tokio::time::Duration::from_secs(duration)).await;
            } else {
                shutdown_signal.await;
            }
            
            info!("Shutting down process manager...");
            process_manager.shutdown().await
                .map_err(|e| anyhow::anyhow!("Shutdown failed: {}", e))?;
            info!("Process manager shut down successfully");
        }
        Err(e) => {
            error!("Failed to start process manager: {}", e);
            return Err(anyhow::anyhow!("Start failed: {}", e));
        }
    }

    Ok(())
}

fn initialize_logging(debug: bool) -> Result<()> {
    let level = if debug { "debug" } else { "info" };
    
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level))
        )
        .with_target(false)
        .with_thread_ids(true)
        .init();

    Ok(())
}

async fn setup_signal_handlers() {
    use tokio::signal;
    
    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to create SIGTERM handler");
        let mut sigint = signal::unix::signal(signal::unix::SignalKind::interrupt())
            .expect("Failed to create SIGINT handler");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT signal");
            }
        }
    }

    #[cfg(windows)]
    {
        let _ = signal::ctrl_c().await;
        info!("Received Ctrl+C signal");
    }
}