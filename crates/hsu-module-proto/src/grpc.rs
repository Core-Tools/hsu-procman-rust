//! gRPC protocol implementation.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **gRPC in Rust** using the `tonic` crate.
//! 
//! ## tonic vs Go's grpc
//! 
//! **Go:**
//! ```go
//! conn, err := grpc.Dial(address, grpc.WithInsecure())
//! client := pb.NewServiceClient(conn)
//! response, err := client.Method(ctx, request)
//! ```
//! 
//! **Rust with tonic:**
//! ```rust
//! let channel = Channel::from_static(address).connect().await?;
//! let mut client = ServiceClient::new(channel);
//! let response = client.method(request).await?;
//! ```
//! 
//! ## Key Differences
//! 
//! 1. **Type Safety**: tonic generates strongly-typed clients
//! 2. **Async**: Built on tokio (native async/await)
//! 3. **Zero-copy**: Uses `Bytes` for efficient memory handling
//! 4. **Streaming**: First-class support for bidirectional streaming

use async_trait::async_trait;
use hsu_common::{Result, ServiceID, ModuleID, Error};
use tracing::{trace, debug, info, error};

use crate::traits::{ProtocolHandler, ProtocolGateway};
use crate::options::GrpcOptions;
use crate::client_connection::{ClientConnection, ClientConnectionVisitor};

/// gRPC protocol handler (server-side).
/// 
/// # Rust Learning Note
/// 
/// ## Server-Side gRPC
/// 
/// In a full implementation, this would:
/// 1. Start a tonic gRPC server
/// 2. Register service implementations
/// 3. Listen on a port
/// 
/// ```rust
/// // Full tonic server example:
/// Server::builder()
///     .add_service(EchoServiceServer::new(handler))
///     .serve(addr)
///     .await?;
/// ```
/// 
/// For now, this is a placeholder showing the structure.
pub struct GrpcProtocolHandler {
    /// Server address (e.g., "0.0.0.0:50051").
    address: String,
    
    /// gRPC options.
    #[allow(dead_code)]
    options: GrpcOptions,
}

impl GrpcProtocolHandler {
    /// Creates a new gRPC protocol handler.
    pub fn new(address: impl Into<String>, options: GrpcOptions) -> Self {
        Self {
            address: address.into(),
            options,
        }
    }
}

#[async_trait]
impl ProtocolHandler for GrpcProtocolHandler {
    /// Handles a gRPC service call.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## gRPC Request Flow
    /// 
    /// 1. Receive protobuf bytes
    /// 2. Deserialize to Rust struct (using prost)
    /// 3. Call actual service method
    /// 4. Serialize response back to protobuf
    /// 5. Send over network
    /// 
    /// ```
    /// Client          Network         Server
    ///   |                |              |
    ///   |-- Request ---->|              |
    ///   |                |-- Bytes ---->|
    ///   |                |              |- Deserialize
    ///   |                |              |- Call method
    ///   |                |              |- Serialize
    ///   |                |<-- Bytes ----|
    ///   |<-- Response ---|              |
    /// ```
    async fn handle(
        &self,
        service_id: &ServiceID,
        method: &str,
        _request: Vec<u8>,
    ) -> Result<Vec<u8>> {
        debug!(
            "gRPC handler: service={}, method={}, address={}",
            service_id, method, self.address
        );

        // TODO: Phase 5 will implement actual gRPC server
        // For now, return error
        Err(Error::Protocol(
            "gRPC handler not yet fully implemented".to_string(),
        ))
    }
}

/// gRPC protocol gateway (client-side).
/// 
/// # Rust Learning Note
/// 
/// ## Client-Side gRPC
/// 
/// The gateway maintains a connection to a remote gRPC server.
/// 
/// ```rust
/// // Full tonic client example:
/// let channel = Channel::from_shared(address)?
///     .timeout(Duration::from_millis(timeout))
///     .connect()
///     .await?;
/// 
/// let mut client = EchoServiceClient::new(channel);
/// let response = client.echo(request).await?;
/// ```
/// 
/// ## Connection Pooling
/// 
/// tonic automatically handles:
/// - Connection pooling
/// - Load balancing
/// - Reconnection
/// - Keep-alive
/// 
/// **All for free!**
#[derive(Debug)]
pub struct GrpcProtocolGateway {
    /// Target server address.
    address: String,
    
