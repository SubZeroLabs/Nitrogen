mod to_client;

use crate::config::Config;
use crate::mc_types::GameProfile;
use mc_packet_protocol::packet::{MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker};
use mc_packet_protocol::protocol_version::MCProtocol;
use std::net::SocketAddr;
use std::sync::Arc;

pub(crate) struct TransferInfo<R: MovableAsyncRead, W: MovableAsyncWrite> {
    pub(crate) profile: GameProfile,
    pub(crate) protocol_version: MCProtocol,
    pub(crate) config: Arc<Config>,
    pub(crate) address: Arc<SocketAddr>,
    pub(crate) read_write_locker: Arc<PacketReadWriteLocker<R, W>>,
}

pub(crate) fn transfer_client<R: MovableAsyncRead, W: MovableAsyncWrite>(
    transfer_info: TransferInfo<R, W>,
) {
    // setup call to dynamic backend and create a new read write locker

    // hold client in place while we setup a backend connector

    // authenticate with backend

    // find a way to transfer from client -> server, possibly with channels (flume maybe)

    // dispose of any flawed packets
}
