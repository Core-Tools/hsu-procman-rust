//! Protocol server abstraction.
//!
//! # Rust Learning Note
//!
//! This module demonstrates **server lifecycle management** in Rust.
//!
//! ## ProtocolServer Trait
//!
//! Provides a common interface for different protocol server implementations:
//! - gRPC servers (tonic-based)
//! - HTTP servers (future)
//! - WebSocket servers (future)
//!
//! ## Lifecycle Pattern
//!
//! ```
//! 1. Create server → new()
//! 2. Start server → start() [async]
//! 3. Server runs in background
//! 4. Stop server → stop() [async, graceful]
//! ```
//!
//! ## Rust vs Golang
//!
//! **Golang Pattern:**
//! ```go
//! type ProtocolServer interface {
//!     Start(ctx context.Context) error
//!     Stop(ctx context.Context) error
//!     Protocol() Protocol
//!     Port() int
//! }
//! ```
//!
//! **Rust Pattern:**
//! ```rust
//! #[async_trait]
//! pub trait ProtocolServer: Send + Sync {
//!     async fn start(&mut self) -> Result<()>;
//!     async fn stop(&mut self) -> Result<()>;
//!     fn protocol(&self) -> Protocol;
//!     fn port(&self) -> u16;
//! }
//! ```
//!
//! Key differences:
//! 1. **Rust uses `async fn`** - explicit async/await
//! 2. **`&mut self`** - mutable borrow for state changes
//! 3. **`Send + Sync` bounds** - ensures thread safety
//! 4. **No context** - Rust uses cancellation tokens differently

use async_trait::async_trait;
use hsu_common::{Protocol, Result};
use std::sync::Arc;

/// Visitor for registering protocol-specific service handlers.
///
/// # Architecture
///
/// This trait uses the **visitor pattern** to handle protocol-specific
/// service handler registration. Each protocol server (gRPC, HTTP, etc.)
/// implements its own registrar that knows how to register services for
/// that specific protocol.
///
/// ## The Pattern
///
/// ```text
/// HandlersRegistrar
///     ↓
/// Creates ProtocolServerHandlersVisitor
///     ↓
/// Passes visitor to each ProtocolServer
///     ↓
/// ProtocolServer calls visitor.register_handlers_X()
///     ↓
/// Visitor registers service with protocol-specific logic
/// ```
///
/// ## Comparison with Go
///
/// **Go version:**
/// ```go
/// type ProtocolServerHandlersVisitor interface {
///     RegisterHandlersGRPC(registrar GRPCServiceRegistrar) error
///     RegisterHandlersHTTP(registrar HTTPServiceRegistrar) error
/// }
/// ```
///
/// **Rust version (this trait):**
/// ```rust
/// #[async_trait]
/// pub trait ProtocolServerHandlersVisitor: Send + Sync {
///     async fn register_handlers_grpc(&self, server: Arc<dyn ProtocolServer>) -> Result<()>;
///     async fn register_handlers_http(&self, server: Arc<dyn ProtocolServer>) -> Result<()>;
/// }
/// ```
///
/// Nearly identical! Main differences:
/// - Rust: `async fn` (Go uses context)
/// - Rust: `Send + Sync` bounds (thread safety)
/// - Rust: `Arc<dyn ProtocolServer>` (shared ownership)
///
/// # Rust Learning Note
///
/// ## Why Arc<dyn ProtocolServer>?
///
/// We pass `Arc<dyn ProtocolServer>` instead of a separate registrar because:
/// 1. **Simplicity**: The server itself can act as the registrar
/// 2. **Flexibility**: Server can store registered services internally
/// 3. **Ownership**: Arc allows shared access without borrowing issues
///
/// ## Double Dispatch Pattern
///
/// This is a classic double dispatch:
/// 1. Server calls `visitor.register_handlers_grpc(self)`
/// 2. Visitor casts self to concrete type and registers services
/// 3. Type-safe at runtime!
#[async_trait]
pub trait ProtocolServerHandlersVisitor: Send + Sync {
    /// Register handlers with a gRPC server.
    ///
    /// The visitor will downcast the server to a concrete gRPC server type
    /// and register its service implementations.
    ///
    /// # Arguments
    ///
    /// * `server` - The gRPC protocol server
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Server is not a gRPC server (wrong protocol)
    /// - Registration fails (duplicate service, etc.)
    async fn register_handlers_grpc(&self, server: Arc<dyn ProtocolServer>) -> Result<()>;
    
