// Client Connection Visitor Pattern
//
// This module implements the visitor pattern for protocol-specific client connections.
// The visitor pattern enables double dispatch: the connection knows which protocol it is,
// and calls the appropriate visitor method, passing protocol-specific data (like a gRPC Channel).
//
// This is analogous to Go's ClientConnectionVisitor interface.

use async_trait::async_trait;
use hsu_common::Result;

/// Visitor for protocol-specific client connections.
///
/// This trait is implemented by code that wants to handle protocol-specific
/// connection details. The visitor receives protocol-specific data (like a gRPC Channel)
/// and can store typed results.
///
/// # Example Flow
/// 1. Caller creates a visitor with desired behavior
/// 2. Caller calls `connection.accept_visitor(visitor)`
/// 3. Connection calls the appropriate `protocol_is_*` method
/// 4. Visitor extracts typed result
#[async_trait]
pub trait ClientConnectionVisitor: Send + Sync {
    /// Called when protocol is direct (local/in-process).
    /// 
    /// The visitor should handle local service calls, typically by
    /// accessing a service handler directly without any network communication.
    async fn protocol_is_direct(&mut self) -> Result<()>;
    
    /// Called when protocol is gRPC.
    /// 
    /// The visitor receives a tonic Channel that can be used to create
    /// gRPC client stubs.
    async fn protocol_is_grpc(&mut self, channel: tonic::transport::Channel) -> Result<()>;
    
    /// Called when protocol is HTTP.
    /// 
    /// The visitor receives an HTTP client that can be used for REST calls.
    /// TODO: Define HttpClient type when HTTP support is added.
    async fn protocol_is_http(&mut self, _client: ()) -> Result<()> {
        Err(hsu_common::Error::Validation {
            message: "HTTP protocol not yet implemented".to_string(),
        })
    }
}

/// Trait for client connections that can be visited.
///
/// Different protocol implementations (gRPC, HTTP, Direct) implement this trait
/// and call the appropriate visitor method with their protocol-specific data.
#[async_trait]
pub trait ClientConnection: Send + Sync {
    /// Accept a visitor and call the appropriate protocol-specific method.
    ///
    /// This is the "double dispatch" part of the visitor pattern:
    /// 1. The caller calls `connection.accept_visitor(visitor)`
    /// 2. The connection (which knows its protocol) calls `visitor.protocol_is_X(...)`
    /// 3. The visitor handles the protocol-specific logic
    async fn accept_visitor(&self, visitor: &mut dyn ClientConnectionVisitor) -> Result<()>;
}

