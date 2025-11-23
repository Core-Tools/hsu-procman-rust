//! Service registry client implementation.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **HTTP client in Rust** using hyper.
//! 
//! ## hyper vs Go's net/http
//! 
//! **Go:**
//! ```go
//! resp, err := http.Get("http://localhost:8080/api/v1/discover/module")
//! if err != nil {
//!     return err
//! }
//! defer resp.Body.Close()
//! 
//! var result DiscoverResponse
//! json.NewDecoder(resp.Body).Decode(&result)
//! ```
//! 
//! **Rust with hyper:**
//! ```rust
//! let resp = client.get(uri).await?;
//! let bytes = hyper::body::to_bytes(resp.into_body()).await?;
//! let result: DiscoverResponse = serde_json::from_slice(&bytes)?;
//! ```
//! 
//! ## Key Differences
//! 
//! 1. **Type Safety**: Rust checks JSON types at compile time with serde
//! 2. **Async**: Built on tokio (no goroutines needed)
//! 3. **Error Handling**: Result<T> with ? operator
//! 4. **Memory**: No GC, explicit ownership

use hsu_common::{ModuleID, Protocol, Result, Error};
use hyper::{Request, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// Remote API information from service registry.
/// 
/// # Rust Learning Note
///
/// This mirrors the Go type but uses Rust idioms:
/// - `serde` for JSON serialization
/// - `Option<T>` instead of pointers
/// - `HashMap` instead of `map`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAPI {
    pub service_id: String,
    pub protocol: Protocol,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// Response from service discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverResponse {
    pub module_id: ModuleID,
    pub apis: Vec<RemoteAPI>,
}

/// Service registry client.
/// 
/// # Rust Learning Note
/// 
/// ## Client Structure
/// 
/// ```rust
/// pub struct ServiceRegistryClient {
///     base_url: String,
///     client: Client<HttpConnector>,
/// }
/// ```
/// 
/// - `base_url`: Registry server address
/// - `client`: Reusable HTTP client (connection pooling!)
/// 
/// ## Connection Pooling
/// 
/// hyper automatically pools connections:
/// - Reuses TCP connections
/// - Keep-alive by default
/// - No manual management needed
/// 
/// **Much simpler than Go's manual http.Client configuration!**
pub struct ServiceRegistryClient {
    /// Base URL of the service registry (e.g., "http://localhost:8080").
    base_url: String,
    
    /// Hyper HTTP client (reusable, pooled connections).
    client: Client<HttpConnector, Full<Bytes>>,
}

impl ServiceRegistryClient {
    /// Creates a new service registry client.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Default Client
    /// 
    /// ```rust
    /// let client = Client::new();
    /// ```
    /// 
    /// This creates a client with:
    /// - HTTP/1.1 support
    /// - Connection pooling
    /// - Keep-alive enabled
    /// - Automatic retries (configurable)
    /// 
    /// **All the good defaults!**
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder(TokioExecutor::new())
            .build_http();
        
        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// Discovers APIs for a specific module.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## Async HTTP Request Flow
    /// 
    /// ```rust
    /// // 1. Build URI
    /// let uri: Uri = format!("{}/api/v1/discover/{}", base_url, module_id)
    ///     .parse()?;
    /// 
    /// // 2. Make request
    /// let resp = self.client.get(uri).await?;
    /// 
    /// // 3. Read body
    /// let bytes = to_bytes(resp.into_body()).await?;
    /// 
    /// // 4. Parse JSON
    /// let result: DiscoverResponse = serde_json::from_slice(&bytes)?;
    /// ```
    /// 
    /// **Error handling at each step with ?**
    /// 
    /// ## Performance
    /// 
    /// - Connection pooling: Reuses connections
    /// - Keep-alive: Reduces TCP overhead
    /// - Async: Non-blocking I/O
    /// - Zero-copy: Bytes are passed by reference
    pub async fn discover(&self, module_id: &ModuleID) -> Result<Vec<RemoteAPI>> {
        let url = format!("{}/api/v1/discover/{}", self.base_url, module_id);
        debug!("Discovering module: {} from {}", module_id, url);

        // Parse URI
        let uri: Uri = url.parse()
            .map_err(|e| Error::Protocol(format!("Invalid URI: {}", e)))?;

        // Build GET request
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .body(Full::new(Bytes::new()))
            .map_err(|e| Error::Protocol(format!("Failed to build request: {}", e)))?;

        // Send request
        let resp = self.client
            .request(req)
            .await
            .map_err(|e| Error::Protocol(format!("HTTP request failed: {}", e)))?;

        // Check status
        if !resp.status().is_success() {
            if resp.status().as_u16() == 404 {
                return Err(Error::module_not_found(module_id.clone()));
            }
            return Err(Error::Protocol(format!(
                "Registry returned status: {}",
                resp.status()
            )));
        }

        // Read body
        let body = resp.into_body();
        let bytes = body.collect()
            .await
            .map_err(|e| Error::Protocol(format!("Failed to read response: {}", e)))?
            .to_bytes();

        // Parse JSON
        let result: DiscoverResponse = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Protocol(format!("Failed to parse JSON: {}", e)))?;