    /// Register handlers with an HTTP server.
    ///
    /// # Arguments
    ///
    /// * `server` - The HTTP protocol server
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Server is not an HTTP server (wrong protocol)
    /// - Registration fails
    async fn register_handlers_http(&self, server: Arc<dyn ProtocolServer>) -> Result<()>;
}

/// Protocol server trait for different communication protocols.
///
/// # Rust Learning Note
///
/// ## Trait Objects and Lifecycle
///
/// This trait is designed to be used as a trait object:
/// ```rust
/// let server: Box<dyn ProtocolServer> = Box::new(GrpcProtocolServer::new(...));
/// server.start().await?;
/// ```
///
/// ## Send + Sync Requirement
///
/// - **Send**: Can be moved between threads
/// - **Sync**: Can be accessed from multiple threads (via &self)
///
/// These bounds are required for:
/// - Storing in collections shared across tasks
/// - Passing to tokio::spawn
/// - Using in async contexts
///
/// ## Mutable Methods
///
/// `start()` and `stop()` take `&mut self` because they modify internal state:
/// - start: spawns background task, stores handle
/// - stop: sends shutdown signal, clears handle
///
/// **This is safe because Rust's borrow checker prevents concurrent mutations!**
#[async_trait]
pub trait ProtocolServer: Send + Sync {
    /// Returns the protocol this server implements.
    fn protocol(&self) -> Protocol;

    /// Returns the port this server is listening on.
    ///
    /// # Rust Learning Note
    ///
    /// Port is `u16` (not `i32` or `usize`) because:
    /// - TCP/UDP ports are 0-65535 (exactly u16 range)
    /// - Type system prevents invalid values
    /// - No need for validation!
    ///
    /// Compare to Golang's `int` which requires validation.
    fn port(&self) -> u16;
    
    /// Returns the full listen address (e.g., "0.0.0.0:50051").
    ///
    /// This is used for publishing to the service registry.
    fn address(&self) -> String;
    
    /// Register service handlers with this server.
    ///
    /// Uses the visitor pattern to allow protocol-specific handler registration.
    /// The visitor will call the appropriate `register_handlers_X()` method based
    /// on this server's protocol.
    ///
    /// # Arguments
    ///
    /// * `visitor` - Visitor that knows how to register services
    ///
    /// # Errors
    ///
    /// Returns error if registration fails.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Create a visitor for your service
    /// let visitor = EchoHandlersVisitor::new(service_handlers);
    ///
    /// // Register with server (protocol-agnostic!)
    /// server.register_handlers(Arc::new(visitor)).await?;
    /// ```
    async fn register_handlers(
        &self,
        visitor: Arc<dyn ProtocolServerHandlersVisitor>,
    ) -> Result<()>;

    /// Starts the protocol server.
    ///
    /// # Rust Learning Note
    ///
    /// ## Async Start Pattern
    ///
    /// ```rust
    /// async fn start(&mut self) -> Result<()> {
    ///     // 1. Create server components
    ///     let (shutdown_tx, shutdown_rx) = oneshot::channel();
    ///     
    ///     // 2. Spawn background task
    ///     let handle = tokio::spawn(async move {
    ///         server.serve_with_shutdown(shutdown_rx).await
    ///     });
    ///     
    ///     // 3. Store handle and shutdown channel
    ///     self.server_handle = Some(handle);
    ///     self.shutdown_tx = Some(shutdown_tx);
    ///     
    ///     Ok(())
    /// }
    /// ```
    ///
    /// Key points:
    /// - Server runs in background task (`tokio::spawn`)
    /// - Main task continues (non-blocking)
    /// - Store handle for cleanup in `stop()`
    ///
    /// ## Error Handling
    ///
    /// Returns `Result<()>` for startup errors:
    /// - Port already in use
    /// - Invalid configuration
    /// - Resource allocation failure
    async fn start(&self) -> Result<()>;

