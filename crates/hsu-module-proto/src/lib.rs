//! # HSU Module Proto
//! 
//! Protocol abstraction layer for inter-module communication.
//! 
//! This crate provides protocol implementations for different communication methods:
//! - **Direct**: In-process, zero-cost function calls
//! - **gRPC**: Cross-process communication using tonic
//! - **HTTP**: Cross-process communication using HTTP (future)

pub mod traits;
pub mod direct;
pub mod grpc;
pub mod options;
pub mod server;
pub mod grpc_server;
pub mod server_manager;
pub mod client_connection;

// Re-export commonly used items
pub use traits::{ProtocolHandler, ProtocolGateway};
pub use direct::DirectProtocol;
pub use options::{ProtocolOptions, GrpcOptions};
pub use server::{ProtocolServer, ProtocolServerHandlersVisitor};
pub use grpc_server::{GrpcProtocolServer, GrpcServerOptions};
pub use server_manager::{ServerManager, ServerID};
pub use client_connection::{ClientConnection, ClientConnectionVisitor};
pub use grpc::{GrpcProtocolGateway, GrpcProtocolHandler};

use hsu_common::{Protocol, Result, Error};
use std::sync::Arc;

/// Factory function to create protocol servers from config.
///
/// # Arguments
///
/// * `protocol` - The protocol type (Grpc, Http, etc.)
/// * `listen_address` - The address to listen on (e.g., "0.0.0.0:50051")
///
/// # Returns
///
/// * `Result<Arc<dyn ProtocolServer>>` - The created protocol server
///
/// # Errors
///
/// Returns an error if:
/// - The protocol is not supported for servers (Direct, Auto)
/// - The listen address format is invalid
/// - The protocol implementation is not available (Http)
///
/// # Example
///
/// ```rust
/// use hsu_common::Protocol;
/// use hsu_module_proto::new_protocol_server;
///
/// let server = new_protocol_server(
///     Protocol::Grpc,
///     "0.0.0.0:50051".to_string(),
/// )?;
/// ```
pub fn new_protocol_server(
    protocol: Protocol,
    listen_address: String,
) -> Result<Arc<dyn ProtocolServer>> {
    match protocol {
        Protocol::Grpc => {
            // Parse listen_address to get host and port
            let (host, port) = parse_listen_address(&listen_address)?;
            
            // Create gRPC server with options
            let server = GrpcProtocolServer::new(
                GrpcServerOptions::new()
                    .with_host(host)
                    .with_port(port)
            );
            
            Ok(Arc::new(server))
        }
        Protocol::Http => {
            Err(Error::Internal("HTTP server not implemented yet".to_string()))
        }
        Protocol::Direct | Protocol::Auto => {
            Err(Error::Internal(format!(
                "Cannot create server for protocol: {:?}. Only Grpc and Http are valid for servers.",
                protocol
            )))
        }
    }
}

/// Parses a listen address into host and port.
///
/// # Arguments
///
/// * `listen_address` - Address in format "host:port" (e.g., "0.0.0.0:50051")
///
/// # Returns
///
/// * `Result<(String, u16)>` - Tuple of (host, port)
///
/// # Errors
///
/// Returns an error if:
/// - The address doesn't contain a colon
/// - The port part is not a valid number
///
/// # Examples
///
/// ```rust
/// assert_eq!(parse_listen_address("0.0.0.0:50051")?, ("0.0.0.0".to_string(), 50051));
/// assert_eq!(parse_listen_address("localhost:8080")?, ("localhost".to_string(), 8080));
/// ```
fn parse_listen_address(listen_address: &str) -> Result<(String, u16)> {
    // Split by last colon to handle IPv6 addresses like "[::1]:8080"
    let parts: Vec<&str> = listen_address.rsplitn(2, ':').collect();
    
    if parts.len() != 2 {
        return Err(Error::Internal(format!(
            "Invalid listen address format: '{}'. Expected 'host:port' (e.g., '0.0.0.0:50051')",
            listen_address
        )));
    }
    
    let port_str = parts[0];
    let host = parts[1];
    
    let port = port_str.parse::<u16>().map_err(|e| {
        Error::Internal(format!(
            "Invalid port '{}' in listen address '{}': {}",
            port_str, listen_address, e
        ))
    })?;
    
    Ok((host.to_string(), port))
}
