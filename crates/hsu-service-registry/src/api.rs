//! HTTP API handlers using axum.
//! 
//! # Rust Learning Note
//! 
//! This module demonstrates **axum** - a modern, ergonomic web framework for Rust.
//! 
//! ## axum vs Go's net/http
//! 
//! **Go:**
//! ```go
//! http.HandleFunc("/api/v1/publish", func(w http.ResponseWriter, r *http.Request) {
//!     var req PublishRequest
//!     json.NewDecoder(r.Body).Decode(&req)
//!     // ... handle request
//!     json.NewEncoder(w).Encode(response)
//! })
//! ```
//! 
//! **Rust with axum:**
//! ```rust
//! async fn publish(
//!     State(registry): State<Arc<Registry>>,
//!     Json(req): Json<PublishRequest>,
//! ) -> Result<Json<PublishResponse>, StatusCode> {
//!     // ... handle request
//!     Ok(Json(response))
//! }
//! ```
//! 
//! ## Key axum Features
//! 
//! 1. **Extractors**: Automatic parsing (Json, Query, Path, State)
//! 2. **Type-safe**: Wrong types = compile error
//! 3. **Async**: Built on tokio
//! 4. **Composable**: Easy to add middleware

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use hsu_common::ModuleID;
use std::sync::Arc;
use tracing::{error, info};

use crate::{
    storage::Registry,
    types::{
        DiscoverResponse, ErrorResponse, ModuleInfo, PublishRequest, PublishResponse,
    },
};

/// Creates the API router.
/// 
/// # Rust Learning Note
/// 
/// ## Router Builder Pattern
/// 
/// ```rust
/// Router::new()
///     .route("/publish", post(publish_handler))
///     .route("/discover/:id", get(discover_handler))
///     .with_state(registry)
/// ```
/// 
/// This is a **builder pattern**:
/// - Chain methods to configure
/// - Type-safe: Can't forget required config
/// - Compile-time checked routes
/// 
/// In Go, you'd manually register each route.
pub fn create_router(registry: Arc<Registry>) -> Router {
    Router::new()
        .route("/api/v1/publish", post(publish_handler))
        .route("/api/v1/discover/:module_id", get(discover_handler))
        .route("/api/v1/list", get(list_handler))
        .route("/api/v1/health", get(health_handler))
        .with_state(registry)
}

/// Publishes module APIs.
/// 
/// # Rust Learning Note
/// 
/// ## axum Extractors
/// 
/// ```rust
/// async fn publish_handler(
///     State(registry): State<Arc<Registry>>,
///     Json(req): Json<PublishRequest>,
/// ) -> Result<Json<PublishResponse>, ApiError>
/// ```
/// 
/// **What's happening:**
/// 1. `State(registry)`: Extract shared state (our Registry)
/// 2. `Json(req)`: Parse JSON body into PublishRequest
/// 3. Return type: Either success (Json) or error (ApiError)
/// 
/// **Type safety:**
/// - If JSON parsing fails: Automatic 400 Bad Request
/// - If we return wrong type: Compile error
/// - No manual error handling for parsing!
async fn publish_handler(
    State(registry): State<Arc<Registry>>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<PublishResponse>, ApiError> {
    info!(
        "Publishing APIs for module: {} (process: {})",
        req.module_id, req.process_id
    );

    let info = ModuleInfo::new(req.module_id.clone(), req.process_id, req.apis);

    registry
        .publish(info)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(PublishResponse {
        success: true,
        message: Some(format!("Published APIs for module {}", req.module_id)),
    }))
}

/// Discovers APIs for a module.
/// 
/// # Rust Learning Note
/// 
/// ## Path Extractor
/// 
/// ```rust
/// async fn discover_handler(
///     Path(module_id): Path<String>,
/// )
/// ```
/// 
/// The `Path` extractor automatically:
/// - Parses URL path segments
/// - Converts to the target type (String, i32, etc.)
/// - Returns 400 if conversion fails
/// 
/// In Go:
/// ```go
/// moduleID := r.URL.Query().Get("module_id")
/// // Manual parsing and validation
/// ```
async fn discover_handler(
    State(registry): State<Arc<Registry>>,
    Path(module_id): Path<String>,
) -> Result<Json<DiscoverResponse>, ApiError> {
    let module_id = ModuleID::from(module_id);
    
    info!("Discovering module: {}", module_id);

    let info = registry
        .discover(&module_id)
        .map_err(|_| ApiError::NotFound(format!("Module {} not found", module_id)))?;

    Ok(Json(DiscoverResponse {
        module_id: info.module_id,
        apis: info.apis,
    }))
}

/// Lists all registered modules.
async fn list_handler(
    State(registry): State<Arc<Registry>>,
) -> Json<Vec<ModuleInfo>> {
    info!("Listing all modules");
    let modules = registry.list_all();
    Json(modules)
}

/// Health check endpoint.
/// 
/// # Rust Learning Note
/// 
/// ## Simple Response
/// 
/// ```rust
/// async fn health_handler() -> &'static str {
///     "OK"
/// }
/// ```
/// 
/// axum automatically converts common types to HTTP responses:
/// - `&str` → 200 OK with text/plain
/// - `String` → 200 OK with text/plain  
/// - `Json<T>` → 200 OK with application/json
/// - `StatusCode` → Response with that status
/// 
/// This is called **IntoResponse** trait.
async fn health_handler() -> &'static str {
    "OK"
}

/// API error type.
/// 
/// # Rust Learning Note
/// 
/// ## Custom Error Responses
/// 
/// We implement `IntoResponse` to convert our errors into HTTP responses.
/// This is like Go's error handling but type-safe:
/// 
/// ```go
/// // Go
/// if err != nil {
///     http.Error(w, err.Error(), http.StatusInternalServerError)
///     return
/// }
/// ```
/// 
/// ```rust
/// // Rust - automatic with ?
/// let result = some_operation().map_err(ApiError::Internal)?;
/// // If error, automatically converted to HTTP response!
/// ```
#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        error!("API error: {} - {}", status, message);

        let body = Json(ErrorResponse { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RemoteModuleAPI;
    use axum::{body::Body, http::Request};
    use hsu_common::Protocol;
    use tower::util::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn test_publish_endpoint() {
        let registry = Arc::new(Registry::new());
        let app = create_router(registry);

        let request_body = PublishRequest {
            module_id: ModuleID::from("test-module"),
            process_id: 12345,
            apis: vec![RemoteModuleAPI {
                service_id: "service1".to_string(),
                protocol: Protocol::Grpc,
                address: Some("localhost:50051".to_string()),
                metadata: None,
            }],
        };

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/publish")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_discover_endpoint() {
        let registry = Arc::new(Registry::new());
        
        // Pre-populate registry
        let info = ModuleInfo::new(
            ModuleID::from("test-module"),
            12345,
            vec![RemoteModuleAPI {
                service_id: "service1".to_string(),
                protocol: Protocol::Grpc,
                address: Some("localhost:50051".to_string()),
                metadata: None,
            }],
        );
        registry.publish(info).unwrap();

        let app = create_router(registry);

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/discover/test-module")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let registry = Arc::new(Registry::new());
        let app = create_router(registry);

        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

