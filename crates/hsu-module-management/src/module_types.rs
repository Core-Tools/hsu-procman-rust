//! Core module types and abstractions.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates the key difference between Go and Rust:
//! - Go uses `interface{}` for protocol-agnostic abstractions
//! - Rust uses **enums** (sum types) for type-safe variants
//! 
//! ## The Key Pattern: Type Erasure (Like Go's interface{})
//! 
//! ```go
//! // Go: Type erasure with interface{}
//! type ServiceHandler interface{}  // Can be anything!
//! 
//! handler := handlers[id]
//! concreteHandler := handler.(ConcreteType)  // Runtime cast
//! ```
//! 
//! ```rust,ignore
//! // Rust: Type erasure with dyn Any (domain-agnostic!)
//! pub struct ServiceHandler(Arc<dyn Any + Send + Sync>);
//! 
//! let handler = handlers.get(&id)?;
//! if let Some(echo) = handler.downcast_ref::<EchoService>() {
//!     // Use echo service
//! } else if let Some(storage) = handler.downcast_ref::<StorageService>() {
//!     // Use storage service
//! }
//! ```
//! 
//! **Key Principle:** Framework stays domain-agnostic!
//! Domain-specific types belong in Layer 2 (Domain) or Layer 5 (Application Glue).

use async_trait::async_trait;
use hsu_common::{ModuleID, ServiceID, Protocol, Result};
use std::collections::HashMap;
use std::sync::Arc;

/// Service handler - server-side abstraction for a service.
/// 
/// # Rust Learning Note
/// 
/// This is the framework's domain-agnostic handler type.
/// It uses **type erasure** (similar to Go's `interface{}`).
/// 
/// ## Type Erasure in Rust
/// 
/// **Go approach:**
/// ```go
/// type ServiceHandler interface{}  // Can be anything!
/// handler := handlers[id]
/// concreteHandler := handler.(ConcreteType)  // Runtime cast
/// ```
/// 
/// **Rust approach:**
/// ```rust
/// // Store as Any trait object - domain-agnostic!
/// pub struct ServiceHandler(Arc<dyn Any + Send + Sync>);
/// 
/// // Application layer can downcast to concrete type
/// let concrete: &ConcreteService = handler.downcast_ref::<ConcreteService>()?;
/// ```
/// 
/// ## Why Arc<dyn Any>?
/// 
/// 1. **Domain-agnostic**: Framework has no knowledge of specific services
/// 2. **Type-safe**: Runtime type checking via `downcast_ref`
/// 3. **Thread-safe**: `Send + Sync` ensures cross-thread safety
/// 4. **Cheap cloning**: Arc is just incrementing a counter
/// 
/// ## Framework Layer Principle
/// 
/// The framework must remain **domain-agnostic**. Domain-specific types
/// (like `EchoService`, `StorageService`) belong in Layer 2 (Domain) or
/// Layer 5 (Application Glue), not in Layer 1 (Framework).
#[derive(Clone)]
pub struct ServiceHandler(Arc<dyn std::any::Any + Send + Sync>);

impl ServiceHandler {
    /// Creates a new service handler from any type-erased service.
    /// 
    /// # Example
    /// 
    /// ```rust,ignore
    /// let echo_service = EchoServiceImpl::new();
    /// let handler = ServiceHandler::new(echo_service);
    /// ```
    pub fn new<T: std::any::Any + Send + Sync>(service: T) -> Self {
        Self(Arc::new(service))
    }

    /// Gets a reference to the inner type-erased service.
    /// 
    /// # Returns
    /// 
    /// Returns `Some(&T)` if the handler contains type `T`, otherwise `None`.
    /// 
    /// # Example
    /// 
    /// ```rust,ignore
    /// if let Some(echo) = handler.downcast_ref::<EchoServiceImpl>() {
    ///     // Use echo service
    /// }
    /// ```
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Gets the inner Arc for advanced use cases.
    pub fn inner(&self) -> &Arc<dyn std::any::Any + Send + Sync> {
        &self.0
    }
}

impl std::fmt::Debug for ServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ServiceHandler(<type-erased>)")
    }
}

