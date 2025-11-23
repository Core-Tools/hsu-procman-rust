// gRPC health check implementation
// Implements the standard gRPC Health Checking Protocol:
// https://github.com/grpc/grpc/blob/master/doc/health-checking.md

use crate::{HealthCheckError, HealthCheckResult, HealthCheckData};
use chrono::Utc;
use std::time::Duration;
use tokio::time::timeout;
use tonic::transport::Channel;
use tracing::{debug, warn};

// Import the generated proto types (these will be generated from health.proto)
pub mod health {
    tonic::include_proto!("grpc.health.v1");
}

use health::health_client::HealthClient;
use health::HealthCheckRequest;

/// gRPC health check configuration
#[derive(Debug, Clone)]
pub struct GrpcHealthCheckConfig {
    pub address: String,
    pub service_name: String,
    pub timeout: Duration,
}

impl GrpcHealthCheckConfig {
    pub fn new(address: String) -> Self {
        Self {
            address,
            service_name: String::new(), // Empty string checks overall server health
            timeout: Duration::from_secs(5),
        }
    }
    
    pub fn with_service(mut self, service_name: String) -> Self {
        self.service_name = service_name;
        self
    }
    
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// Perform gRPC health check
pub async fn check_grpc_health(
    address: &str,
    check_timeout: Duration,
) -> HealthCheckResult<HealthCheckData> {
    let config = GrpcHealthCheckConfig::new(address.to_string())
        .with_timeout(check_timeout);
    
    check_grpc_health_with_config(&config).await
}

/// Perform gRPC health check with custom configuration
pub async fn check_grpc_health_with_config(
    config: &GrpcHealthCheckConfig,
) -> HealthCheckResult<HealthCheckData> {
    let start_time = std::time::Instant::now();
    
    debug!("Starting gRPC health check: {} (service: {})", 
        config.address, 
        if config.service_name.is_empty() { "all" } else { &config.service_name }
    );
    
    // Connect to gRPC server with timeout
    let channel_result = timeout(
        config.timeout,
        Channel::from_shared(config.address.clone())
            .map_err(|e| HealthCheckError::InvalidResponse {
                id: config.address.clone(),
                response: format!("Invalid address: {}", e),
            })?
            .connect(),
    ).await;
    
    let channel = match channel_result {
        Ok(Ok(ch)) => ch,
        Ok(Err(e)) => {
            warn!("gRPC health check connection failed: {} - {}", config.address, e);
            let elapsed = start_time.elapsed().as_millis() as u64;
            return Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(elapsed),
                error_message: Some(format!("Connection failed: {}", e)),
            });
        }
        Err(_) => {
            warn!("gRPC health check connection timeout: {}", config.address);
            return Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(config.timeout.as_millis() as u64),
                error_message: Some("Connection timeout".to_string()),
            });
        }
    };
    
    // Create health check client
    let mut client = HealthClient::new(channel);
    
    // Prepare health check request
    let request = tonic::Request::new(HealthCheckRequest {
        service: config.service_name.clone(),
    });
    
    // Perform health check with timeout
    let response_result = timeout(config.timeout, client.check(request)).await;
    
    let elapsed = start_time.elapsed().as_millis() as u64;
    
    match response_result {
        Ok(Ok(response)) => {
            let health_response = response.into_inner();
            
            // Check the serving status
            // 0 = UNKNOWN, 1 = SERVING, 2 = NOT_SERVING, 3 = SERVICE_UNKNOWN
            let is_healthy = health_response.status == 1; // SERVING
            
            debug!(
                "gRPC health check complete: {} - status={} healthy={} time={}ms",
                config.address, health_response.status, is_healthy, elapsed
            );
            
            Ok(HealthCheckData {
                is_healthy,
                checked_at: Utc::now(),
                response_time_ms: Some(elapsed),
                error_message: if !is_healthy {
                    Some(format!("Service not serving (status: {})", health_response.status))
                } else {
                    None
                },
            })
        }
        Ok(Err(status)) => {
            warn!("gRPC health check failed: {} - {}", config.address, status);
            Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(elapsed),
                error_message: Some(format!("gRPC error: {}", status)),
            })
        }
        Err(_) => {
            warn!("gRPC health check timeout: {}", config.address);
            Ok(HealthCheckData {
                is_healthy: false,
                checked_at: Utc::now(),
                response_time_ms: Some(config.timeout.as_millis() as u64),
                error_message: Some("Health check timeout".to_string()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_grpc_health_check_config() {
        let config = GrpcHealthCheckConfig::new("http://localhost:50051".to_string())
            .with_service("my.service.v1".to_string())
            .with_timeout(Duration::from_secs(10));
        
        assert_eq!(config.address, "http://localhost:50051");
        assert_eq!(config.service_name, "my.service.v1");
        assert_eq!(config.timeout, Duration::from_secs(10));
    }
    
    // Note: Real gRPC tests would require a test server with health checking
    // These can be added as integration tests
}
