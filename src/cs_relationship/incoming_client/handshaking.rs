use super::status::FakeServerStatusHandler;
use super::ClientState;
use crate::Config;
use mc_packet_protocol::registry;
use async_trait::async_trait;
use minecraft_data_types::packets::handshaking::server::{Handshake, NextState};
use minecraft_data_types::Decodable;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use mc_packet_protocol::registry::LazyHandle;

pub(crate) struct FakeServerHandshakingHandler {
    next_state: Box<Option<ClientState>>,
    config: Arc<Mutex<Config>>,
    client_address: Arc<SocketAddr>,
}

impl FakeServerHandshakingHandler {
    pub fn new(config: &Arc<Mutex<Config>>, client_address: &Arc<SocketAddr>) -> Self {
        FakeServerHandshakingHandler {
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
impl registry::HandshakingServerBoundRegistryHandler
    for FakeServerHandshakingHandler
{
    async fn handle_default<T: Decodable, H: LazyHandle<T> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_handshake<H: LazyHandle<Handshake> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let handshake = handle.decode_type()?;
        log::trace!(target: &self.client_address.to_string(), "Received handshake {:#?}", handshake);
        match handshake.next_state {
            NextState::Status => {
                self.next_state = Box::new(Some(ClientState::Status(FakeServerStatusHandler::new(
                    &self.config,
                    &self.client_address,
                ))))
            }
            NextState::Login => self.next_state = Box::new(Some(ClientState::End)),
        }
        Ok(())
    }
}