        debug!("Discovered {} APIs for module {}", result.apis.len(), module_id);

        Ok(result.apis)
    }

    /// Publishes APIs to the service registry.
    /// 
    /// # Rust Learning Note
    /// 
    /// ## POST Request with JSON Body
    /// 
    /// ```rust
    /// // 1. Serialize to JSON
    /// let body = serde_json::to_vec(&request)?;
    /// 
    /// // 2. Build request
    /// let req = Request::builder()
    ///     .method("POST")
    ///     .uri(uri)
    ///     .header("content-type", "application/json")
    ///     .body(Body::from(body))?;
    /// 
    /// // 3. Send
    /// let resp = self.client.request(req).await?;
    /// ```
    pub async fn publish(
        &self,
        module_id: &ModuleID,
        process_id: u32,
        apis: Vec<RemoteAPI>,
    ) -> Result<()> {
        let url = format!("{}/api/v1/publish", self.base_url);
        debug!("Publishing {} APIs for module {}", apis.len(), module_id);

        // Build request body
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct PublishRequest {
            module_id: ModuleID,
            process_id: u32,
            apis: Vec<RemoteAPI>,
        }

        let request = PublishRequest {
            module_id: module_id.clone(),
            process_id,
            apis,
        };

        // Serialize to JSON
        let body = serde_json::to_vec(&request)
            .map_err(|e| Error::Protocol(format!("Failed to serialize request: {}", e)))?;

        // Parse URI
        let uri: Uri = url.parse()
            .map_err(|e| Error::Protocol(format!("Invalid URI: {}", e)))?;

        // Build POST request
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|e| Error::Protocol(format!("Failed to build request: {}", e)))?;

        // Send request  
        let resp = self.client
            .request(req)
            .await
            .map_err(|e| Error::Protocol(format!("HTTP request failed: {}", e)))?;

        // Check status
        if !resp.status().is_success() {
            return Err(Error::Protocol(format!(
                "Registry returned status: {}",
                resp.status()
            )));
        }

        debug!("Successfully published APIs for module {}", module_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = ServiceRegistryClient::new("http://localhost:8080");
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_remote_api_serialization() {
        let api = RemoteAPI {
            service_id: "echo".to_string(),
            protocol: Protocol::Grpc,
            address: Some("localhost:50051".to_string()),
            metadata: None,
        };

        // Serialize
        let json = serde_json::to_string(&api).unwrap();
        
        // Deserialize
        let decoded: RemoteAPI = serde_json::from_str(&json).unwrap();
        
        assert_eq!(decoded.service_id, "echo");
        assert_eq!(decoded.protocol, Protocol::Grpc);
    }

    // Note: Integration tests that actually call the registry
    // would require the registry server to be running.
}

