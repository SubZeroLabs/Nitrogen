use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use mc_packet_protocol::registry;
use mc_packet_protocol::registry::LazyHandle;
use minecraft_data_types::Decodable;
use tokio::sync::Mutex;

use crate::Config;
use super::ClientState;

pub(crate) struct FakeServerLoginHandler {
    next_state: Box<Option<ClientState>>,
    config: Arc<Mutex<Config>>,
    client_address: Arc<SocketAddr>,
}

impl FakeServerLoginHandler {
    pub fn new(config: &Arc<Mutex<Config>>, client_address: &Arc<SocketAddr>) -> Self {
        Self {
            next_state: Box::new(None),
            config: Arc::clone(config),
            client_address: Arc::clone(client_address),
        }
    }

    pub fn next_state(self) -> ClientState {
        self.next_state.unwrap()
    }

    pub fn peek_state(&self) -> bool {
        !matches!(*self.next_state, None)
    }
}

#[async_trait]
impl registry::LoginServerBoundRegistryHandler for FakeServerLoginHandler {
    async fn handle_default<T: Decodable, H: LazyHandle<T> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        handle.consume_bytes()
    }
}
