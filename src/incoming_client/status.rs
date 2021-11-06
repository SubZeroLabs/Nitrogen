use crate::Config;
use mc_packet_protocol::packet::{PacketReadWriteLocker, WritablePacket};
use mc_packet_protocol::protocol_version::*;
use mc_packet_protocol::registry::{status::*, *};
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use crate::players::{Player, PlayerList};
use std::sync::atomic::Ordering;

#[derive(serde_derive::Serialize)]
struct Version {
    name: String,
    protocol: i32,
}

#[derive(serde_derive::Serialize)]
struct Players<'ser> {
    max: usize,
    online: usize,
    sample: Vec<&'ser Player>,
}

impl<'ser> Players<'ser> {
    async fn new(config: &Config, player_list: &'ser PlayerList, players_vec: &'ser MutexGuard<'_, Vec<Player>>) -> Players<'ser> {
        let (players, max_players) = match config.players {
            crate::config::Players::Moving => {
                let len = player_list.size.load(Ordering::Relaxed);
                (len, len + 1)
            }
            crate::config::Players::Strict { max_players } => {
                (player_list.size.load(Ordering::Relaxed), max_players)
            }
            crate::config::Players::Constant { players, max_players, .. } => {
                (players, max_players)
            }
        };
        Players {
            max: max_players,
            online: players,
            sample: players_vec.iter().take(5).collect(),
        }
    }
}

#[derive(serde_derive::Serialize)]
struct Description {
    text: String,
}

#[derive(serde_derive::Serialize)]
struct StatusJson<'ser> {
    version: Version,
    players: Players<'ser>,
    description: Description,
    #[serde(skip_serializing_if = "Option::is_none")]
    favicon: Option<String>,
}

pub struct ServerBoundStatusHandler<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin,
> {
    client_handle: Arc<Mutex<super::ClientHandle>>,
    locker: Arc<PacketReadWriteLocker<R, W>>,
    next_state: Option<Box<super::ClientState<R, W>>>,
}

impl<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin,
> ServerBoundStatusHandler<R, W>
{
    pub fn new(
        client_handle: Arc<Mutex<super::ClientHandle>>,
        locker: Arc<PacketReadWriteLocker<R, W>>,
    ) -> Self {
        Self {
            client_handle,
            locker,
            next_state: None,
        }
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

    async fn get_player_list(&self) -> Arc<PlayerList> {
        let locked_client_handle = self.client_handle.lock().await;
        let player_list = Arc::clone(&locked_client_handle.players);
        drop(locked_client_handle);
        player_list
    }
}

#[async_trait::async_trait]
impl<
    R: tokio::io::AsyncRead + Send + Sync + Sized + Unpin,
    W: tokio::io::AsyncWrite + Send + Sync + Sized + Unpin,
> status::server_bound::RegistryHandler for ServerBoundStatusHandler<R, W>
{
    async fn handle_default<T: MapDecodable, H: LazyHandle<T> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_status_request<H: LazyHandle<server_bound::StatusRequest> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()?;

        let protocol = self.get_protocol_version().await;
        let config = self.get_config().await;

        let numbered_protocol: i32 = if let MCProtocol::Illegal(_) = protocol {
            0
        } else {
            protocol.as_i32()
        };

        let player_list = self.get_player_list().await;
        let player_vec_lock = player_list.players.lock().await;
        let json = serde_json::to_string(&StatusJson {
            version: Version {
                name: crate::CURRENT_PROTOCOL.1.to_string(),
                protocol: numbered_protocol,
            },
            players: Players::new(&config, &player_list, &player_vec_lock).await,
            description: Description {
                text: config.server_info.motd.clone(),
            },
            favicon: None,
        })?;
        drop(player_vec_lock);
        let mut response = client_bound::StatusResponse {
            json_response: client_bound::JSONResponse::from(json),
        }
            .to_resolved_packet(protocol)?;
        self.locker.send_packet(&mut response).await?;
        Ok(())
    }

    async fn handle_ping<H: LazyHandle<server_bound::Ping> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let ping = handle.decode_type()?;

        let mut pong = client_bound::Pong {
            payload: ping.payload,
        }
            .to_resolved_packet(self.get_protocol_version().await)?;

        self.locker.send_packet(&mut pong).await?;
        self.next_state = Some(Box::new(super::ClientState::End));

        Ok(())
    }
}
