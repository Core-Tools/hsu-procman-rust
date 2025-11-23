//! Generic Service Gateway Factory
//!
//! # Architecture
//!
//! This module implements `ServiceGatewayFactory<C>` - a **generic** factory
//! that creates typed service gateways without protocol matching.
//!
//! This is the Rust equivalent of Go's `ServiceGatewayFactory[Contract]`.
//!
//! # Key Innovation
//!
//! **The Problem:**
//! - Old approach: Framework returns enum, domain code matches protocols
//! - Result: Protocol knowledge leaks into domain code
//!
//! **The Solution:**
//! - Generic factory returns typed gateway (`Arc<dyn Service>`)
//! - Uses visitor pattern internally for protocol dispatch
//! - Result: Domain code is completely protocol-agnostic!
//!
//! # Rust Learning Note
//!
//! ## Generic Types with Trait Bounds
//!
//! ```rust
//! pub struct ServiceGatewayFactory<C: ?Sized + Send + Sync> {
//!     //                            ^^^^^^^^^^^^^^^^^^^^^^
//!     //                            C can be any trait object
//!     // ...
//! }
//! ```
//!
//! **What does `C: ?Sized` mean?**
//! - `Sized`: Type has known size at compile time
//! - `?Sized`: Type might NOT have known size (e.g., trait objects)
//! - `dyn Service` is NOT Sized (it's a trait object)
//!
//! **Why this matters:**
//! ```rust
//! // Without ?Sized - ERROR!
//! ServiceGatewayFactory<dyn EchoService>  // ❌ dyn EchoService is not Sized
//!
//! // With ?Sized - OK!
//! ServiceGatewayFactory<dyn EchoService>  // ✅ Works!
//! ```
//!
//! ## Comparison with Go
//!
//! **Go version:**
//! ```go
//! type ServiceGatewayFactory[Contract any] struct {
//!     moduleID         ModuleID
//!     serviceID        ServiceID
//!     serviceConnector ServiceConnector
//!     factoryFuncs     GatewayFactoryFuncs[Contract]
//! }
//!
//! func (f *ServiceGatewayFactory[Contract]) NewServiceGateway(
//!     ctx context.Context,
//!     protocol Protocol,
//! ) (Contract, error) {
//!     visitor := NewGatewayFactoryVisitor(f.factoryFuncs)
//!     err := f.serviceConnector.Connect(ctx, f.moduleID, f.serviceID, protocol, visitor)
//!     return visitor.Gateway(), err
//! }
//! ```
//!
//! **Rust version:**
//! ```rust
//! pub struct ServiceGatewayFactory<C: ?Sized + Send + Sync> {
//!     module_id: ModuleID,
//!     service_id: ServiceID,
//!     service_connector: Arc<dyn ServiceConnector>,
//!     factory_funcs: GatewayFactoryFuncs<C>,
//! }
//!
//! impl<C: ?Sized + Send + Sync + 'static> ServiceGatewayFactory<C> {
//!     pub async fn new_service_gateway(&self, protocol: Protocol) -> Result<Arc<C>> {
//!         let mut visitor = GatewayFactoryVisitor::new(&self.factory_funcs);
//!         self.service_connector.connect(&self.module_id, &self.service_id, protocol, &mut visitor).await?;
//!         visitor.gateway()
//!     }
//! }
//! ```
//!
//! Nearly identical! Main differences:
//! - Rust: `?Sized` for trait objects
//! - Rust: `Arc<C>` for shared ownership
//! - Rust: `async`/`.await` for async operations

use std::sync::{Arc, RwLock};
use async_trait::async_trait;
use hsu_common::{ModuleID, ServiceID, Protocol, Result, Error};
use hsu_module_proto::ClientConnectionVisitor;
use tracing::debug;

use crate::service_connector::ServiceConnector;