    /// Stops the protocol server gracefully.
    ///
    /// # Rust Learning Note
    ///
    /// ## Graceful Shutdown Pattern
    ///
    /// ```rust
    /// async fn stop(&mut self) -> Result<()> {
    ///     // 1. Send shutdown signal
    ///     if let Some(tx) = self.shutdown_tx.take() {
    ///         tx.send(()).ok();
    ///     }
    ///     
    ///     // 2. Wait for server to stop
    ///     if let Some(handle) = self.server_handle.take() {
    ///         handle.await??;
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    ///
    /// Key points:
    /// - `take()` moves value out of Option, leaving None
    /// - Ensures cleanup happens only once
    /// - `await` waits for background task to finish
    ///
    /// ## Idempotency
    ///
    /// Using `Option::take()` makes this idempotent:
    /// - First call: stops server
    /// - Subsequent calls: no-op (already stopped)
    async fn stop(&self) -> Result<()>;
    
    /// Adds a gRPC service to this server (gRPC-specific).
    ///
    /// This is a protocol-specific method that allows adding gRPC services.
    /// The service adder knows how to add its service to a tonic Router.
    /// By default, this returns an error. Only `GrpcProtocolServer` implements this.
    ///
    /// # Arguments
    ///
    /// * `service_adder` - An object that can add a service to a tonic Router
    ///
    /// # Errors
    ///
    /// Returns error if this is not a gRPC server or if registration fails.
    async fn add_grpc_service_adder(
        &self,
        _service_adder: Arc<dyn crate::grpc_server::GrpcServiceAdder>,
    ) -> Result<()> {
        Err(hsu_common::Error::Validation {
            message: format!("Protocol server {:?} does not support gRPC services", self.protocol()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Mock server for testing
    struct MockServer {
        protocol: Protocol,
        port: u16,
        started: Arc<AtomicBool>,
    }

    impl MockServer {
        fn new(port: u16) -> Self {
            Self {
                protocol: Protocol::Grpc,
                port,
                started: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[async_trait]
    impl ProtocolServer for MockServer {
        fn protocol(&self) -> Protocol {
            self.protocol
        }

        fn port(&self) -> u16 {
            self.port
        }
        
        fn address(&self) -> String {
            format!("0.0.0.0:{}", self.port)
        }
        
        async fn register_handlers(
            &self,
            _visitor: Arc<dyn ProtocolServerHandlersVisitor>,
        ) -> Result<()> {
            Ok(())
        }

        async fn start(&self) -> Result<()> {
            self.started.store(true, Ordering::SeqCst);
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            self.started.store(false, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_protocol_server_lifecycle() {
        let server = MockServer::new(50051);

        assert_eq!(server.protocol(), Protocol::Grpc);
        assert_eq!(server.port(), 50051);
        assert!(!server.started.load(Ordering::SeqCst));

        // Start
        server.start().await.unwrap();
        assert!(server.started.load(Ordering::SeqCst));

        // Stop
        server.stop().await.unwrap();
        assert!(!server.started.load(Ordering::SeqCst));
    }

    #[test]
    fn test_address() {
        let server = MockServer::new(8080);
        assert_eq!(server.address(), "0.0.0.0:8080");
    }

    #[tokio::test]
    async fn test_trait_object() {
        let server: Box<dyn ProtocolServer> = Box::new(MockServer::new(3000));

        server.start().await.unwrap();
        assert_eq!(server.port(), 3000);
        server.stop().await.unwrap();
    }
}