/// Service gateway - client-side abstraction for a service.
/// 
/// # Rust Learning Note
/// 
/// This enum represents how to reach a service:
/// - `Direct`: In-process call (zero overhead)
/// - `Grpc`: Cross-process gRPC call
/// - `Http`: Cross-process HTTP call
/// 
/// The key insight: the **gateway type** tells you the communication method,
/// and the **inner type** tells you which service it talks to.
/// 
/// ## Memory Layout
/// 
/// ```text
/// ServiceGateway in memory:
/// [discriminant: u8][data: actual gateway]
///     ^                    ^
///     0 = Direct           If 0, contains ServiceHandler
///     1 = Grpc             If 1, contains GrpcGateway
///     2 = Http             If 2, contains HttpGateway
/// ```
/// 
/// Pattern matching on this is just an integer comparison - very fast!
#[derive(Clone)]
pub enum ServiceGateway {
    /// Direct in-process gateway (holds the actual handler).
    Direct(Arc<ServiceHandler>),
    
    /// gRPC gateway for cross-process communication.
    Grpc(GrpcGateway),
    
    /// HTTP gateway for cross-process communication.
    Http(HttpGateway),
}

impl std::fmt::Debug for ServiceGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceGateway::Direct(handler) => write!(f, "ServiceGateway::Direct({:?})", handler),
            ServiceGateway::Grpc(gateway) => write!(f, "ServiceGateway::Grpc({:?})", gateway),
            ServiceGateway::Http(gateway) => write!(f, "ServiceGateway::Http({:?})", gateway),
        }
    }
}

// No domain-specific methods here!
// Framework stays domain-agnostic.
//
// For Go-like simplicity, use extension traits in your application code.
// See the echo example for the pattern.

/// gRPC gateway - domain-agnostic client for gRPC services.
/// 
/// # Rust Learning Note
/// 
/// Like `ServiceHandler`, this uses **type erasure** to remain domain-agnostic.
/// 
/// The framework doesn't need to know about specific gRPC clients (EchoClient,
/// StorageClient, etc.). It just holds a type-erased reference that the
/// application layer can downcast to the concrete type.
/// 
/// ## Usage Pattern
/// 
/// **Framework layer (this):**
/// ```rust
/// // Domain-agnostic storage
/// pub struct GrpcGateway(Arc<dyn Any + Send + Sync>);
/// ```
/// 
/// **Application layer:**
/// ```rust
/// // Downcast to concrete client
/// let echo_client = gateway.downcast_ref::<EchoGrpcClient>()?;
/// let result = echo_client.echo("hello").await?;
/// ```
#[derive(Clone)]
pub struct GrpcGateway(Arc<dyn std::any::Any + Send + Sync>);

impl GrpcGateway {
    /// Creates a new gRPC gateway from any gRPC client.
    pub fn new<T: std::any::Any + Send + Sync>(client: T) -> Self {
        Self(Arc::new(client))
    }

    /// Gets a reference to the concrete client type.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Gets the inner Arc for advanced use cases.
    pub fn inner(&self) -> &Arc<dyn std::any::Any + Send + Sync> {
        &self.0
    }
}

impl std::fmt::Debug for GrpcGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcGateway(<type-erased>)")
    }
}

/// HTTP gateway - domain-agnostic client for HTTP services (future).
/// 
/// # Rust Learning Note
/// 
/// Like `GrpcGateway`, this uses type erasure for domain-agnostic HTTP clients.
/// Currently a placeholder for future HTTP protocol support.
#[derive(Clone)]
pub struct HttpGateway(Arc<dyn std::any::Any + Send + Sync>);

impl HttpGateway {
    /// Creates a new HTTP gateway from any HTTP client.
    pub fn new<T: std::any::Any + Send + Sync>(client: T) -> Self {
        Self(Arc::new(client))
    }

    /// Gets a reference to the concrete client type.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    /// Gets the inner Arc for advanced use cases.
    pub fn inner(&self) -> &Arc<dyn std::any::Any + Send + Sync> {
        &self.0
    }
}

impl std::fmt::Debug for HttpGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpGateway(<type-erased>)")
    }
}

