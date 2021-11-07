use crate::authenticated_client::PlayerInfo;
use crate::config::{Server, ForwardingMode};
use anyhow::Context;
use mc_packet_protocol::packet::{
    MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker, PacketReader, PacketWriter,
    WritablePacket,
};
use mc_packet_protocol::protocol_version::MCProtocol;
use mc_packet_protocol::registry;
use mc_packet_protocol::registry::handshake::server_bound::{NextState, ServerAddress};
use mc_packet_protocol::registry::login::LoginName;
use mc_packet_protocol::registry::RegistryBase;
use minecraft_data_types::nums::VarInt;
use std::sync::Arc;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use crate::mc_types::GameProfile;
use std::net::SocketAddr;
use std::borrow::Borrow;

mod login;

fn create_address(address: String, user_address: &SocketAddr, forwarding: &ForwardingMode, profile: &GameProfile) -> anyhow::Result<ServerAddress> {
    if let ForwardingMode::Legacy = forwarding {
        let mut str = address;
        str.push('\u{0}');
        str.push_str(&user_address.to_string());
        str.push('\u{0}');
        str.push_str(&profile.id.to_simple_ref().to_string());
        str.push('\u{0}');
        str.push_str(&serde_json::to_string::<GameProfile>(profile.borrow())?);
        Ok(ServerAddress::from(str))
    } else {
        Ok(ServerAddress::from(address))
    }
}

fn initiate_login(
    server: Server,
    player_info: Arc<PlayerInfo>,
) -> JoinHandle<anyhow::Result<Arc<PacketReadWriteLocker<OwnedReadHalf, OwnedWriteHalf>>>> {
    tokio::task::spawn(async move {
        let stream = TcpStream::connect(format!("{}:{}", server.ip, server.port)).await?;
        let (read, write) = stream.into_split();

        let locker = Arc::new(PacketReadWriteLocker::new(
            Arc::new(Mutex::new(PacketWriter::new(write))),
            Arc::new(Mutex::new(PacketReader::new(
                read,
                Arc::clone(&player_info.address),
            ))),
        ));

        // send initial handshake
        let mut writer_lock = locker.lock_writer().await;
        writer_lock
            .send_resolved_packet(
                &mut registry::handshake::server_bound::Handshake {
                    protocol_version: VarInt::from(player_info.protocol_version.as_i32()),
                    server_address: create_address(server.ip, player_info.address.borrow(), &server.forwarding_mode, player_info.profile.borrow())?,
                    server_port: server.port,
                    next_state: NextState::Login,
                }
                    .to_resolved_packet(player_info.protocol_version)?,
            )
            .await?;
        writer_lock
            .send_resolved_packet(
                &mut registry::login::server_bound::LoginStart {
                    name: LoginName::from(&*player_info.profile.name),
                }
                    .to_resolved_packet(player_info.protocol_version)?,
            )
            .await?;
        drop(writer_lock);

        // continue negotiation
        let mut handler = login::LoginClientBoundHandler::new(
            Arc::clone(&locker),
            player_info.protocol_version,
            Arc::clone(&player_info.address),
            Arc::clone(&player_info.profile),
            server.forwarding_mode.clone(),
        );

        while !handler.finished {
            let mut read_lock = locker.lock_reader().await;
            let cursor = read_lock.next_packet().await?;
            drop(read_lock);
            mc_packet_protocol::registry::login::client_bound::Registry::handle_packet(
                &mut handler,
                cursor,
                player_info.protocol_version,
            )
                .await?;
        }

        Ok(Arc::clone(&locker))
    })
}
