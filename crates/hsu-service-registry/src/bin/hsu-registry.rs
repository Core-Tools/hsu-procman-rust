//! Standalone service registry server.
//! 
//! # Rust Learning Note
//! 
//! This is a **binary crate** (executable), not a library.
//! 
//! ## Binary vs Library
//! 
//! - `src/lib.rs`: Library code (can be used by other crates)
//! - `src/bin/name.rs`: Executable (has a `main` function)
//! 
//! A single crate can have both a library AND multiple binaries!
//! 
//! ## File location
//! 
//! ```
//! hsu-service-registry/
//! ├── src/
//! │   ├── lib.rs         ← Library
//! │   └── bin/
//! │       └── hsu-registry.rs  ← This file! (executable)
//! ```
//! 
//! Build with: `cargo build --bin hsu-registry`

use hsu_service_registry::{RegistryServer, transport::TransportConfig};
use tracing_subscriber;

/// Main entry point.
/// 
/// # Rust Learning Note
/// 
/// ## async main with tokio
/// 
/// ```rust
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // async code here
/// }
/// ```
/// 
/// The `#[tokio::main]` macro transforms this into:
/// ```rust
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     tokio::runtime::Runtime::new()?.block_on(async {
///         // your async code
///     })
/// }
/// ```
/// 
/// It sets up the tokio runtime for us!
/// 
/// ## Go Comparison
/// 
/// In Go, `main` is always sync:
/// ```go
/// func main() {
///     // Goroutines run in background
///     go someAsyncWork()
///     // Must wait explicitly
///     time.Sleep(time.Second)
/// }
/// ```
/// 
/// In Rust, main can be async, and we must `.await`:
/// ```rust
/// #[tokio::main]
/// async fn main() {
///     some_async_work().await;  // Explicit
/// }
/// ```
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (logging)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Parse command-line arguments
    let transport = parse_args();

    // Create and run server
    let server = RegistryServer::new(transport);
    
    tracing::info!("HSU Service Registry starting...");
    tracing::info!("Press Ctrl+C to stop");

    server.run().await?;

    Ok(())
}

/// Parses command-line arguments.
/// 
/// # Rust Learning Note
/// 
/// ## Simple Argument Parsing
/// 
/// ```rust
/// let args: Vec<String> = std::env::args().collect();
/// ```
/// 
/// This is the simple approach. For production, use:
/// - `clap` crate: Full-featured CLI parser
/// - `structopt`: Derive-based (even simpler)
/// 
/// ## Example with clap
/// 
/// ```rust
/// use clap::Parser;
/// 
/// #[derive(Parser)]
/// struct Args {
///     #[arg(short, long, default_value = "8080")]
///     port: u16,
/// }
/// 
/// let args = Args::parse();
/// ```
fn parse_args() -> TransportConfig {
    let args: Vec<String> = std::env::args().collect();

    // Simple argument parsing
    // Usage:
    //   hsu-registry                    # Default platform transport
    //   hsu-registry tcp 8080           # TCP on port 8080
    //   hsu-registry unix /tmp/reg.sock # Unix socket (Unix only)
    //   hsu-registry pipe registry-name # Named pipe (Windows only)

    if args.len() < 2 {
        return TransportConfig::default();
    }

    match args[1].as_str() {
        "tcp" => {
            let port = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(8080);
            TransportConfig::tcp(port)
        }

        #[cfg(unix)]
        "unix" => {
            let path = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("/tmp/hsu-registry.sock");
            TransportConfig::unix_socket(path)
        }

        #[cfg(windows)]
        "pipe" => {
            let name = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("hsu-registry");
            TransportConfig::named_pipe(name)
        }

        _ => {
            eprintln!("Unknown transport: {}", args[1]);
            eprintln!("Usage:");
            eprintln!("  hsu-registry                    # Default");
            eprintln!("  hsu-registry tcp <port>         # TCP");
            #[cfg(unix)]
            eprintln!("  hsu-registry unix <path>        # Unix socket");
            #[cfg(windows)]
            eprintln!("  hsu-registry pipe <name>        # Named pipe");
            std::process::exit(1);
        }
    }
}