/// Map of service IDs to service handlers.
/// 
/// # Rust Learning Note
/// 
/// Unlike Go's `map[ServiceID]interface{}`, this HashMap stores
/// **ServiceHandler enums**, not `dyn Any`. This means:
/// - Type-safe: Can't accidentally store wrong type
/// - Pattern matching: Extract the right variant with match
/// - No unsafe: No downcasting needed!
pub type ServiceHandlersMap = HashMap<ServiceID, ServiceHandler>;

/// Factory for creating service gateways.
/// 
/// # Rust Learning Note
/// 
/// This is a **trait** (similar to Go's interface). Unlike Go's empty
/// interface, Rust traits define actual methods that must be implemented.
/// 
/// ## async_trait
/// 
/// The `#[async_trait]` macro allows us to use `async fn` in traits.
/// Without it, async methods in traits aren't supported yet in stable Rust.
#[async_trait]
pub trait ServiceGatewayFactory: Send + Sync {
    /// Creates a new service gateway for the specified module and service.
    /// 
    /// # Arguments
    /// - `module_id`: The target module
    /// - `service_id`: The target service within the module
    /// - `protocol`: The desired communication protocol
    /// 
    /// # Returns
    /// A `ServiceGateway` enum variant representing how to reach the service.
    /// 
    /// # Rust Learning Note
    /// 
    /// This returns `Result<ServiceGateway>`, not `Result<Box<dyn Any>>`.
    /// The caller knows exactly what they're getting - a ServiceGateway enum!
    async fn new_service_gateway(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        protocol: Protocol,
    ) -> Result<ServiceGateway>;
}

/// User-provided factory for creating protocol-specific service gateways.
/// 
/// # Rust Learning Note
/// 
/// This trait allows users to provide their own gateway creation logic,
/// similar to Go's `GatewayFactoryFunc` pattern.
/// 
/// ## Pattern: User-Provided Factory
/// 
/// **Go equivalent:**
/// ```go
/// type ProtocolServiceGatewayFactoryFunc func(
///     protocolClientConnection moduleproto.ProtocolClientConnection,
///     logger logging.Logger,
/// ) moduletypes.ServiceGateway
/// 
/// // User provides:
/// GatewayFactoryFunc: grpcapi.NewGRPCGateway1
/// ```
/// 
/// **Rust version:**
/// ```rust
/// #[async_trait]
/// impl ProtocolGatewayFactory for EchoGrpcGatewayFactory {
///     async fn create_gateway(&self, address: String) -> Result<ServiceGateway> {
///         // Connect to gRPC service
///         let gateway = EchoGrpcGateway::connect(address).await?;
///         
///         // Wrap in domain-agnostic GrpcGateway (type-erased)
///         Ok(ServiceGateway::Grpc(GrpcGateway::new(gateway)))
///     }
/// }
/// ```
/// 
/// ## Why This Design?
/// 
/// 1. **Type Safety**: User creates properly-typed gateway
/// 2. **Flexibility**: User controls connection/initialization
/// 3. **Testability**: Easy to mock factories
/// 4. **Async-friendly**: Native async support via async_trait
/// 
/// ## Comparison to ServiceGatewayFactory
/// 
/// - `ServiceGatewayFactory`: Framework-level, handles discovery & routing
/// - `ProtocolGatewayFactory`: User-level, creates service-specific gateways
/// 
/// The framework calls user factories to create the actual gateway implementations.
#[async_trait]
pub trait ProtocolGatewayFactory: Send + Sync {
    /// Creates a service-specific gateway for the given address.
    /// 
    /// # Arguments
    /// - `address`: The service address (from service discovery)
    /// 
    /// # Returns
    /// A `ServiceGateway` with the concrete service implementation.
    /// 
    /// # Example
    /// 
    /// ```rust,ignore
    /// use hsu_module_management::ProtocolGatewayFactory;
    /// 
    /// pub struct EchoGrpcGatewayFactory;
    /// 
    /// #[async_trait]
    /// impl ProtocolGatewayFactory for EchoGrpcGatewayFactory {
    ///     async fn create_gateway(&self, address: String) -> Result<ServiceGateway> {
    ///         // Connect to service
    ///         let gateway = EchoGrpcGateway::connect(address).await?;
    ///         
    ///         // Wrap in domain-agnostic ServiceGateway (type-erased)
    ///         Ok(ServiceGateway::Grpc(GrpcGateway::new(gateway)))
    ///     }
    /// }
    /// ```
    async fn create_gateway(&self, address: String) -> Result<ServiceGateway>;
}