/// Generic factory for creating typed service gateways.
///
/// This factory uses the visitor pattern to achieve type-safe protocol dispatch
/// without requiring the domain code to know about protocols.
///
/// # Type Parameter
///
/// * `C` - The service contract type (typically `dyn SomeTrait`)
///
/// # Example Usage
///
/// ```rust,ignore
/// // Create factory for EchoService
/// let factory = ServiceGatewayFactory::<dyn EchoService>::new(
///     ModuleID::from("echo"),
///     ServiceID::from("echo-service"),
///     service_connector,
///     GatewayFactoryFuncs {
///         direct: Some(Box::new(|| Ok(direct_handler))),
///         grpc: Some(Box::new(|channel| {
///             Ok(Arc::new(EchoGrpcGateway::new(channel)))
///         })),
///         http: None,
///     },
/// );
///
/// // Get typed gateway (no protocol matching!)
/// let service: Arc<dyn EchoService> = factory.new_service_gateway(Protocol::Auto).await?;
///
/// // Use the service (protocol-agnostic!)
/// let response = service.echo("Hello".to_string()).await?;
/// ```
pub struct ServiceGatewayFactory<C: ?Sized + Send + Sync> {
    /// Target module ID.
    module_id: ModuleID,
    
    /// Target service ID.
    service_id: ServiceID,
    
    /// Service connector for protocol selection and connection.
    service_connector: Arc<dyn ServiceConnector>,
    
    /// Protocol-specific factory functions.
    factory_funcs: GatewayFactoryFuncs<C>,
}

/// Protocol-specific factory functions.
///
/// Each function creates a gateway for a specific protocol.
///
/// # Rust Learning Note
///
/// ## Function Types vs. Closures
///
/// ```rust
/// pub struct GatewayFactoryFuncs<C: ?Sized + Send + Sync> {
///     pub direct: Option<Box<dyn Fn() -> Result<Arc<C>> + Send + Sync>>,
///     //                       ^^^
///     //                       This is a TRAIT OBJECT for closures
/// }
/// ```
///
/// **Why `Box<dyn Fn(...)>`?**
/// - Different closures have different types (even with same signature!)
/// - We need to store them in a struct (fixed size)
/// - `Box<dyn Fn(...)>` erases the type, making them all the same size
///
/// **Comparison with Go:**
/// ```go
/// type GatewayFactoryFuncs[Contract any] struct {
///     Direct func() (Contract, error)
///     GRPC   func(*grpc.ClientConn) (Contract, error)
///     HTTP   func(http.Client) (Contract, error)
/// }
/// ```
///
/// In Go, all functions with the same signature are the same type.
/// In Rust, each closure is a unique type, so we need trait objects.
pub struct GatewayFactoryFuncs<C: ?Sized + Send + Sync> {
    /// Factory for direct (in-process) gateways.
    ///
    /// Called when using Protocol::Direct or Protocol::Auto (if available locally).
    pub direct: Option<Box<dyn Fn() -> Result<Arc<C>> + Send + Sync>>,
    
    /// Factory for gRPC gateways.
    ///
    /// Receives a tonic Channel and creates a gRPC client gateway.
    pub grpc: Option<Box<dyn Fn(tonic::transport::Channel) -> Result<Arc<C>> + Send + Sync>>,
    
    /// Factory for HTTP gateways.
    ///
    /// Not yet implemented - placeholder for future HTTP support.
    pub http: Option<Box<dyn Fn() -> Result<Arc<C>> + Send + Sync>>,  // TODO: HTTP client type
}

