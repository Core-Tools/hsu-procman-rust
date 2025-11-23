//! Transport layer for the service registry.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **platform-specific code** in Rust.
//! 
//! ## Conditional Compilation
//! 
//! ```rust
//! #[cfg(unix)]
//! fn use_unix_socket() { ... }
//! 
//! #[cfg(windows)]
//! fn use_named_pipe() { ... }
//! ```
//! 
//! The `#[cfg(...)]` attribute controls what code gets compiled:
//! - `cfg(unix)`: Linux, macOS, BSD
//! - `cfg(windows)`: Windows
//! - `cfg(target_os = "linux")`: Only Linux
//! 
//! **Go equivalent:**
//! ```go
//! // +build windows
//! // transport_windows.go
//! 
//! // +build unix
//! // transport_unix.go
//! ```
//! 
//! Rust's approach is more flexible - can mix in same file!

#[cfg(unix)]
use std::path::PathBuf;

/// Transport configuration for the registry server.
/// 
/// # Rust Learning Note
/// 
/// This enum demonstrates **configuration as data**.
/// 
/// ```rust
/// match transport {
///     TransportConfig::Tcp { port } => bind_tcp(port),
///     TransportConfig::UnixSocket { path } => bind_uds(path),
///     TransportConfig::NamedPipe { name } => bind_pipe(name),
/// }
/// ```
/// 
/// The type system ensures you handle all cases!
#[derive(Debug, Clone)]
pub enum TransportConfig {
    /// TCP socket on specified port.
    Tcp { port: u16 },
    
    /// Unix domain socket (Unix only).
    #[cfg(unix)]
    UnixSocket { path: PathBuf },
    
    /// Named pipe (Windows only).
    #[cfg(windows)]
    NamedPipe { name: String },
}

impl TransportConfig {
    /// Creates a TCP transport config.
    pub fn tcp(port: u16) -> Self {
        Self::Tcp { port }
    }

    /// Creates a Unix domain socket config (Unix only).
    #[cfg(unix)]
    pub fn unix_socket(path: impl Into<PathBuf>) -> Self {
        Self::UnixSocket { path: path.into() }
    }

    /// Creates a named pipe config (Windows only).
    #[cfg(windows)]
    pub fn named_pipe(name: impl Into<String>) -> Self {
        Self::NamedPipe { name: name.into() }
    }

    /// Returns the default transport for the current platform.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Platform-Specific Defaults
    /// 
    /// ```rust
    /// #[cfg(unix)]
    /// fn default() -> Self {
    ///     Self::unix_socket("/tmp/hsu-registry.sock")
    /// }
    /// 
    /// #[cfg(windows)]
    /// fn default() -> Self {
    ///     Self::named_pipe("hsu-registry")
    /// }
    /// ```
    /// 
    /// The compiler ensures only the right version is compiled!
    #[cfg(unix)]
    pub fn default_platform() -> Self {
        Self::unix_socket("/tmp/hsu-registry.sock")
    }

    #[cfg(windows)]
    pub fn default_platform() -> Self {
        Self::named_pipe("hsu-registry")
    }

    #[cfg(not(any(unix, windows)))]
    pub fn default_platform() -> Self {
        Self::tcp(8080)
    }

    /// Returns a human-readable description of the transport.
    pub fn describe(&self) -> String {
        match self {
            TransportConfig::Tcp { port } => format!("TCP on port {}", port),
            
            #[cfg(unix)]
            TransportConfig::UnixSocket { path } => {
                format!("Unix domain socket at {}", path.display())
            }
            
            #[cfg(windows)]
            TransportConfig::NamedPipe { name } => {
                format!("Named pipe: {}", name)
            }
        }
    }
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self::default_platform()
    }
}

/// Transport address for clients.
/// 
/// # Rust Learning Note
/// 
/// This type encodes **how to connect** to the registry.
/// Like TransportConfig, but from the client's perspective.
#[derive(Debug, Clone)]
pub enum TransportAddress {
    /// TCP address (host:port).
    Tcp(String),
    
    /// Unix domain socket path.
    #[cfg(unix)]
    UnixSocket(PathBuf),
    
    /// Named pipe name.
    #[cfg(windows)]
    NamedPipe(String),
}

impl TransportAddress {
    /// Parses a transport address from a string.
    /// 
    /// Format:
    /// - `tcp://localhost:8080` → TCP
    /// - `unix:///tmp/registry.sock` → Unix socket
    /// - `pipe://registry-name` → Named pipe
    /// 
    /// # Rust Learning Note
    /// 
    /// This demonstrates **string parsing** with pattern matching.
    pub fn parse(s: &str) -> Result<Self, String> {
        if let Some(addr) = s.strip_prefix("tcp://") {
            Ok(Self::Tcp(addr.to_string()))
        } else if s.starts_with("unix://") {
            #[cfg(unix)]
            {
                let path = s.strip_prefix("unix://").unwrap();
                Ok(Self::UnixSocket(PathBuf::from(path)))
            }
            #[cfg(not(unix))]
            {
                let _ = s;  // Suppress unused warning
                Err("Unix sockets not supported on this platform".to_string())
            }
        } else if s.starts_with("pipe://") {
            #[cfg(windows)]
            {
                let name = s.strip_prefix("pipe://").unwrap();
                Ok(Self::NamedPipe(name.to_string()))
            }
            #[cfg(not(windows))]
            {
                let _ = s;  // Suppress unused warning
                Err("Named pipes not supported on this platform".to_string())
            }
        } else {
            Err(format!("Invalid transport address: {}", s))
        }
    }

    /// Returns the default address for the current platform.
    #[cfg(unix)]
    pub fn default_platform() -> Self {
        Self::UnixSocket(PathBuf::from("/tmp/hsu-registry.sock"))
    }

    #[cfg(windows)]
    pub fn default_platform() -> Self {
        Self::NamedPipe("hsu-registry".to_string())
    }

    #[cfg(not(any(unix, windows)))]
    pub fn default_platform() -> Self {
        Self::Tcp("localhost:8080".to_string())
    }
}

impl Default for TransportAddress {
    fn default() -> Self {
        Self::default_platform()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_config_describe() {
        let tcp = TransportConfig::tcp(8080);
        assert_eq!(tcp.describe(), "TCP on port 8080");
    }

    #[test]
    #[cfg(unix)]
    fn test_unix_socket_config() {
        let uds = TransportConfig::unix_socket("/tmp/test.sock");
        assert!(uds.describe().contains("Unix domain socket"));
    }

    #[test]
    fn test_transport_address_parse_tcp() {
        let addr = TransportAddress::parse("tcp://localhost:8080").unwrap();
        match addr {
            TransportAddress::Tcp(s) => assert_eq!(s, "localhost:8080"),
            _ => panic!("Expected TCP address"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_transport_address_parse_unix() {
        let addr = TransportAddress::parse("unix:///tmp/test.sock").unwrap();
        match addr {
            TransportAddress::UnixSocket(path) => {
                assert_eq!(path, PathBuf::from("/tmp/test.sock"));
            }
            _ => panic!("Expected Unix socket address"),
        }
    }

    #[test]
    fn test_transport_address_parse_invalid() {
        let result = TransportAddress::parse("invalid://something");
        assert!(result.is_err());
    }
}