    /// gRPC options.
    #[allow(dead_code)]
    options: GrpcOptions,
    
    /// Tonic channel for gRPC communication.
    /// 
    /// # Rust Learning Note
    /// 
    /// tonic::transport::Channel is:
    /// - **Cheap to clone** (uses Arc internally)
    /// - **Connection pooling** built-in
    /// - **Lazy connection** (connects on first use)
    /// - **Thread-safe** (Send + Sync)
    /// 
    /// Note: Currently unused because we recommend service-specific gateways,
    /// but available for future generic protocol features.
    #[allow(dead_code)]
    channel: Option<tonic::transport::Channel>,
}

impl GrpcProtocolGateway {
    /// Creates a new gRPC protocol gateway.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Async Constructor Pattern
    /// 
    /// In Rust, constructors (`new`) can't be async.
    /// So we have two patterns:
    /// 
    /// 1. **Sync new + lazy connect:**
    /// ```rust
    /// let gateway = GrpcGateway::new(address);  // Doesn't connect yet
    /// gateway.call(...).await?;  // Connects on first call
    /// ```
    /// 
    /// 2. **Async connect method:**
    /// ```rust
    /// let gateway = GrpcGateway::connect(address).await?;  // Connects immediately
    /// gateway.call(...).await?;
    /// ```
    /// 
    /// We use pattern 1 (lazy connection).
    pub fn new(address: impl Into<String>, options: GrpcOptions) -> Self {
        Self {
            address: address.into(),
            options,
            channel: None,
        }
    }

    /// Connect immediately and create a channel.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## async fn vs fn -> Future
    /// 
    /// These are equivalent:
    /// ```rust
    /// // Style 1: async fn
    /// pub async fn connect(address: String) -> Result<Self> {
    ///     // async code
    /// }
    /// 
    /// // Style 2: fn returning Future
    /// pub fn connect(address: String) -> impl Future<Output = Result<Self>> {
    ///     async move {
    ///         // async code
    ///     }
    /// }
    /// ```
    /// 
    /// Style 1 is more common and readable.
    /// 
    /// ## Tonic Channel Connection
    /// 
    /// tonic channels are "lazy" by default - they don't actually connect
    /// until first use. The `connect()` call here just validates the address
    /// and sets up the channel configuration.
    pub async fn connect(address: impl Into<String>, options: GrpcOptions) -> Result<Self> {
        let address = address.into();
        
        info!("🔌 Attempting to create gRPC channel for: {}", address);
        debug!("   Timeout: {} ms", options.timeout_ms.unwrap_or(5000));
        
        // Create tonic channel with configuration
        debug!("   Step 1: Parsing address...");
        let endpoint = tonic::transport::Channel::from_shared(address.clone())
            .map_err(|e| {
                error!("❌ Failed to parse gRPC address '{}': {:?}", address, e);
                Error::Protocol(format!("Invalid gRPC address '{}': {}", address, e))
            })?;
        
        debug!("   Step 2: Setting timeout...");
        let endpoint = endpoint.timeout(std::time::Duration::from_millis(
            options.timeout_ms.unwrap_or(5000)
        ));
        
        debug!("   Step 3: Connecting to {}...", address);
        let channel = endpoint.connect()
            .await
            .map_err(|e| {
                error!("❌ Failed to connect to '{}': {:?}", address, e);
                error!("   Error details: {}", e);
                Error::Protocol(format!("Failed to connect to '{}': transport error", address))
            })?;
        
        info!("✅ gRPC channel created for: {}", address);
        
        Ok(Self {
            address,
            options,
            channel: Some(channel),
        })
    }
    
