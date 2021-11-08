use crate::authenticated_client::server_handle::ServerHandle;
use anyhow::Context;
use flume::{Receiver, Sender};
use futures_lite::FutureExt;
use mc_packet_protocol::packet::ResolvedPacket;
use mc_packet_protocol::protocol_version::MCProtocol;
use mc_packet_protocol::registry::play::*;
use mc_packet_protocol::registry::RegistryBase;
use tokio::task::JoinHandle;

pub type ClientHandle = (Sender<ResolvedPacket>, JoinHandle<()>);

pub(crate) fn client_handle_send_packet(
    client_handle: &ClientHandle,
    packet: ResolvedPacket,
) -> anyhow::Result<()> {
    client_handle
        .0
        .send(packet)
        .context("Failed to send packet.")
}

pub type FlumeCursor = Receiver<std::io::Cursor<Vec<u8>>>;

pub(crate) struct Mediator {
    server_handle: ServerHandle,
    server_to_proxy: FlumeCursor,
    client_handle: ClientHandle,
    client_to_proxy: FlumeCursor,
    protocol: MCProtocol,
}

impl Mediator {
    pub(crate) fn new(
        server_handle: ServerHandle,
        server_to_proxy: FlumeCursor,
        client_handle: ClientHandle,
        client_to_proxy: FlumeCursor,
        protocol: MCProtocol,
    ) -> Self {
        Self {
            server_handle,
            server_to_proxy,
            client_handle,
            client_to_proxy,
            protocol,
        }
    }

    pub fn mediate(self) {
        let handle1: JoinHandle<anyhow::Result<()>> = tokio::task::spawn(async move {
            let mut handler = super::play::MediatorLoginServerBound::new(self.server_handle, self.protocol);
            loop {
                let next = self.client_to_proxy.recv_async().await?;
                server_bound::Registry::handle_packet(&mut handler, next, self.protocol).await?;
            }
        });
        let handle2: JoinHandle<anyhow::Result<()>> = tokio::task::spawn(async move {
            let mut handler = super::play::MediatorLoginClientBound::new(self.client_handle, self.protocol);
            loop {
                let next = self.server_to_proxy.recv_async().await?;
                client_bound::Registry::handle_packet(&mut handler, next, self.protocol).await?;
            }
        });
        tokio::spawn(async move {
            let result = handle1.race(handle2).await; // todo find a better way to handle this race, we need to know more about who failed
            match result {
                Ok(inner_result) => {
                    if let Err(err) = inner_result {
                        log::error!("Inner thread failed with exception: {:?}", err);
                    }
                }
                Err(err) => {
                    log::error!("Failed to handle race cleanly: {:?}", err);
                }
            }
        });
    }
}
