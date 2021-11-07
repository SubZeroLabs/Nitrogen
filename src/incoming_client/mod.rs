mod handshaking;
mod login;
mod status;

use crate::players::PlayerList;
use crate::Config;
use mc_packet_protocol::packet::{
    MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker, PacketReader, PacketWriter,
    WritablePacket,
};
use mc_packet_protocol::protocol_version::MCProtocol;
use mc_packet_protocol::registry::{
    handshake, login as registry_login, status as registry_status, RegistryBase,
};
use minecraft_data_types::common::Chat;
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

pub(crate) enum ClientState<R: MovableAsyncRead, W: MovableAsyncWrite> {
    Handshaking(handshaking::ServerBoundHandshakeHandler<R, W>),
    Status(status::ServerBoundStatusHandler<R, W>),
    Login(login::ServerBoundLoginHandler<R, W>),
    Transfer(crate::authenticated_client::TransferInfo<R, W>),
    End,
}

impl<R: MovableAsyncRead, W: MovableAsyncWrite> ClientState<R, W> {
    pub fn peek_state(&self) -> bool {
        match self {
            ClientState::Handshaking(handler) => handler.peek_state(),
            ClientState::Status(handler) => handler.peek_state(),
            ClientState::Login(handler) => handler.peek_state(),
            _ => unreachable!(),
        }
    }

    pub fn next_state(self) -> ClientState<R, W> {
        match self {
            ClientState::Handshaking(handler) => handler.next_state(),
            ClientState::Status(handler) => handler.next_state(),
            ClientState::Login(handler) => handler.next_state(),
            _ => unreachable!(),
        }
    }

    pub async fn handle_packet(
        &mut self,
        packet: Cursor<Vec<u8>>,
        protocol: MCProtocol,
    ) -> anyhow::Result<()> {
        match self {
            ClientState::Handshaking(handler) => {
                handshake::server_bound::Registry::handle_packet(handler, packet, protocol).await
            }
            ClientState::Status(handler) => {
                registry_status::server_bound::Registry::handle_packet(handler, packet, protocol)
                    .await
            }
            ClientState::Login(handler) => {
                registry_login::server_bound::Registry::handle_packet(handler, packet, protocol)
                    .await
            }
            _ => unreachable!(),
        }
    }

    pub fn is_login(&self) -> bool {
        matches!(self, ClientState::Login(_))
    }

    pub fn is_end(&self) -> bool {
        matches!(self, ClientState::End)
    }
}

impl<R: MovableAsyncRead, W: MovableAsyncWrite> Display for &ClientState<R, W> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientState::Handshaking(_) => f.write_str("Handshaking"),
            ClientState::Status(_) => f.write_str("Status"),
            ClientState::Login(_) => f.write_str("Login"),
            ClientState::Transfer(_) => f.write_str("Transfer"),
            ClientState::End => f.write_str("End"),
        }
    }
}

pub struct ClientHandle {
    protocol_version: MCProtocol,
    config: Arc<Config>,
    address: Arc<SocketAddr>,
    players: Arc<PlayerList>,
}

impl ClientHandle {
    pub(crate) fn new(
        config: Arc<Config>,
        address: Arc<SocketAddr>,
        players: Arc<PlayerList>,
    ) -> Self {
        Self {
            protocol_version: MCProtocol::Undefined,
            config,
            address,
            players,
        }
    }

    pub fn update_protocol(&mut self, protocol: MCProtocol) {
        self.protocol_version = protocol;
    }
}

async fn disconnect_client<R: MovableAsyncRead, W: MovableAsyncWrite>(
    locker: Arc<PacketReadWriteLocker<R, W>>,
    reason: String,
    protocol: MCProtocol,
) -> anyhow::Result<()> {
    let mut disconnect = registry_login::client_bound::Disconnect {
        reason: Chat::from(format!(
            r#"{}
                            "text": {:?},
                            "color": "red"
                        {}"#,
            '{',
            reason.replace("\"", "'"),
            '}'
        )),
    }
    .to_resolved_packet(protocol)?;
    let mut locked_write = locker.lock_writer().await;
    locked_write.send_resolved_packet(&mut disconnect).await?;
    drop(locked_write);
    Ok(())
}

pub(crate) async fn accept_client(
    client: TcpStream,
    address: Arc<SocketAddr>,
    config: Arc<Config>,
    players: Arc<PlayerList>,
) -> anyhow::Result<()> {
    log::trace!(target: &address.to_string(), "Handling new client.");
    let (read, write) = client.into_split();

    let client_handle = Arc::new(Mutex::new(ClientHandle::new(
        config,
        Arc::clone(&address),
        players,
    )));
    let locker = Arc::new(PacketReadWriteLocker::new(
        Arc::new(Mutex::new(PacketWriter::new(write))),
        Arc::new(Mutex::new(PacketReader::new(read, Arc::clone(&address)))),
    ));

    let mut client_state = ClientState::Handshaking(handshaking::ServerBoundHandshakeHandler::new(
        Arc::clone(&client_handle),
        Arc::clone(&locker),
    ));

    loop {
        let locked_client_handle = client_handle.lock().await;
        let protocol = locked_client_handle.protocol_version;
        drop(locked_client_handle);

        if let Err(err) = next(&mut client_state, Arc::clone(&locker), protocol).await {
            return disconnect_client(
                Arc::clone(&locker),
                format!("Encountered Error: {:?}", err),
                protocol,
            )
            .await;
        }

        if client_state.peek_state() {
            client_state = client_state.next_state();

            let locked_client_handle = client_handle.lock().await;
            let protocol = locked_client_handle.protocol_version;
            drop(locked_client_handle);
            if let MCProtocol::Illegal(_) = protocol {
                if client_state.is_login() {
                    let mut writer = locker.lock_writer().await;
                    let disconnect = registry_login::client_bound::Disconnect {
                        reason: format!(r#"{}
                            "text": "Your current client version ({}) is not supported by the server.",
                            "color": "red"
                        {}"#, '{', protocol, '}').into(),
                    };
                    let mut resolved = disconnect.to_resolved_packet(protocol)?;
                    writer.send_resolved_packet(&mut resolved).await?;
                    drop(writer);
                    return Ok(());
                }
            } else if client_state.is_end() {
                return Ok(());
            } else if let ClientState::Transfer(transfer_info) = client_state {
                crate::authenticated_client::transfer_client(transfer_info);
                return Ok(());
            }
        }
    }
}

async fn next<R: MovableAsyncRead, W: MovableAsyncWrite>(
    client_state: &mut ClientState<R, W>,
    locker: Arc<PacketReadWriteLocker<R, W>>,
    protocol: MCProtocol,
) -> anyhow::Result<()> {
    let mut locked_read = locker.lock_reader().await;
    let next_packet = locked_read.next_packet().await?;
    drop(locked_read);

    client_state.handle_packet(next_packet, protocol).await?;
    Ok(())
}
