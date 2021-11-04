use std::sync::Arc;
use mc_packet_protocol::packet::PacketReadWriteLocker;
use mc_packet_protocol::protocol_version::*;
use mc_packet_protocol::registry::*;
use tokio::sync::Mutex;
use mc_packet_protocol::registry::handshake::server_bound::NextState;
use crate::incoming_client::status::ServerBoundStatusHandler;
use crate::incoming_client::login::ServerBoundLoginHandler;

pub struct ServerBoundHandshakeHandler<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin
> {
    client_handle: Arc<Mutex<super::ClientHandle>>,
    locker: Arc<PacketReadWriteLocker<R, W>>,
    next_state: Option<Box<super::ClientState<R, W>>>,
}

impl<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin
> ServerBoundHandshakeHandler<R, W> {
    pub fn new(client_handle: Arc<Mutex<super::ClientHandle>>, locker: Arc<PacketReadWriteLocker<R, W>>) -> Self {
        Self { client_handle, locker, next_state: None }
    }

    pub(crate) fn next_state(self) -> super::ClientState<R, W> {
        *self.next_state.unwrap()
    }

    pub(crate) fn peek_state(&self) -> bool {
        self.next_state.is_some()
    }
}

#[async_trait::async_trait]
impl<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin
> handshake::server_bound::RegistryHandler for ServerBoundHandshakeHandler<R, W> {
    async fn handle_default<T: MapDecodable, H: LazyHandle<T> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_handshake<H: LazyHandle<handshake::server_bound::Handshake> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        let handshake = handle.decode_type()?;
        let target_protocol = MCProtocol::from(handshake.protocol_version);

        let mut locked_client = self.client_handle.lock().await;
        locked_client.update_protocol(target_protocol);
        drop(locked_client);

        match handshake.next_state {
            NextState::Status => self.next_state = Some(Box::new(super::ClientState::Status(ServerBoundStatusHandler::new(Arc::clone(&self.client_handle), Arc::clone(&self.locker))))),
            NextState::Login => self.next_state = Some(Box::new(super::ClientState::Login(ServerBoundLoginHandler::new(Arc::clone(&self.client_handle), Arc::clone(&self.locker))))),
        }

        Ok(())
    }
}