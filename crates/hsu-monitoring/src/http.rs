// HTTP health check implementation

use crate::{HealthCheckError, HealthCheckResult, HealthCheckData};
use chrono::Utc;
use hyper::{Method, Request, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, warn};

/// HTTP health check configuration
#[derive(Debug, Clone)]
pub struct HttpHealthCheckConfig {
    pub endpoint: String,
    pub method: HttpMethod,
    pub timeout: Duration,
    pub expected_status: Vec<u16>,
    pub expected_body: Option<String>,
}

/// HTTP methods supported for health checks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Head,
}

impl Default for HttpHealthCheckConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            method: HttpMethod::Get,
            timeout: Duration::from_secs(5),
            expected_status: vec![200],
            expected_body: None,
        }
    }
}

impl HttpHealthCheckConfig {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            ..Default::default()
        }
    }
    
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    
    pub fn with_method(mut self, method: HttpMethod) -> Self {
        self.method = method;
        self
    }
    
    pub fn with_expected_status(mut self, status_codes: Vec<u16>) -> Self {
        self.expected_status = status_codes;
        self
    }
}

/// Perform HTTP health check
pub async fn check_http_health(
    endpoint: &str,
    check_timeout: Duration,
) -> HealthCheckResult<HealthCheckData> {
    let config = HttpHealthCheckConfig::new(endpoint.to_string())
        .with_timeout(check_timeout);
    
    check_http_health_with_config(&config).await
}

/// Perform HTTP health check with custom configuration
pub async fn check_http_health_with_config(
    config: &HttpHealthCheckConfig,
) -> HealthCheckResult<HealthCheckData> {
    let start_time = std::time::Instant::now();
    
    debug!("Starting HTTP health check: {}", config.endpoint);
    
    // Parse URI
    let uri: Uri = config.endpoint.parse().map_err(|e| {
        HealthCheckError::InvalidResponse {
            id: config.endpoint.clone(),
            response: format!("Invalid URI: {}", e),
        }
    })?;
    
    // Build HTTP client
    let client = Client::builder(TokioExecutor::new()).build_http();
    
    // Build request
    let method = match config.method {
        HttpMethod::Get => Method::GET,
        HttpMethod::Post => Method::POST,
        HttpMethod::Head => Method::HEAD,
    };
    
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("User-Agent", "HSU-ProcessManager/1.0")
        .body(Empty::<Bytes>::new())
        .map_err(|e| HealthCheckError::InvalidResponse {
            id: config.endpoint.clone(),
            response: format!("Failed to build request: {}", e),
        })?;
    
    // Perform request with timeout
    let response_result = timeout(config.timeout, client.request(request)).await;
    
    let response = match response_result {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            warn!("HTTP health check connection failed: {} - {}", config.endpoint, e);
            let elapsed = start_time.elapsed().as_millis() as u64;
            return Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(elapsed),
                error_message: Some(format!("Connection failed: {}", e)),
            });
        }
        Err(_) => {
            warn!("HTTP health check timeout: {}", config.endpoint);
            return Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(config.timeout.as_millis() as u64),
                error_message: Some("Timeout".to_string()),
            });
        }
    };
    
    let status = response.status();
    let elapsed = start_time.elapsed().as_millis() as u64;
    
    // Check if status code is expected
    let is_healthy = config.expected_status.contains(&status.as_u16());
    
    // Read response body if needed
    if !is_healthy || config.expected_body.is_some() {
        let body_bytes = response.into_body().collect().await
            .map_err(|e| HealthCheckError::InvalidResponse {
                id: config.endpoint.clone(),
                response: format!("Failed to read body: {}", e),
            })?
            .to_bytes();
        
        let body_str = String::from_utf8_lossy(&body_bytes);
        
        // Check expected body if configured
        if let Some(ref expected) = config.expected_body {
            let body_matches = body_str.contains(expected);
            if !body_matches {
                debug!(
                    "HTTP health check body mismatch: {} (expected '{}', got '{}')",
                    config.endpoint, expected, body_str
                );
                return Ok(HealthCheckData {
                    is_healthy: false,
                    checked_at: Utc::now(),
                    response_time_ms: Some(elapsed),
                    error_message: Some(format!("Body does not contain '{}'", expected)),
                });
            }
        }
    }
    
    debug!(
        "HTTP health check complete: {} - status={} healthy={} time={}ms",
        config.endpoint, status, is_healthy, elapsed
    );
    
    Ok(HealthCheckData {
        is_healthy,
        checked_at: Utc::now(),
        response_time_ms: Some(elapsed),
        error_message: if !is_healthy {
            Some(format!("Unexpected status code: {}", status))
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_http_health_check_config() {
        let config = HttpHealthCheckConfig::new("http://localhost:8080/health".to_string())
            .with_timeout(Duration::from_secs(10))
            .with_method(HttpMethod::Get)
            .with_expected_status(vec![200, 204]);
        
        assert_eq!(config.endpoint, "http://localhost:8080/health");
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.method, HttpMethod::Get);
        assert_eq!(config.expected_status, vec![200, 204]);
    }
    
    #[tokio::test]
    async fn test_http_health_check_invalid_url() {
        let result = check_http_health("not-a-valid-url", Duration::from_secs(5)).await;
        // Invalid URL returns an error during URI parsing
        match result {
            Err(_) => {}, // Expected
            Ok(data) => {
                // Or it returns unhealthy status
                assert!(!data.is_healthy, "Invalid URL should result in unhealthy status");
            }
        }
    }
    
    // Note: Real HTTP tests would require a test server
    // These can be added as integration tests
}
