//! Data types for the service registry.
//! 
//! # Rust Learning Note
//! 
//! These types mirror the Go implementation but leverage Rust's type system:
//! - `serde` for automatic JSON serialization
//! - `chrono` for type-safe timestamps
//! - `Clone` for cheap copying when needed

use chrono::{DateTime, Utc};
use hsu_common::{ModuleID, Protocol};
use serde::{Deserialize, Serialize};

/// Information about a remote module API endpoint.
/// 
/// This represents a single service endpoint that a module exposes.
/// 
/// # Rust Learning Note
/// 
/// ## derive Macros
/// 
/// ```rust
/// #[derive(Debug, Clone, Serialize, Deserialize)]
/// ```
/// 
/// These automatically generate trait implementations:
/// - `Debug`: For printing with `{:?}`
/// - `Clone`: For making copies
/// - `Serialize`: For converting to JSON
/// - `Deserialize`: For parsing from JSON
/// 
/// In Go, you'd manually write JSON tags:
/// ```go
/// type RemoteModuleAPI struct {
///     ServiceID string `json:"serviceID"`
///     Protocol  string `json:"protocol"`
/// }
/// ```
/// 
/// Rust's `serde` is **much** more powerful - it's compile-time checked!
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteModuleAPI {
    /// Service identifier within the module.
    pub service_id: String,
    
    /// Communication protocol (e.g., "grpc", "http").
    pub protocol: Protocol,
    
    /// Network address (e.g., "localhost:50051").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    
    /// Additional metadata (protocol-specific).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Information about a registered module.
/// 
/// # Rust Learning Note
/// 
/// Notice the `DateTime<Utc>` type - this is from the `chrono` crate.
/// Unlike Go's `time.Time`, Rust's type system lets you:
/// - Distinguish UTC vs local time at compile time
/// - Use different calendar systems
/// - Have nanosecond precision
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInfo {
    /// Unique module identifier.
    pub module_id: ModuleID,
    
    /// Process ID of the module.
    pub process_id: u32,
    
    /// List of APIs this module provides.
    pub apis: Vec<RemoteModuleAPI>,
    
    /// When this entry was registered.
    pub registered_at: DateTime<Utc>,
    
    /// When this entry was last updated.
    pub last_updated: DateTime<Utc>,
}

impl ModuleInfo {
    /// Creates a new ModuleInfo.
    pub fn new(module_id: ModuleID, process_id: u32, apis: Vec<RemoteModuleAPI>) -> Self {
        let now = Utc::now();
        Self {
            module_id,
            process_id,
            apis,
            registered_at: now,
            last_updated: now,
        }
    }
    
    /// Updates the APIs and last_updated timestamp.
    pub fn update_apis(&mut self, apis: Vec<RemoteModuleAPI>) {
        self.apis = apis;
        self.last_updated = Utc::now();
    }
}

/// Request to publish module APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    pub module_id: ModuleID,
    pub process_id: u32,
    pub apis: Vec<RemoteModuleAPI>,
}

/// Response from publishing APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Response from discovering a module.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResponse {
    pub module_id: ModuleID,
    pub apis: Vec<RemoteModuleAPI>,
}

/// Error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_info_creation() {
        let module_id = ModuleID::from("test");
        let info = ModuleInfo::new(module_id.clone(), 12345, vec![]);
        
        assert_eq!(info.module_id, module_id);
        assert_eq!(info.process_id, 12345);
        assert_eq!(info.apis.len(), 0);
    }

    #[test]
    fn test_module_info_update() {
        let mut info = ModuleInfo::new(ModuleID::from("test"), 12345, vec![]);
        
        let api = RemoteModuleAPI {
            service_id: "service1".to_string(),
            protocol: Protocol::Grpc,
            address: Some("localhost:50051".to_string()),
            metadata: None,
        };
        
        info.update_apis(vec![api]);
        assert_eq!(info.apis.len(), 1);
    }

    #[test]
    fn test_json_serialization() {
        let api = RemoteModuleAPI {
            service_id: "service1".to_string(),
            protocol: Protocol::Grpc,
            address: Some("localhost:50051".to_string()),
            metadata: None,
        };
        
        // Serialize to JSON
        let json = serde_json::to_string(&api).unwrap();
        
        // Deserialize back
        let decoded: RemoteModuleAPI = serde_json::from_str(&json).unwrap();
        
        assert_eq!(decoded.service_id, "service1");
    }
}