impl<C: ?Sized + Send + Sync + 'static> ServiceGatewayFactory<C> {
    /// Creates a new generic service gateway factory.
    ///
    /// # Arguments
    ///
    /// * `module_id` - Target module ID
    /// * `service_id` - Target service ID
    /// * `service_connector` - Connector for protocol selection
    /// * `factory_funcs` - Protocol-specific factory functions
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let factory = ServiceGatewayFactory::<dyn EchoService>::new(
    ///     ModuleID::from("echo"),
    ///     ServiceID::from("echo-service"),
    ///     connector,
    ///     GatewayFactoryFuncs {
    ///         direct: Some(Box::new(|| Ok(handler))),
    ///         grpc: Some(Box::new(|ch| Ok(Arc::new(GrpcGateway::new(ch))))),
    ///         http: None,
    ///     },
    /// );
    /// ```
    pub fn new(
        module_id: ModuleID,
        service_id: ServiceID,
        service_connector: Arc<dyn ServiceConnector>,
        factory_funcs: GatewayFactoryFuncs<C>,
    ) -> Self {
        Self {
            module_id,
            service_id,
            service_connector,
            factory_funcs,
        }
    }
    
    /// Creates a new service gateway for the specified protocol.
    ///
    /// This is where the magic happens:
    /// 1. Creates a visitor with the factory functions
    /// 2. Calls service connector (protocol selection + connection)
    /// 3. Connector calls visitor with protocol-specific data
    /// 4. Visitor creates typed gateway
    /// 5. Returns typed gateway to caller
    ///
    /// **The caller never sees protocols or enums!**
    ///
    /// # Arguments
    ///
    /// * `protocol` - Desired protocol (Auto, Direct, gRPC, HTTP)
    ///
    /// # Returns
    ///
    /// Returns a typed gateway (`Arc<C>`) ready to use.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Service not found
    /// - Protocol not available
    /// - Connection failed
    /// - Factory function failed
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Get typed gateway
    /// let service: Arc<dyn EchoService> = factory
    ///     .new_service_gateway(Protocol::Auto)
    ///     .await?;
    ///
    /// // Use it (protocol-agnostic!)
    /// let response = service.echo("Hello".to_string()).await?;
    /// ```
    pub async fn new_service_gateway(&self, protocol: Protocol) -> Result<Arc<C>> {
        debug!(
            "Creating gateway for {}/{} with protocol {:?}",
            self.module_id, self.service_id, protocol
        );
        
        // Create visitor with our factory functions
        let mut visitor = GatewayFactoryVisitor::new(&self.factory_funcs);
        
        // Connect using service connector (protocol selection + visitor dispatch)
        self.service_connector
            .connect(&self.module_id, &self.service_id, protocol, &mut visitor)
            .await?;
        
        // Extract typed gateway from visitor
        let gateway = visitor.gateway()?;
        
        debug!("✅ Gateway created successfully for {}/{}", self.module_id, self.service_id);
        Ok(gateway)
    }
}

/// Visitor that creates typed gateways.
///
/// This visitor implements the `ClientConnectionVisitor` trait and uses
/// the protocol-specific factory functions to create typed gateways.
///
/// # Rust Learning Note
///
/// ## Interior Mutability with RwLock
///
/// ```rust
/// struct GatewayFactoryVisitor<'a, C: ?Sized + Send + Sync> {
///     factory_funcs: &'a GatewayFactoryFuncs<C>,
///     gateway: Arc<RwLock<Option<Arc<C>>>>,  // ← Interior mutability!
/// }
/// ```
///
/// **Why RwLock<Option<Arc<C>>>?**
///
/// The visitor trait methods take `&mut self`, but we want to:
/// 1. Store the result inside the visitor
/// 2. Extract it later with `gateway()`
///
/// **Option 1: Just use &mut self**
/// ```rust
/// struct Visitor<'a, C> {
///     gateway: Option<Arc<C>>,  // ← Simple!
/// }
/// ```
/// Problem: Can't easily share visitor across async boundaries.
///
/// **Option 2: Use RwLock (our choice)**
/// ```rust
/// struct Visitor<'a, C> {
///     gateway: Arc<RwLock<Option<Arc<C>>>>,  // ← Thread-safe sharing!
/// }
/// ```
/// Benefit: Can be used across async/await points safely.
///
/// **Why Arc<RwLock<...>> instead of just RwLock<...>?**
/// - We create the visitor in `new_service_gateway()`
/// - We pass it to `service_connector.connect()` (may be async)
/// - We extract result after async call
/// - Arc ensures the RwLock lives long enough
struct GatewayFactoryVisitor<'a, C: ?Sized + Send + Sync> {
    /// Reference to factory functions (borrowed from parent).
    factory_funcs: &'a GatewayFactoryFuncs<C>,
    
    /// The created gateway (interior mutability).
    gateway: Arc<RwLock<Option<Arc<C>>>>,
}