    /// Gets the tonic channel, creating it lazily if needed.
    /// 
    /// # Rust Learning Note
    /// 
    /// This is the "lazy initialization" pattern. If the channel wasn't
    /// created via `connect()`, we create it on first access.
    /// 
    /// Note: Available for future use if generic protocol layer features are needed.
    #[allow(dead_code)]
    async fn get_or_create_channel(&mut self) -> Result<tonic::transport::Channel> {
        if let Some(channel) = &self.channel {
            return Ok(channel.clone());
        }
        
        trace!("Lazy-creating gRPC channel for: {}", self.address);
        
        let channel = tonic::transport::Channel::from_shared(self.address.clone())
            .map_err(|e| Error::Protocol(format!("Invalid gRPC address '{}': {}", self.address, e)))?
            .timeout(std::time::Duration::from_millis(
                self.options.timeout_ms.unwrap_or(5000)
            ))
            .connect()
            .await
            .map_err(|e| Error::Protocol(format!("Failed to connect to '{}': {}", self.address, e)))?;
        
        self.channel = Some(channel.clone());
        Ok(channel)
    }
}

#[async_trait]
impl ProtocolGateway for GrpcProtocolGateway {
    /// Generic gRPC calls are not supported by design.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Why This Method Returns an Error
    /// 
    /// gRPC requires **service-specific generated code** from .proto files:
    /// 
    /// ```rust
    /// // Generated by tonic from .proto:
    /// pub struct EchoServiceClient<T> { ... }
    /// 
    /// impl EchoServiceClient<Channel> {
    ///     pub async fn echo(&mut self, request: EchoRequest) -> Result<EchoResponse> { ... }
    /// }
    /// ```
    /// 
    /// **This generic protocol layer cannot:**
    /// - Know about specific .proto definitions
    /// - Import generated service clients
    /// - Call service-specific methods
    /// 
    /// **The correct approach:**
    /// Use the `ProtocolGatewayFactory` trait to create service-specific gateways!
    /// 
    /// ## Example: Service-Specific Gateway
    /// 
    /// ```rust,ignore
    /// // Your service-specific gateway:
    /// pub struct EchoGrpcGateway {
    ///     client: EchoServiceClient<Channel>,  // ← Generated client!
    /// }
    /// 
    /// impl EchoGrpcGateway {
    ///     pub async fn connect(address: String) -> Result<Self> {
    ///         let channel = Channel::from_shared(address)?.connect().await?;
    ///         let client = EchoServiceClient::new(channel);  // ← Service-specific!
    ///         Ok(Self { client })
    ///     }
    /// }
    /// 
    /// // Your factory:
    /// impl ProtocolGatewayFactory for EchoGrpcGatewayFactory {
    ///     async fn create_gateway(&self, address: String) -> Result<ServiceGateway> {
    ///         let gateway = EchoGrpcGateway::connect(address).await?;  // ← Creates real gateway!
    ///         Ok(ServiceGateway::Grpc(GrpcGateway::new(gateway)))  // ← Type-erased!
    ///     }
    /// }
    /// ```
    /// 
    /// **This is by design!** Service-specific gateways provide:
    /// - Type safety (compile-time checking)
    /// - Generated code efficiency
    /// - Protocol-specific features
    /// - Clear domain boundaries
    /// 
    /// ## Performance Note
    /// 
    /// When you DO make gRPC calls through service-specific gateways:
    /// ```
    /// 1. Serialize request (prost):     ~1-5μs
    /// 2. Network round-trip:            ~0.1-10ms (local/remote)
    /// 3. Deserialize request (server):  ~1-5μs
    /// 4. Execute method:                varies
    /// 5. Serialize response:            ~1-5μs
    /// 6. Network return:                ~0.1-10ms
    /// 7. Deserialize response:          ~1-5μs
    /// 
    /// Total: ~0.2-20ms (mostly network)
    /// ```
    /// 
    /// Compare to Direct protocol: ~0.006ms (6μs)
    /// **Direct is ~3,000x faster, but gRPC works across processes/machines!**
    async fn call(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        method: &str,
        _request: Vec<u8>,
    ) -> Result<Vec<u8>> {
        trace!(
            "gRPC call attempted: module={}, service={}, method={}, address={}",
            module_id,
            service_id,
            method,
            self.address
        );

        // This generic protocol layer intentionally does not support direct calls
        Err(Error::Protocol(
            format!(
                "Generic gRPC calls not supported. Use ProtocolGatewayFactory to create \
                 service-specific gateways for module '{}', service '{}'",
                module_id, service_id
            )
        ))
    }
}

