use std::collections::HashMap;
use std::sync::Arc;

use mc_packet_protocol::packet::{PacketReadWriteLocker, ResolvedPacket};

use crate::authenticated_client::PlayerInfo;
use crate::config::Server;
use anyhow::Context;
use flume::Sender;
use futures_lite::FutureExt;
use mc_packet_protocol::protocol_version::{MCProtocol, MapEncodable};
use minecraft_data_types::common::Chat;
use std::convert::TryInto;
use std::io::Cursor;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::JoinHandle;

type SpinHandle = (Sender<ResolvedPacket>, JoinHandle<anyhow::Result<()>>);

pub struct ServerProcess {
    locker: Arc<PacketReadWriteLocker<OwnedReadHalf, OwnedWriteHalf>>,
    proxy_link: Sender<std::io::Cursor<Vec<u8>>>,
    spin_handle: Option<SpinHandle>,
    protocol: MCProtocol,
}

impl ServerProcess {
    fn new(
        locker: Arc<PacketReadWriteLocker<OwnedReadHalf, OwnedWriteHalf>>,
        proxy_link: Sender<std::io::Cursor<Vec<u8>>>,
        protocol: MCProtocol,
    ) -> Self {
        Self {
            locker,
            proxy_link,
            spin_handle: None,
            protocol,
        }
    }

    fn spin_process(&mut self) -> anyhow::Result<()> {
        if self.spin_handle.is_none() {
            let (sender, read_handle, write_handle) = mc_packet_protocol::packet::spin(
                "server".into(),
                Arc::clone(&self.locker),
                self.proxy_link.clone(),
            );
            let futures_spin = ServerProcess::spin_futures(self, read_handle, write_handle);

            self.spin_handle = Some((sender, futures_spin));
            Ok(())
        } else {
            anyhow::bail!(
                "Found existing spin handle. Please abort and reallocate before spinning."
            );
        }
    }

    fn spin_futures(
        &self,
        read_handle: JoinHandle<anyhow::Result<()>>,
        write_handle: JoinHandle<anyhow::Result<()>>,
    ) -> JoinHandle<anyhow::Result<()>> {
        let protocol = self.protocol;
        let sender = self.proxy_link.clone();
        tokio::task::spawn(async move {
            let disconnect = if let Err(error) = read_handle.race(write_handle).await {
                mc_packet_protocol::registry::play::client_bound::Disconnect {
                    reason: Chat::from(format!("Interrupted by server caused by: {:?}", error)),
                }
            } else {
                mc_packet_protocol::registry::play::client_bound::Disconnect {
                    reason: Chat::from("Connected interrupted by server"),
                }
            };
            let mut disconnect_vec =
                Vec::with_capacity(disconnect.size_mapped(protocol)?.try_into()?);
            disconnect.encode_mapped(protocol, &mut disconnect_vec)?;
            sender.send_async(Cursor::new(disconnect_vec)).await?;
            Ok(())
        })
    }

    fn abort(&self) -> anyhow::Result<()> {
        if let Some(handle) = &self.spin_handle {
            handle.1.abort();
            Ok(())
        } else {
            anyhow::bail!("No spin handle in progress.");
        }
    }

    async fn send_packet(&self, packet: ResolvedPacket) -> anyhow::Result<()> {
        if let Some(handle) = &self.spin_handle {
            handle
                .0
                .send_async(packet)
                .await
                .context("Failed to send packet.")
        } else {
            anyhow::bail!("No spin handle in progress.")
        }
    }
}

pub(crate) struct ServerHandle {
    current_server: Option<ServerProcess>,
    server_map: HashMap<String, Arc<Server>>,
    player_info: Arc<PlayerInfo>,
    proxy_link: Sender<std::io::Cursor<Vec<u8>>>,
}

impl ServerHandle {
    pub(crate) fn new(
        player_info: PlayerInfo,
        proxy_link: Sender<std::io::Cursor<Vec<u8>>>,
    ) -> Self {
        let mut server_map = HashMap::with_capacity(player_info.config.servers.len());
        for server in &player_info.config.servers {
            server_map.insert(server.name.clone(), Arc::clone(server));
        }

        Self {
            current_server: None,
            server_map,
            player_info: Arc::new(player_info),
            proxy_link,
        }
    }

    pub(crate) async fn init(&mut self) -> anyhow::Result<()> {
        let priority_iter = &mut self.player_info.config.server_info.priorities.iter();
        for priority in priority_iter {
            if let Some(server) = self.server_map.get(priority) {
                if let Ok(locker) = &super::initializer::initiate_login(
                    Arc::clone(server),
                    Arc::clone(&self.player_info),
                )
                .await
                {
                    self.current_server = Some(ServerProcess::new(
                        Arc::clone(locker),
                        self.proxy_link.clone(),
                        self.player_info.protocol_version,
                    ));
                    self.spin_server()?;
                    return Ok(());
                }
            } else {
                return Err(anyhow::Error::msg(format!(
                    "Failed to resolve {} as a defined server.",
                    priority
                )));
            }
        }
        Err(anyhow::Error::msg(
            "Failed to connect to any of the servers listed.",
        ))
    }

    pub(crate) async fn new_server(&'static mut self, server_name: &str) -> anyhow::Result<()> {
        if let Some(server) = self.server_map.get(server_name) {
            let inner_server = Arc::clone(server);
            tokio::task::spawn(async move {
                match super::initializer::initiate_login(
                    inner_server,
                    Arc::clone(&self.player_info),
                )
                .await
                {
                    Ok(locker) => {
                        if let Some(current_server) = &self.current_server {
                            current_server.abort()?;
                        }
                        self.current_server = Some(ServerProcess::new(
                            Arc::clone(&locker),
                            self.proxy_link.clone(),
                            self.player_info.protocol_version,
                        ));
                        self.spin_server()?;
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            })
            .await?
        } else {
            Err(anyhow::Error::msg(format!(
                "Could not find server {}.",
                server_name
            )))
        }
    }

    pub(crate) fn send_packet(&self, packet: ResolvedPacket) -> anyhow::Result<()> {
        if let Some(server_process) = &self.current_server {
            if let Some(spin_handle) = &server_process.spin_handle {
                spin_handle.0.send(packet)?;
            }
        }
        Ok(())
    }

    pub(crate) fn spin_server(&mut self) -> anyhow::Result<()> {
        if let Some(server_process) = &mut self.current_server {
            server_process.spin_process()
        } else {
            Ok(())
        }
    }

    pub(crate) fn abort(&self) -> anyhow::Result<()> {
        if let Some(server_process) = &self.current_server {
            server_process.abort()?;
        }
        Ok(())
    }
}
