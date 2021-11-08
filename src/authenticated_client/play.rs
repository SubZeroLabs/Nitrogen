use crate::authenticated_client::mediator::{client_handle_send_packet, ClientHandle};
use crate::authenticated_client::server_handle::ServerHandle;
use mc_packet_protocol::packet::{ResolvedPacket, WritablePacket};
use mc_packet_protocol::{registry::play::*, registry::*};
use mc_packet_protocol::protocol_version::MCProtocol;

pub struct MediatorLoginServerBound {
    server_handle: ServerHandle,
    protocol: MCProtocol,
}

impl MediatorLoginServerBound {
    pub(crate) fn new(server_handle: ServerHandle, protocol: MCProtocol) -> Self {
        Self { server_handle, protocol }
    }
}

#[async_trait::async_trait]
impl server_bound::RegistryHandler for MediatorLoginServerBound {
    async fn handle_unknown(
        &mut self,
        packet_cursor: std::io::Cursor<Vec<u8>>,
    ) -> anyhow::Result<()> {
        self.server_handle
            .send_packet(ResolvedPacket::from_cursor(packet_cursor)?)
    }

    async fn handle_default<
        T: mc_packet_protocol::protocol_version::MapDecodable,
        H: LazyHandle<T> + Send,
    >(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        self.server_handle
            .send_packet(handle.into_resolved_packet()?)
    }

    async fn handle_plugin_message<H: LazyHandle<server_bound::PluginMessage> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        let mut message = handle.decode_type()?;
        if super::plugin_message_util::is_brand(&message.channel) {
            message.data = super::plugin_message_util::rewrite_brand(message.data)?;
        }
        self.server_handle.send_packet(message.to_resolved_packet(self.protocol)?)?;
        Ok(())
    }
}

pub struct MediatorLoginClientBound {
    client_handle: ClientHandle,
    protocol: MCProtocol,
}

impl MediatorLoginClientBound {
    pub(crate) fn new(client_handle: ClientHandle, protocol: MCProtocol) -> Self {
        Self { client_handle, protocol }
    }
}

#[async_trait::async_trait]
impl client_bound::RegistryHandler for MediatorLoginClientBound {
    async fn handle_unknown(
        &mut self,
        packet_cursor: std::io::Cursor<Vec<u8>>,
    ) -> anyhow::Result<()> {
        client_handle_send_packet(
            &self.client_handle,
            ResolvedPacket::from_cursor(packet_cursor)?,
        )
    }

    async fn handle_default<
        T: mc_packet_protocol::protocol_version::MapDecodable,
        H: LazyHandle<T> + Send,
    >(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        client_handle_send_packet(&self.client_handle, handle.into_resolved_packet()?)
    }

    async fn handle_plugin_message<H: LazyHandle<client_bound::PluginMessage> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        let mut message = handle.decode_type()?;
        if super::plugin_message_util::is_brand(&message.channel) {
            message.data = super::plugin_message_util::rewrite_brand(message.data)?;
        }
        self.client_handle.0.send(message.to_resolved_packet(self.protocol)?)?;
        Ok(())
    }
}