/// Module trait - represents a module in the HSU system.
/// 
/// # Rust Learning Note
/// 
/// This trait is equivalent to Go's Module interface, but with some
/// Rust-specific additions:
/// - `Send + Sync`: Required for thread safety (can be shared across threads)
/// - `async_trait`: Allows async methods
#[async_trait]
pub trait Module: Send + Sync {
    /// Returns the module's unique identifier.
    fn id(&self) -> &ModuleID;

    /// Returns the map of service handlers this module provides.
    /// 
    /// # Rust Learning Note
    /// 
    /// Returning `Option<ServiceHandlersMap>` allows modules to be
    /// client-only (no services provided). This is more explicit than
    /// Go's returning nil.
    fn service_handlers_map(&self) -> Option<ServiceHandlersMap>;

    /// Sets the service gateway factory for this module.
    /// 
    /// The factory is used to create gateways to other modules' services.
    fn set_service_gateway_factory(&mut self, factory: Arc<dyn ServiceGatewayFactory>);

    /// Starts the module.
    async fn start(&mut self) -> Result<()>;

    /// Stops the module gracefully.
    async fn stop(&mut self) -> Result<()>;
}

// NOTE: Domain-specific service traits (like EchoService, StorageService, etc.)
// belong in Layer 2 (Domain) or Layer 5 (Application Glue), NOT here in the
// framework (Layer 1).
//
// The framework remains completely domain-agnostic.
//
// See the echo-contract crate in hsu-example1-rust for examples of
// domain-specific service traits.

#[cfg(test)]
mod tests {
    use super::*;

    // Mock service for testing (domain-agnostic)
    struct MockService {
        value: String,
    }

    impl MockService {
        fn new(value: String) -> Self {
            Self { value }
        }

        fn get_value(&self) -> &str {
            &self.value
        }
    }

    #[test]
    fn test_service_handler_type_erasure() {
        // Create a handler with type-erased service
        let mock_service = MockService::new("test".to_string());
        let handler = ServiceHandler::new(mock_service);
        
        // Downcast to concrete type
        let concrete = handler.downcast_ref::<MockService>().unwrap();
        assert_eq!(concrete.get_value(), "test");
    }

    #[test]
    fn test_service_handler_wrong_type() {
        // Create a handler with one type
        let mock_service = MockService::new("test".to_string());
        let handler = ServiceHandler::new(mock_service);
        
        // Try to downcast to wrong type
        let result = handler.downcast_ref::<String>();
        assert!(result.is_none());
    }

    #[test]
    fn test_service_gateway_direct() {
        // Create a direct gateway with type-erased handler
        let mock_service = MockService::new("test".to_string());
        let handler = ServiceHandler::new(mock_service);
        let gateway = ServiceGateway::Direct(Arc::new(handler));
        
        // Pattern match on gateway type
        match gateway {
            ServiceGateway::Direct(handler) => {
                // Extract and verify the service
                let concrete = handler.downcast_ref::<MockService>().unwrap();
                assert_eq!(concrete.get_value(), "test");
            }
            _ => panic!("Expected direct gateway"),
        }
    }

    #[test]
    fn test_grpc_gateway_type_erasure() {
        // Simulate a gRPC client (domain-agnostic)
        struct MockGrpcClient {
            address: String,
        }

        let client = MockGrpcClient {
            address: "localhost:50051".to_string(),
        };
        let gateway = GrpcGateway::new(client);

        // Downcast to concrete type
        let concrete = gateway.downcast_ref::<MockGrpcClient>().unwrap();
        assert_eq!(concrete.address, "localhost:50051");
    }

    #[test]
    fn test_http_gateway_type_erasure() {
        // Simulate an HTTP client (domain-agnostic)
        struct MockHttpClient {
            base_url: String,
        }

        let client = MockHttpClient {
            base_url: "http://localhost:8080".to_string(),
        };
        let gateway = HttpGateway::new(client);

        // Downcast to concrete type
        let concrete = gateway.downcast_ref::<MockHttpClient>().unwrap();
        assert_eq!(concrete.base_url, "http://localhost:8080");
    }
}

