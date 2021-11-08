mod initializer;
mod mediator;
mod play;
mod server_handle;
pub(crate) mod plugin_message_util;

use crate::authenticated_client::mediator::{ClientHandle, Mediator};
use crate::authenticated_client::server_handle::ServerHandle;
use crate::config::Config;
use crate::mc_types::GameProfile;
use crate::players::PlayerList;
use futures_lite::FutureExt;
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

pub(crate) struct PlayerInfo {
    profile: Arc<GameProfile>,
    protocol_version: MCProtocol,
    config: Arc<Config>,
    address: Arc<SocketAddr>,
}

pub(crate) async fn transfer_client<R: MovableAsyncRead, W: MovableAsyncWrite>(
    transfer_info: TransferInfo<R, W>,
    players: Arc<PlayerList>,
) -> anyhow::Result<()> {
    let arc_profile = Arc::new(transfer_info.profile);

    let (client_to_proxy, read_from_client) = flume::unbounded::<std::io::Cursor<Vec<u8>>>(); // channel for client
    let (server_to_proxy, read_from_server) = flume::unbounded::<std::io::Cursor<Vec<u8>>>(); // channel for server

    let mut server_handle = ServerHandle::new(
        PlayerInfo {
            profile: Arc::clone(&arc_profile),
            protocol_version: transfer_info.protocol_version,
            config: Arc::clone(&transfer_info.config),
            address: Arc::clone(&transfer_info.address),
        },
        server_to_proxy,
    );
    server_handle.init().await?;

    let (client_bound_sender, client_read, client_write) = packet::spin(
        "client".into(),
        Arc::clone(&transfer_info.read_write_locker),
        client_to_proxy,
    );

    let client_handle: ClientHandle = (
        client_bound_sender,
        tokio::task::spawn(async move {
            let profile = Arc::clone(&arc_profile);
            if let Err(err) = client_read.race(client_write).await {
                println!(
                    "Error encountered with connection {}: {:?}",
                    profile.name, err
                )
            }
            players.size.fetch_sub(1, Ordering::SeqCst);
        }),
    );

    Mediator::new(
        server_handle,
        read_from_server,
        client_handle,
        read_from_client,
        transfer_info.protocol_version,
    )
    .mediate();
    Ok(())
}
