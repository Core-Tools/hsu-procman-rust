//! Core protocol traits.
//! 
//! # Rust Learning Note
//! 
//! This module defines **trait-based abstractions** for protocols.
//! 
//! ## Traits vs Go Interfaces
//! 
//! **Go:**
//! ```go
//! type Handler interface {
//!     Handle(ctx context.Context, req interface{}) (interface{}, error)
//! }
//! ```
//! 
//! **Rust:**
//! ```rust
//! #[async_trait]
//! trait ProtocolHandler: Send + Sync {
//!     async fn handle(&self, service_id: &ServiceID, method: &str, request: Vec<u8>) 
//!         -> Result<Vec<u8>>;
//! }
//! ```
//! 
//! ## Key Differences
//! 
//! 1. **Type Safety**: Rust uses `Vec<u8>` (bytes), not `interface{}`
//! 2. **Thread Safety**: `Send + Sync` explicitly required
//! 3. **Async**: Built-in async support with `async_trait`
//! 4. **No nil**: Result<T> instead of (T, error)

use async_trait::async_trait;
use hsu_common::{Result, ServiceID, ModuleID};

/// Protocol handler trait - server-side.
/// 
/// # Rust Learning Note
/// 
/// This is a **trait object boundary** - we'll use `dyn ProtocolHandler`.
/// 
/// ## Dynamic Dispatch
/// 
/// ```rust
/// let handler: Box<dyn ProtocolHandler> = Box::new(MyHandler);
/// handler.handle(...).await;  // ← Virtual function call
/// ```
/// 
/// **Cost:** One pointer indirection (~1-2 CPU cycles)
/// **Benefit:** Protocol abstraction
/// 
/// ## Why async_trait?
/// 
/// Rust doesn't support `async fn` in traits yet (stable).
/// The `async_trait` macro works around this by boxing the Future.
/// 
/// **Small cost:** Heap allocation for Future (~16 bytes)
/// **Big benefit:** Can use async methods in traits
#[async_trait]
pub trait ProtocolHandler: Send + Sync {
    /// Handles a service method call.
    /// 
    /// # Arguments
    /// 
    /// * `service_id` - The service being called
    /// * `method` - The method name
    /// * `request` - Serialized request bytes
    /// 
    /// # Returns
    /// 
    /// Serialized response bytes or error.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Why Vec<u8>?
    /// 
    /// We use raw bytes instead of generic types for flexibility:
    /// - Different protocols serialize differently (JSON, Protobuf, etc.)
    /// - Allows protocol-agnostic abstraction
    /// - Actual types handled by higher layers
    /// 
    /// **In Go:** Would use `interface{}` (runtime type checking)
    /// **In Rust:** Use bytes + protocol-specific encoding
    async fn handle(
        &self,
        service_id: &ServiceID,
        method: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>>;
}

/// Protocol gateway trait - client-side.
/// 
/// # Rust Learning Note
/// 
/// This is the **client-side counterpart** to ProtocolHandler.
/// 
/// ## Symmetry
/// 
/// ```
/// Client Side          Server Side
///     ↓                     ↓
/// ProtocolGateway  →  ProtocolHandler
///     call()       →      handle()
/// ```
/// 
/// Both use `Vec<u8>` for protocol independence.
#[async_trait]
pub trait ProtocolGateway: Send + Sync {
    /// Calls a remote service method.
    /// 
    /// # Arguments
    /// 
    /// * `module_id` - The target module
    /// * `service_id` - The target service
    /// * `method` - The method name
    /// * `request` - Serialized request bytes
    /// 
    /// # Returns
    /// 
    /// Serialized response bytes or error.
    async fn call(
        &self,
        module_id: &ModuleID,
        service_id: &ServiceID,
        method: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock handler for testing
    struct MockHandler;

    #[async_trait]
    impl ProtocolHandler for MockHandler {
        async fn handle(
            &self,
            _service_id: &ServiceID,
            _method: &str,
            request: Vec<u8>,
        ) -> Result<Vec<u8>> {
            // Echo the request
            Ok(request)
        }
    }

    // Mock gateway for testing
    struct MockGateway;

    #[async_trait]
    impl ProtocolGateway for MockGateway {
        async fn call(
            &self,
            _module_id: &ModuleID,
            _service_id: &ServiceID,
            _method: &str,
            request: Vec<u8>,
        ) -> Result<Vec<u8>> {
            // Echo the request
            Ok(request)
        }
    }

    #[tokio::test]
    async fn test_handler_trait() {
        let handler = MockHandler;
        let result = handler
            .handle(&ServiceID::from("test"), "method", vec![1, 2, 3])
            .await
            .unwrap();
        
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_gateway_trait() {
        let gateway = MockGateway;
        let result = gateway
            .call(
                &ModuleID::from("module"),
                &ServiceID::from("service"),
                "method",
                vec![1, 2, 3],
            )
            .await
            .unwrap();
        
        assert_eq!(result, vec![1, 2, 3]);
    }
}

