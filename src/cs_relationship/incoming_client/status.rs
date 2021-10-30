use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use log::{debug, trace};
use mc_packet_protocol::packet::{WritablePacket, ResolvedPacket};
use mc_packet_protocol::registry::LazyHandle;
use minecraft_data_types::Decodable;
use minecraft_data_types::packets::status::client::{Pong, StatusResponse, StatusResponseJson};
use minecraft_data_types::packets::status::server::{Ping, StatusRequest};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::Config;
use super::Client;
use super::ClientState;

#[derive(serde_derive::Serialize)]
struct PlayerSamplePart {
    name: String,
    id: String,
}

#[derive(serde_derive::Serialize)]
struct VersionPart {
    name: String,
    protocol: i32,
}

#[derive(serde_derive::Serialize)]
struct PlayersPart {
    max: usize,
    online: usize,
    sample: Vec<PlayerSamplePart>,
}

#[derive(serde_derive::Serialize)]
struct DescriptionPart {
    text: String, // update this later maybe to a "ChatJsonFormatted" string type when it's impl
}

#[derive(serde_derive::Serialize)]
struct StatusResponseJsonRepresentation {
    version: VersionPart,
    players: PlayersPart,
    description: DescriptionPart,
    #[serde(skip_serializing_if = "Option::is_none")]
    favicon: Option<String>,
}

pub(crate) struct FakeServerStatusHandler {
    config: Arc<Mutex<Config>>,
    send_packet: Option<ResolvedPacket>,
    client_address: Arc<SocketAddr>,
    next_state: Box<Option<ClientState>>,
}

impl FakeServerStatusHandler {
    pub fn new(config: &Arc<Mutex<Config>>, client_address: &Arc<SocketAddr>) -> Self {
        FakeServerStatusHandler {
            config: Arc::clone(config),
            send_packet: None,
            next_state: Box::new(None),
            client_address: Arc::clone(client_address),
        }
    }

    fn stage_packet<T: WritablePacket>(&mut self, packet: T) -> anyhow::Result<()> {
        self.send_packet = Some(packet.to_resolved_packet()?);
        Ok(())
    }

    pub async fn update_client(&mut self, client: &mut Client) -> anyhow::Result<()> {
        if let Some(packet) = &self.send_packet {
            log::trace!(target: &self.client_address.to_string(), "Writing packet {:?} to client handle.", &packet);
            packet.write_async(&mut client.write).await?;
            self.send_packet = None;
        }
        Ok(())
    }

    pub fn next_state(self) -> ClientState {
        self.next_state.unwrap()
    }

    pub fn peek_state(&self) -> bool {
        !matches!(*self.next_state, None)
    }
}

#[async_trait]
impl mc_packet_protocol::registry::StatusServerBoundRegistryHandler for FakeServerStatusHandler {
    async fn handle_default<T: Decodable>(
        &mut self,
        handle: impl LazyHandle<T> + Send + 'async_trait,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_status_request(
        &mut self,
        handle: impl LazyHandle<StatusRequest> + Send + 'async_trait,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()?;
        trace!(target: &self.client_address.to_string(), "Handling status request.");
        let response = {
            let local = self.config.try_lock()?;
            let response = StatusResponseJsonRepresentation {
                version: VersionPart {
                    name: String::from(crate::CURRENT_PROTOCOL.1),
                    protocol: crate::CURRENT_PROTOCOL.0,
                },
                players: PlayersPart {
                    max: local.server_info.max_players,
                    online: 1337,
                    sample: vec![],
                },
                description: DescriptionPart {
                    text: local.server_info.motd.clone(),
                },
                favicon: None,
            };
            drop(local);
            response
        };
        trace!(target: &self.client_address.to_string(), "Responding in turn with json: StatusResponse {}", serde_json::to_string_pretty(&response)?);
        self.stage_packet(StatusResponse {
            json_response: StatusResponseJson::from(serde_json::to_string(&response)?),
        })?;
        drop(response);
        Ok(())
    }

    async fn handle_ping(
        &mut self,
        handle: impl LazyHandle<Ping> + Send + 'async_trait,
    ) -> anyhow::Result<()> {
        let ping = handle.decode_type()?;
        trace!(target: &self.client_address.to_string(), "Handling ping! {:#?}", &ping);
        let pong = Pong {
            payload: ping.payload,
        };
        trace!(target: &self.client_address.to_string(), "Responding in turn: {:#?}", &pong);
        self.stage_packet(pong)?;
        self.next_state = Box::new(Some(ClientState::End));
        Ok(())
    }
}
