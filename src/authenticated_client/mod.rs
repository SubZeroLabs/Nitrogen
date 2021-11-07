mod initializer;
mod mediator;
mod play;
mod server_handle;

use crate::config::Config;
use crate::mc_types::GameProfile;
use crate::players::PlayerList;
use futures_lite::FutureExt;
use mc_packet_protocol::packet::ResolvedPacket;
use mc_packet_protocol::protocol_version::MCProtocol;
use mc_packet_protocol::{
    packet,
    packet::{MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker},
};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(crate) struct TransferInfo<R: MovableAsyncRead, W: MovableAsyncWrite> {
    pub(crate) profile: GameProfile,
    pub(crate) protocol_version: MCProtocol,
    pub(crate) config: Arc<Config>,
    pub(crate) address: Arc<SocketAddr>,
    pub(crate) read_write_locker: Arc<PacketReadWriteLocker<R, W>>,
}

struct PlayerInfo {
    profile: Arc<GameProfile>,
    protocol_version: MCProtocol,
    config: Arc<Config>,
    address: Arc<SocketAddr>,
}

pub(crate) fn transfer_client<R: MovableAsyncRead, W: MovableAsyncWrite>(
    transfer_info: TransferInfo<R, W>,
    players: Arc<PlayerList>,
) {
    let arc_profile = Arc::new(transfer_info.profile);

    let (client_to_proxy, read_from_client) = flume::unbounded::<std::io::Cursor<Vec<u8>>>(); // channel for client
    let (server_to_proxy, read_from_server) = flume::unbounded::<std::io::Cursor<Vec<u8>>>(); // channel for server

    let (client_bound_sender, client_read, client_write) = packet::spin(
        Arc::clone(&transfer_info.read_write_locker),
        client_to_proxy,
    ); // spin client unconditionally

    tokio::task::spawn(async move {
        let profile = Arc::clone(&arc_profile);
        if let Err(err) = client_read.race(client_write).await {
            println!(
                "Error encountered with connection {}: {:?}",
                profile.name, err
            )
        }
        players.size.fetch_sub(1, Ordering::SeqCst);
    });
}
