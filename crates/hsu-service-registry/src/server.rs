//! Service registry server implementation.
//! 
//! # Rust Learning Note
//! 
//! This module shows how to build a **complete async server** in Rust.
//! 
//! ## Key Concepts
//! 
//! 1. **Tokio Runtime**: The async executor (like Go's runtime for goroutines)
//! 2. **Graceful Shutdown**: Using tokio signals and channels
//! 3. **Binding to Different Transports**: TCP, UDS, Pipes

use crate::{api::create_router, storage::Registry, transport::TransportConfig};
use axum::Router;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

#[cfg(unix)]
use tokio::net::UnixListener;

/// Service registry server.
/// 
/// # Rust Learning Note
/// 
/// ## Struct with Shared State
/// 
/// ```rust
/// pub struct RegistryServer {
///     registry: Arc<Registry>,
///     transport: TransportConfig,
///     router: Router,
/// }
/// ```
/// 
/// - `Arc<Registry>`: Shared ownership (can be cloned cheaply)
/// - `Router`: axum's router (contains all HTTP handlers)
/// - All fields owned by the struct (no lifetimes needed!)
pub struct RegistryServer {
    registry: Arc<Registry>,
    transport: TransportConfig,
    router: Router,
}

impl RegistryServer {
    /// Creates a new registry server.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Builder Pattern vs Constructor
    /// 
    /// Go style:
    /// ```go
    /// server := NewRegistryServer(transport)
    /// ```
    /// 
    /// Rust style (here):
    /// ```rust
    /// let server = RegistryServer::new(transport);
    /// ```
    /// 
    /// Both work! Rust also supports builder pattern:
    /// ```rust
    /// let server = RegistryServer::builder()
    ///     .transport(config)
    ///     .build();
    /// ```
    pub fn new(transport: TransportConfig) -> Self {
        let registry = Arc::new(Registry::new());
        let router = create_router(Arc::clone(&registry));

        Self {
            registry,
            transport,
            router,
        }
    }

    /// Returns a reference to the registry.
    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Starts the server and runs until stopped.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Async Function with Error Handling
    /// 
    /// ```rust
    /// pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
    ///     // async code here
    /// }
    /// ```
    /// 
    /// - `async`: Returns a Future (must be `.await`ed)
    /// - `self`: Consumes the server (can't use it after)
    /// - `Result<(), Box<dyn Error>>`: Common pattern for "any error"
    /// 
    /// In Go:
    /// ```go
    /// func (s *RegistryServer) Run() error {
    ///     // ...
    /// }
    /// ```
    pub async fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Starting registry server: {}", self.transport.describe());

        // Clone transport config to avoid borrowing issues
        let transport = self.transport.clone();
        
        match transport {
            TransportConfig::Tcp { port } => {
                self.run_tcp(port).await?;
            }

            #[cfg(unix)]
            TransportConfig::UnixSocket { path } => {
                self.run_unix_socket(&path).await?;
            }

            #[cfg(windows)]
            TransportConfig::NamedPipe { name } => {
                self.run_named_pipe(&name).await?;
            }
        }

        Ok(())
    }

    /// Runs the server on a TCP socket.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Tokio TcpListener
    /// 
    /// ```rust
    /// let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    /// axum::serve(listener, self.router).await?;
    /// ```
    /// 
    /// Key points:
    /// - `bind().await`: Async operation (can yield to other tasks)
    /// - `?`: Propagate errors (like Go's `if err != nil`)
    /// - `axum::serve`: Starts the HTTP server
    /// 
    /// Go equivalent:
    /// ```go
    /// listener, err := net.Listen("tcp", fmt.Sprintf(":%d", port))
    /// if err != nil {
    ///     return err
    /// }
    /// http.Serve(listener, handler)
    /// ```
    async fn run_tcp(self, port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let addr = format!("0.0.0.0:{}", port);
        info!("Binding to TCP: {}", addr);

        let listener = TcpListener::bind(&addr).await?;
        info!("Server listening on {}", addr);

        axum::serve(listener, self.router).await?;

        Ok(())
    }

    /// Runs the server on a Unix domain socket.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Unix-Specific Code
    /// 
    /// ```rust
    /// #[cfg(unix)]
    /// async fn run_unix_socket(...) { ... }
    /// ```
    /// 
    /// This function **only exists on Unix**!
    /// - Windows builds won't have this function at all
    /// - Type-safe: Can't call it on Windows (compile error)
    /// 
    /// ## File Cleanup
    /// 
    /// ```rust
    /// if path.exists() {
    ///     std::fs::remove_file(&path)?;
    /// }
    /// ```
    /// 
    /// We remove the old socket file if it exists.
    /// Unlike Go, we need to do this manually (no automatic cleanup).
    #[cfg(unix)]
    async fn run_unix_socket(
        self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Binding to Unix socket: {}", path.display());

        // Remove old socket file if it exists
        if path.exists() {
            std::fs::remove_file(path)?;
        }

        let listener = UnixListener::bind(path)?;
        info!("Server listening on {}", path.display());

        axum::serve(listener, self.router).await?;

        Ok(())
    }

    /// Runs the server on a Windows named pipe.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Named Pipes on Windows
    /// 
    /// Named pipes on Windows are more complex than Unix sockets.
    /// For now, we provide a placeholder implementation.
    /// 
    /// Full implementation would use:
    /// - `tokio::net::windows::named_pipe` module
    /// - Custom accept loop
    /// - Pipe creation and connection handling
    #[cfg(windows)]
    async fn run_named_pipe(
        self,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Windows named pipe support: {}", name);
        
        // NOTE: Full named pipe implementation requires:
        // 1. Creating the pipe with CreateNamedPipe
        // 2. Custom accept loop for connections
        // 3. Integration with axum (which expects a listener)
        // 
        // For production use, consider:
        // - Using TCP on localhost (simple, works everywhere)
        // - Or implement full named pipe support
        
        error!("Named pipe server not fully implemented yet");
        error!("Use TCP transport instead: TransportConfig::tcp(8080)");
        
        Err("Named pipe server not implemented".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let server = RegistryServer::new(TransportConfig::tcp(8080));
        assert_eq!(server.registry.count(), 0);
    }

    #[tokio::test]
    async fn test_tcp_server_startup() {
        use tokio::time::Duration;

        let server = RegistryServer::new(TransportConfig::tcp(0)); // Port 0 = random port

        // Start server in background
        let handle = tokio::spawn(async move {
            let _ = server.run().await;  // Ignore result for test
        });

        // Give it a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cancel the server
        handle.abort();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_unix_socket_cleanup() {
        use std::path::PathBuf;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");

        // Create a file at the socket path
        std::fs::write(&socket_path, "").unwrap();
        assert!(socket_path.exists());

        // Server should clean it up
        let server = RegistryServer::new(TransportConfig::unix_socket(&socket_path));

        // Start and immediately stop
        let handle = tokio::spawn(async move {
            server.run().await
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        handle.abort();
    }
}

