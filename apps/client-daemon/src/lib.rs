pub mod engine;
pub mod socks;

use config::EndpointDescriptor;
use std::sync::Arc;
use transport_api::Transport;

pub fn default_transports() -> Vec<Arc<dyn Transport>> {
    vec![
        Arc::new(transport_native::DirectTlsTransport),
        Arc::new(transport_native::NoiseQuicTransport),
    ]
}

pub fn build_engine(endpoints: Vec<EndpointDescriptor>) -> Arc<engine::ConnectionEngine> {
    Arc::new(engine::ConnectionEngine::new(
        default_transports(),
        endpoints,
    ))
}
