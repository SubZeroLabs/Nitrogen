use mc_packet_protocol::packet::{MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker};

struct ScopedClient<R: MovableAsyncRead, W: MovableAsyncWrite> {
    locker: PacketReadWriteLocker<R, W>,
}

impl<R: MovableAsyncRead, W: MovableAsyncWrite> ScopedClient<R, W> {}