impl<'a, C: ?Sized + Send + Sync> GatewayFactoryVisitor<'a, C> {
    /// Creates a new visitor.
    fn new(factory_funcs: &'a GatewayFactoryFuncs<C>) -> Self {
        Self {
            factory_funcs,
            gateway: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Extracts the created gateway.
    ///
    /// # Errors
    ///
    /// Returns error if no gateway was created (shouldn't happen if visitor methods succeeded).
    fn gateway(&self) -> Result<Arc<C>> {
        self.gateway
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or_else(|| Error::Internal(
                "Gateway not created by visitor".to_string(),
            ))
    }
}

#[async_trait]
impl<'a, C: ?Sized + Send + Sync + 'static> ClientConnectionVisitor for GatewayFactoryVisitor<'a, C> {
    async fn protocol_is_direct(&mut self) -> Result<()> {
        debug!("Visitor: Creating direct gateway");
        
        let factory = self.factory_funcs.direct.as_ref()
            .ok_or_else(|| Error::Validation {
                message: "Direct factory not provided".to_string(),
            })?;
        
        let gateway = factory()?;
        *self.gateway.write().unwrap() = Some(gateway);
        
        debug!("✅ Visitor: Direct gateway created");
        Ok(())
    }
    
    async fn protocol_is_grpc(&mut self, channel: tonic::transport::Channel) -> Result<()> {
        debug!("Visitor: Creating gRPC gateway");
        
        let factory = self.factory_funcs.grpc.as_ref()
            .ok_or_else(|| Error::Validation {
                message: "gRPC factory not provided".to_string(),
            })?;
        
        let gateway = factory(channel)?;
        *self.gateway.write().unwrap() = Some(gateway);
        
        debug!("✅ Visitor: gRPC gateway created");
        Ok(())
    }
    
    async fn protocol_is_http(&mut self, _client: ()) -> Result<()> {
        debug!("Visitor: HTTP protocol requested");
        
        let factory = self.factory_funcs.http.as_ref()
            .ok_or_else(|| Error::Validation {
                message: "HTTP factory not provided (HTTP not yet implemented)".to_string(),
            })?;
        
        let gateway = factory()?;
        *self.gateway.write().unwrap() = Some(gateway);
        
        debug!("✅ Visitor: HTTP gateway created");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Mock service trait for testing
    trait MockService: Send + Sync {
        fn call(&self) -> String;
    }

    struct MockServiceImpl {
        name: String,
    }

    impl MockService for MockServiceImpl {
        fn call(&self) -> String {
            self.name.clone()
        }
    }

    #[test]
    fn test_direct_factory_func() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let call_count_clone = call_count.clone();
        
        let factory_func: Box<dyn Fn() -> Result<Arc<dyn MockService>> + Send + Sync> = 
            Box::new(move || {
                call_count_clone.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(MockServiceImpl {
                    name: "direct".to_string(),
                }) as Arc<dyn MockService>)
            });
        
        // Call the factory
        let service = factory_func().unwrap();
        assert_eq!(service.call(), "direct");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_visitor_pattern() {
        let factory_funcs = GatewayFactoryFuncs::<dyn MockService> {
            direct: Some(Box::new(|| {
                Ok(Arc::new(MockServiceImpl {
                    name: "direct".to_string(),
                }) as Arc<dyn MockService>)
            })),
            grpc: None,
            http: None,
        };
        
        let mut visitor = GatewayFactoryVisitor::new(&factory_funcs);
        
        // Simulate protocol_is_direct being called
        visitor.protocol_is_direct().await.unwrap();
        
        // Extract gateway
        let gateway = visitor.gateway().unwrap();
        assert_eq!(gateway.call(), "direct");
    }
}

