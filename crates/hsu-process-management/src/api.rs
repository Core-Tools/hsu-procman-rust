// API module for gRPC and HTTP endpoints
// TODO: Implement management API

pub mod grpc;
pub mod http;

pub struct ApiServer {
    #[allow(dead_code)]
    port: u16,
}

impl ApiServer {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: Start API server
        Ok(())
    }
}
