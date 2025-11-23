//! Protocol-specific options.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **type-safe options** using enums.
//! 
//! ## Go vs Rust Approach
//! 
//! **Go Version:**
//! ```go
//! type ProtocolOptions interface{}  // Can be anything!
//! 
//! // User must cast:
//! if grpcOpts, ok := opts.(*GrpcOptions); ok {
//!     // use grpcOpts
//! }
//! ```
//! 
//! **Rust Version:**
//! ```rust
//! enum ProtocolOptions {
//!     Direct,
//!     Grpc(GrpcOptions),
//!     Http(HttpOptions),
//! }
//! 
//! // No casting needed:
//! match options {
//!     ProtocolOptions::Grpc(grpc_opts) => {
//!         // grpc_opts is the right type!
//!     }
//!     _ => {}
//! }
//! ```
//! 
//! **Advantages:**
//! - ✅ **No runtime casts**: Pattern matching reveals type
//! - ✅ **Exhaustive checking**: Must handle all variants
//! - ✅ **Type-safe**: Wrong type = compile error

/// Protocol-specific options.
/// 
/// # Rust Learning Note
/// 
/// ## Enum with Associated Data
/// 
/// This enum demonstrates **sum types** - each variant can carry
/// different data:
/// 
/// ```rust
/// match options {
///     ProtocolOptions::Direct => {
///         // No options for direct
///     }
///     ProtocolOptions::Grpc(grpc_opts) => {
///         // grpc_opts: GrpcOptions
///         let timeout = grpc_opts.timeout;
///     }
/// }
/// ```
/// 
/// **Memory layout:**
/// ```
/// ProtocolOptions:
/// [discriminant: u8][largest variant data]
/// ```
/// 
/// Size = 1 byte + size of largest variant
#[derive(Debug, Clone)]
pub enum ProtocolOptions {
    /// Direct protocol (no options needed).
    Direct,
    
    /// gRPC protocol options.
    Grpc(GrpcOptions),
    
    /// HTTP protocol options (future).
    Http(HttpOptions),
}

impl Default for ProtocolOptions {
    fn default() -> Self {
        Self::Direct
    }
}

/// gRPC protocol options.
/// 
/// # Rust Learning Note
/// 
/// ## Struct with Optional Fields
/// 
/// ```rust
/// pub struct GrpcOptions {
///     pub address: String,           // Required
///     pub timeout_ms: Option<u64>,   // Optional
///     pub max_retries: Option<u32>,  // Optional
/// }
/// ```
/// 
/// **Option<T>** is Rust's way of handling optional values:
/// - `Some(value)`: Has a value
/// - `None`: No value
/// 
/// **No null pointers!**
#[derive(Debug, Clone)]
pub struct GrpcOptions {
    /// Server address (e.g., "localhost:50051").
    pub address: String,
    
    /// Request timeout in milliseconds.
    pub timeout_ms: Option<u64>,
    
    /// Maximum number of retry attempts.
    pub max_retries: Option<u32>,
    
    /// Enable TLS.
    pub use_tls: bool,
    
    /// Additional metadata (key-value pairs).
    pub metadata: Vec<(String, String)>,
}

impl GrpcOptions {
    /// Creates new gRPC options with an address.
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            timeout_ms: Some(5000), // Default 5 second timeout
            max_retries: Some(3),   // Default 3 retries
            use_tls: false,
            metadata: Vec::new(),
        }
    }

    /// Builder method for timeout.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Builder Pattern
    /// 
    /// ```rust
    /// let opts = GrpcOptions::new("localhost:50051")
    ///     .with_timeout(10000)
    ///     .with_retries(5)
    ///     .with_tls(true);
    /// ```
    /// 
    /// Each method returns `self`, allowing chaining.
    /// This is a common Rust pattern for configuration.
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Builder method for retries.
    pub fn with_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Builder method for TLS.
    pub fn with_tls(mut self, use_tls: bool) -> Self {
        self.use_tls = use_tls;
        self
    }

    /// Builder method for adding metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.push((key.into(), value.into()));
        self
    }
}

impl Default for GrpcOptions {
    fn default() -> Self {
        Self::new("localhost:0")
    }
}

/// HTTP protocol options (placeholder).
#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// Server base URL.
    pub base_url: String,
    
    /// Request timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

impl HttpOptions {
    /// Creates new HTTP options.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            timeout_ms: Some(5000),
        }
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self::new("http://localhost:8080")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_options_builder() {
        let opts = GrpcOptions::new("localhost:9000")
            .with_timeout(10000)
            .with_retries(5)
            .with_tls(true)
            .with_metadata("key1", "value1")
            .with_metadata("key2", "value2");

        assert_eq!(opts.address, "localhost:9000");
        assert_eq!(opts.timeout_ms, Some(10000));
        assert_eq!(opts.max_retries, Some(5));
        assert_eq!(opts.use_tls, true);
        assert_eq!(opts.metadata.len(), 2);
    }

    #[test]
    fn test_protocol_options_enum() {
        let direct = ProtocolOptions::Direct;
        let grpc = ProtocolOptions::Grpc(GrpcOptions::default());

        // Pattern matching
        match direct {
            ProtocolOptions::Direct => { /* OK */ }
            _ => panic!("Expected Direct"),
        }

        match grpc {
            ProtocolOptions::Grpc(_) => { /* OK */ }
            _ => panic!("Expected Grpc"),
        }
    }

    #[test]
    fn test_option_some_none() {
        let opts = GrpcOptions::new("test:123");
        
        // Option<T> pattern matching
        match opts.timeout_ms {
            Some(timeout) => assert_eq!(timeout, 5000),
            None => panic!("Expected Some"),
        }
        
        // Or use if let
        if let Some(timeout) = opts.timeout_ms {
            assert_eq!(timeout, 5000);
        }
    }
}

