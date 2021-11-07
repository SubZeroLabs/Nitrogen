use mc_packet_protocol::packet::ResolvedPacket;

pub struct ServerHandle {
    server_to_proxy: flume::Sender<ResolvedPacket>, // send packets to the proxy to handle
}
