use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;

use log::error;
use mc_packet_protocol::buffer::BufferState;
use mc_packet_protocol::registry::{HandshakingServerBoundRegistry, StatusServerBoundRegistry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::Config;
use std::io::Cursor;
use tokio::time::{timeout_at, Instant, Duration};

pub mod handshaking;
// pub mod login;
pub mod status;

pub(crate) enum ClientState {
    Handshaking(handshaking::FakeServerHandshakingHandler),
    Status(status::FakeServerStatusHandler),
    End,
}

impl ClientState {
    pub fn peek_state(&self) -> bool {
        match self {
            ClientState::Handshaking(handler) => handler.peek_state(),
            ClientState::Status(handler) => handler.peek_state(),
            ClientState::End => false,
        }
    }

    pub fn next_state(self) -> ClientState {
        match self {
            ClientState::Handshaking(handler) => handler.next_state(),
            ClientState::Status(handler) => handler.next_state(),
            ClientState::End => ClientState::End,
        }
    }

    pub async fn update_client(&mut self, client: &mut Client) -> anyhow::Result<()> {
        match self {
            ClientState::Status(handler) => handler.update_client(client).await,
            _ => Ok(()),
        }
    }

    pub async fn handle_packet(&mut self, packet: Cursor<Vec<u8>>) -> anyhow::Result<()> {
        match self {
            ClientState::Handshaking(handler) => {
                HandshakingServerBoundRegistry::read_packet(handler, packet)
                    .await
            }
            ClientState::Status(handler) => {
                StatusServerBoundRegistry::read_packet(handler, packet)
                    .await
            }
            ClientState::End => Ok(()),
        }
    }
}

impl Display for &ClientState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientState::Handshaking(_) => f.write_str("Handshaking"),
            ClientState::Status(_) => f.write_str("Status"),
            ClientState::End => f.write_str("End"),
        }
    }
}

pub(crate) struct Client {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
    address: Arc<SocketAddr>,
    buffer: mc_packet_protocol::buffer::MinecraftPacketBuffer,
    config: Arc<Mutex<Config>>,
}

impl Client {
    async fn read_buf(&mut self) -> anyhow::Result<()> {
        let mut buf = self.buffer.inner_buf();
        self.read.read_buf(&mut buf).await?;
        Ok(())
    }
}

async fn legacy_handle_mc_ping(config: &Arc<Mutex<Config>>) -> anyhow::Result<Vec<u8>> {
    let bind = Arc::clone(config);
    let locked = bind.try_lock()?;
    let string = format!(
        "\u{a7}1\u{0}{}\u{0}{}\u{0}{}\u{0}0\u{0}{}",
        crate::CURRENT_PROTOCOL.0,
        crate::CURRENT_PROTOCOL.1,
        locked.server_info.motd.clone(),
        locked.server_info.max_players
    );
    drop(locked);
    let length = string.len() as u16;
    let length_bytes = length.to_be_bytes();
    let bytes = [
        vec![0xff],
        vec![length_bytes[0], length_bytes[1]],
        string
            .encode_utf16()
            .into_iter()
            .map(|u16| {
                let bytes = u16.to_be_bytes();
                vec![bytes[0], bytes[1]]
            })
            .collect::<Vec<Vec<u8>>>()
            .concat(),
    ]
        .concat();
    Ok(bytes)
}

pub async fn new_client(socket: TcpStream, address: SocketAddr, config: Arc<Mutex<Config>>) {
    log::debug!("Accepting new potential client {}", address.to_string());

    let (owned_read, owned_write) = socket.into_split();

    let address_arc = Arc::new(address);

    let mut client_state = ClientState::Handshaking(
        handshaking::FakeServerHandshakingHandler::new(&config, &address_arc),
    );

    let mut client = Client {
        read: owned_read,
        write: owned_write,
        address: address_arc,
        buffer: mc_packet_protocol::buffer::MinecraftPacketBuffer::new(),
        config,
    };

    loop {
        let (encoded, decoded) = client.buffer.len();

        match client.buffer.poll() {
            BufferState::Waiting => {
                match (&client_state, client.buffer.len().1, client.buffer.inner_buf()) {
                    (ClientState::Handshaking(_), decode_len, buf) if decode_len > 0 && buf[0] == 0xFE => {
                        // check for server list ping packet, because mc clients are dumb
                        // we need to handle this stupid dumb packet
                        match legacy_handle_mc_ping(&client.config).await {
                            Ok(response) => {
                                if let Err(err) = client.write.write_all(&response).await {
                                    error!(target: &client.address.to_string(), "Error writing response to client: {:?}", err);
                                    break;
                                }
                                break; // server is always done responding after this
                            }
                            Err(err) => {
                                error!(target: &client.address.to_string(), "Error resolved server list ping: {:?}", err);
                                break;
                            }
                        }
                    }
                    _ => {
                        log::trace!(target: &client.address.to_string(), "Buf read awaiting packet: Encoded {}, Decoded: {}", encoded, decoded);
                        if let Err(err) = timeout_at(Instant::now() + Duration::from_secs(10), client.read_buf()).await {
                            let len = {
                                client.buffer.len()
                            };
                            log::trace!(target: &client.address.to_string(), "Failed read with buffer: {:?}, {:?}", client.buffer.inner_buf(), len);
                            error!(target: &client.address.to_string(), "Error occurred reading buffer: {:?}", err);
                            break;
                        }
                    }
                }
            }
            BufferState::PacketReady => {
                match client.buffer.packet_reader() {
                    Ok(reader) => {
                        if let Err(err) = client_state.handle_packet(reader).await {
                            error!(target: &client.address.to_string(), "Error handling packet: {:?}", err);
                            break;
                        }
                    }
                    Err(err) => {
                        error!(target: &client.address.to_string(), "Error setting up packet reader: {:?}", err);
                        break;
                    }
                }
            }
            BufferState::Error(err) => {
                error!(target: &client.address.to_string(), "Error occurred polling buffer: {:?}", err);
                break;
            }
        }

        if let Err(err) = client_state.update_client(&mut client).await {
            error!(target: &client.address.to_string(), "Error occurred updating client: {:?}", err);
            break;
        }

        if client_state.peek_state() {
            client_state = client_state.next_state();
            log::trace!(target: &client.address.to_string(), "Pushing client state to: {}", &client_state);
        }

        if let ClientState::End = client_state {
            break;
        }
    }
}
