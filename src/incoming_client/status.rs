use std::sync::Arc;
use tokio::sync::Mutex;
use mc_packet_protocol::packet::{PacketReadWriteLocker, WritablePacket};
use mc_packet_protocol::protocol_version::*;
use mc_packet_protocol::registry::{*, status::*};
use crate::Config;

#[derive(serde_derive::Serialize)]
struct Version {
    name: String,
    protocol: i32,
}

#[derive(serde_derive::Serialize)]
struct PlayerSample {
    name: String,
    id: uuid::Uuid,
}

#[derive(serde_derive::Serialize)]
struct Players {
    max: usize,
    online: usize,
    sample: Vec<PlayerSample>,
}

#[derive(serde_derive::Serialize)]
struct Description {
    text: String,
}

#[derive(serde_derive::Serialize)]
struct StatusJson {
    version: Version,
    players: Players,
    description: Description,
    #[serde(skip_serializing_if = "Option::is_none")]
    favicon: Option<String>,
}

pub struct ServerBoundStatusHandler<
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
> ServerBoundStatusHandler<R, W> {
    pub fn new(client_handle: Arc<Mutex<super::ClientHandle>>, locker: Arc<PacketReadWriteLocker<R, W>>) -> Self {
        Self { client_handle, locker, next_state: None }
    }

    pub(crate) fn next_state(self) -> super::ClientState<R, W> {
        *self.next_state.unwrap()
    }

    pub(crate) fn peek_state(&self) -> bool {
        self.next_state.is_some()
    }

    async fn get_config(&self) -> Arc<Config> {
        let locked_client_handle = self.client_handle.lock().await;
        let config = Arc::clone(&locked_client_handle.config);
        drop(locked_client_handle);
        config
    }

    async fn get_protocol_version(&self) -> MCProtocol {
        let locked_client_handle = self.client_handle.lock().await;
        let protocol_version = locked_client_handle.protocol_version;
        drop(locked_client_handle);
        protocol_version
    }
}

#[async_trait::async_trait]
impl<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin
> status::server_bound::RegistryHandler for ServerBoundStatusHandler<R, W> {
    async fn handle_default<T: MapDecodable, H: LazyHandle<T> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_status_request<H: LazyHandle<server_bound::StatusRequest> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        handle.consume_bytes()?;

        let protocol = self.get_protocol_version().await;
        let config = self.get_config().await;

        let numbered_protocol: i32 = if let MCProtocol::Illegal(_) = protocol {
            0
        } else {
            protocol.as_i32()
        };

        let json = serde_json::to_string(&StatusJson {
            version: Version {
                name: crate::CURRENT_PROTOCOL.1.to_string(),
                protocol: numbered_protocol,
            },
            players: Players {
                max: config.server_info.max_players,
                online: 1337,
                sample: vec![PlayerSample {
                    name: String::from("TooLegit"),
                    id: uuid::Uuid::new_v4(),
                }],
            },
            description: Description {
                text: config.server_info.motd.clone(),
            },
            favicon: None,
        })?;
        let mut response = client_bound::StatusResponse {
            json_response: client_bound::JSONResponse::from(json),
        }.to_resolved_packet(protocol)?;
        self.locker.send_packet(&mut response).await?;
        Ok(())
    }

    async fn handle_ping<H: LazyHandle<server_bound::Ping> + Send>(&mut self, handle: H) -> anyhow::Result<()> {
        let ping = handle.decode_type()?;

        let mut pong = client_bound::Pong {
            payload: ping.payload,
        }.to_resolved_packet(self.get_protocol_version().await)?;

        self.locker.send_packet(&mut pong).await?;
        self.next_state = Some(Box::new(super::ClientState::End));

        Ok(())
    }
}