/// Implementation of ClientConnection for GrpcProtocolGateway.
///
/// This enables the visitor pattern for gRPC connections, allowing
/// protocol-agnostic code to interact with gRPC channels through the visitor interface.
///
/// # Rust Learning Note
///
/// ## Visitor Pattern in Rust
///
/// The visitor pattern enables "double dispatch" - a way to achieve type-safe
/// polymorphism when dealing with multiple types:
///
/// ```text
/// 1. Caller creates a visitor
/// 2. Caller calls connection.accept_visitor(visitor)
/// 3. Connection (knows it's gRPC) calls visitor.protocol_is_grpc(channel)
/// 4. Visitor stores the typed result
/// 5. Caller extracts the result
/// ```
///
/// This is similar to Go's approach, but Rust's async traits require the `async_trait` macro.
#[async_trait]
impl ClientConnection for GrpcProtocolGateway {
    async fn accept_visitor(&self, visitor: &mut dyn ClientConnectionVisitor) -> Result<()> {
        let channel = self.channel.as_ref()
            .ok_or_else(|| Error::Internal(
                "gRPC channel not initialized".to_string(),
            ))?;
        
        debug!("Accepting visitor for gRPC connection: {}", self.address);
        
        // Double dispatch: call the gRPC-specific visitor method
        visitor.protocol_is_grpc(channel.clone()).await
    }
}

// Placeholder for future gRPC service trait
// This would be generated by tonic from .proto files

/// Example of what a generated gRPC service would look like.
#[allow(dead_code)]
/// 
/// # Rust Learning Note
/// 
/// ## Code Generation with tonic
/// 
/// tonic uses `prost-build` to generate Rust code from .proto files:
/// 
/// ```rust
/// // build.rs
/// fn main() {
///     tonic_build::compile_protos("proto/echo.proto")?;
/// }
/// ```
/// 
/// This generates:
/// ```rust
/// // Generated code
/// pub mod echo {
///     #[derive(Clone, PartialEq, prost::Message)]
///     pub struct EchoRequest {
///         #[prost(string, tag = "1")]
///         pub message: String,
///     }
///     
///     #[derive(Clone, PartialEq, prost::Message)]
///     pub struct EchoResponse {
///         #[prost(string, tag = "1")]
///         pub message: String,
///     }
///     
///     #[async_trait]
///     pub trait Echo: Send + Sync + 'static {
///         async fn echo(
///             &self,
///             request: Request<EchoRequest>,
///         ) -> Result<Response<EchoResponse>, Status>;
///     }
/// }
/// ```
/// 
/// **All type-safe, zero-cost!**
#[cfg(test)]
mod _example_generated_code {
    // This is what tonic would generate:
    #[derive(Clone, PartialEq)]
    pub struct EchoRequest {
        pub message: String,
    }

    #[derive(Clone, PartialEq)]
    pub struct EchoResponse {
        pub message: String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_handler_creation() {
        let options = GrpcOptions::new("localhost:50051");
        let handler = GrpcProtocolHandler::new("0.0.0.0:50051", options);
        
        assert_eq!(handler.address, "0.0.0.0:50051");
    }

    #[test]
    fn test_grpc_gateway_creation() {
        let options = GrpcOptions::new("localhost:50051");
        let gateway = GrpcProtocolGateway::new("localhost:50051", options);
        
        assert_eq!(gateway.address, "localhost:50051");
    }

    #[tokio::test]
    async fn test_grpc_gateway_connect() {
        let options = GrpcOptions::new("http://localhost:50051");
        let result = GrpcProtocolGateway::connect("http://localhost:50051", options).await;
        
        // Should fail to connect to non-existent server
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to connect"));
    }

    #[tokio::test]
    async fn test_grpc_not_implemented() {
        let options = GrpcOptions::new("localhost:50051");
        let handler = GrpcProtocolHandler::new("0.0.0.0:50051", options);
        
        let result = handler
            .handle(&ServiceID::from("test"), "method", vec![])
            .await;
        
        // Should return error (not implemented yet)
        assert!(result.is_err());
    }
}

