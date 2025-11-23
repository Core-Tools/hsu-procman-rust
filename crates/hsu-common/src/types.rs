//! Core domain types used throughout the HSU framework.
//! 
//! These types mirror the Go implementation but use Rust idioms for
//! type safety and zero-cost abstractions.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Module identifier - uniquely identifies a module in the system.
/// 
/// # Example
/// ```
/// use hsu_common::ModuleID;
/// 
/// let module_id = ModuleID::from("echo");
/// assert_eq!(module_id.as_str(), "echo");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleID(String);

impl ModuleID {
    /// Creates a new ModuleID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the module ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModuleID {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ModuleID {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for ModuleID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Service identifier - identifies a specific service within a module.
/// 
/// A module can provide multiple services. Each service has a unique ID
/// within that module.
/// 
/// # Example
/// ```
/// use hsu_common::ServiceID;
/// 
/// let service_id = ServiceID::from("service1");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServiceID(String);

impl ServiceID {
    /// Creates a new ServiceID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the service ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ServiceID {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ServiceID {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl fmt::Display for ServiceID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Communication protocol used for inter-module communication.
/// 
/// # Rust Learning Note
/// 
/// This is an **enum** - Rust's way of defining a type that can be one of
/// several variants. Unlike Go's string constants, this is type-safe at
/// compile time.
/// 
/// ## Go Comparison
/// ```go
/// // Go version (just a string)
/// type Protocol string
/// const (
///     ProtocolDirect Protocol = ""
///     ProtocolAuto   Protocol = "auto"
///     ProtocolGRPC   Protocol = "grpc"
/// )
/// ```
/// 
/// ## Rust Advantages
/// - Compile-time checking: Can't create invalid protocols
/// - Pattern matching: Compiler ensures all cases are handled
/// - No string comparisons: Direct integer comparison at runtime
/// 
/// # Example
/// ```
/// use hsu_common::Protocol;
/// 
/// let protocol = Protocol::Grpc;
/// 
/// match protocol {
///     Protocol::Direct => println!("In-process"),
///     Protocol::Grpc => println!("gRPC"),
///     Protocol::Http => println!("HTTP"),
///     Protocol::Auto => println!("Auto-select"),
/// }
/// // Compiler ensures all cases are covered!
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    /// Direct in-process communication (zero overhead)
    Direct,
    
    /// Automatic protocol selection (direct if available, otherwise remote)
    Auto,
    
    /// gRPC protocol for cross-process communication
    Grpc,
    
    /// HTTP protocol for cross-process communication
    Http,
}

impl Protocol {
    /// Returns true if this is a remote protocol (not direct).
    pub fn is_remote(&self) -> bool {
        matches!(self, Protocol::Grpc | Protocol::Http)
    }

    /// Returns the protocol name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Direct => "direct",
            Protocol::Auto => "auto",
            Protocol::Grpc => "grpc",
            Protocol::Http => "http",
        }
    }
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::Auto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_id() {
        let id = ModuleID::from("test-module");
        assert_eq!(id.as_str(), "test-module");
        assert_eq!(id.to_string(), "test-module");
    }

    #[test]
    fn test_service_id() {
        let id = ServiceID::from("service1");
        assert_eq!(id.as_str(), "service1");
    }

    #[test]
    fn test_protocol() {
        assert_eq!(Protocol::Direct.as_str(), "direct");
        assert_eq!(Protocol::Grpc.as_str(), "grpc");
        assert!(Protocol::Grpc.is_remote());
        assert!(!Protocol::Direct.is_remote());
    }

    #[test]
    fn test_protocol_pattern_matching() {
        // This test demonstrates exhaustive pattern matching
        let protocol = Protocol::Auto;
        
        let result = match protocol {
            Protocol::Direct => "direct",
            Protocol::Auto => "auto",
            Protocol::Grpc => "grpc",
            Protocol::Http => "http",
            // If we forget a variant, this won't compile!
        };
        
        assert_eq!(result, "auto");
    }
}

