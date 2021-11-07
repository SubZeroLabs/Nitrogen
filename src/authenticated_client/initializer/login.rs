use crate::config::ForwardingMode;
use crate::mc_types::GameProfile;
use hmac::Mac;
use mc_packet_protocol::packet::{
    MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker, WritablePacket,
};
use mc_packet_protocol::protocol_version::{MCProtocol, MapDecodable};
use mc_packet_protocol::registry::login::client_bound::*;
use mc_packet_protocol::registry::login::server_bound::LoginPluginResponse;
use mc_packet_protocol::registry::login::LoginName;
use mc_packet_protocol::registry::LazyHandle;
use minecraft_data_types::auto_string;
use minecraft_data_types::encoder::Encodable;
use minecraft_data_types::nums::VarInt;
use minecraft_data_types::strings::McString;
use std::convert::TryFrom;
use std::net::SocketAddr;
use std::sync::Arc;

const VELOCITY_FORWARDING_CHANNEL: &str = "velocity:player_info";
const VELOCITY_FORWARDING_VERSION: i32 = 1;

auto_string!(Address, 255);
auto_string!(RandString, 32767);

pub(crate) struct LoginClientBoundHandler<R: MovableAsyncRead, W: MovableAsyncWrite> {
    locker: Arc<PacketReadWriteLocker<R, W>>,
    protocol: MCProtocol,
    address: Arc<SocketAddr>,
    profile: Arc<GameProfile>,
    forwarding_mode: ForwardingMode,
    pub(crate) finished: bool,
}

impl<R: MovableAsyncRead, W: MovableAsyncWrite> LoginClientBoundHandler<R, W> {
    pub(crate) fn new(
        locker: Arc<PacketReadWriteLocker<R, W>>,
        protocol: MCProtocol,
        address: Arc<SocketAddr>,
        profile: Arc<GameProfile>,
        forwarding_mode: ForwardingMode,
    ) -> Self {
        Self {
            locker,
            protocol,
            address,
            profile,
            forwarding_mode,
            finished: false,
        }
    }
}

#[async_trait::async_trait]
impl<R: MovableAsyncRead, W: MovableAsyncWrite>
    mc_packet_protocol::registry::login::client_bound::RegistryHandler
    for LoginClientBoundHandler<R, W>
{
    async fn handle_unknown(&mut self, _: std::io::Cursor<Vec<u8>>) -> anyhow::Result<()> {
        anyhow::bail!("Failed to understand packet cursor.");
    }

    async fn handle_default<T: MapDecodable, H: LazyHandle<T> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_disconnect<H: LazyHandle<Disconnect> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let disconnect = handle.decode_type()?;
        anyhow::bail!("{}", String::from(disconnect.reason))
    }

    async fn handle_encryption_request<H: LazyHandle<EncryptionRequest> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()?;
        anyhow::bail!("Not expecting an encryption request from server.")
    }

    async fn handle_login_success<H: LazyHandle<LoginSuccess> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let login_success = handle.decode_type()?;

        if login_success.username.string() == &self.profile.name
            && login_success.uuid == self.profile.id
        {
            self.finished = true;
        } else {
            anyhow::bail!("Found invalid profile response for login success.");
        }
        Ok(())
    }

    async fn handle_set_compression<H: LazyHandle<SetCompression> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let set_compression = handle.decode_type()?;
        let (mut read_lock, mut write_lock) = (
            self.locker.lock_reader().await,
            self.locker.lock_writer().await,
        );
        read_lock.enable_decompression();
        drop(read_lock);
        write_lock.enable_compression(set_compression.threshold.into());
        drop(write_lock);
        Ok(())
    }

    //noinspection RsRedundantElse
    async fn handle_login_plugin_request<H: LazyHandle<LoginPluginRequest> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let request = handle.decode_type()?;
        if request.channel.string().eq(VELOCITY_FORWARDING_CHANNEL) {
            if let ForwardingMode::Velocity { forwarding_secret } = &self.forwarding_mode {
                let mut data = Vec::new();

                // encode info
                VarInt::from(VELOCITY_FORWARDING_VERSION).encode(&mut data)?;
                Address::from(self.address.to_string()).encode(&mut data)?;
                self.profile.id.encode(&mut data)?;
                LoginName::from(&*self.profile.name).encode(&mut data)?;

                // encode properties
                VarInt::try_from(self.profile.properties.len())?.encode(&mut data)?;
                for property in &self.profile.properties {
                    RandString::from(&*property.name).encode(&mut data)?;
                    RandString::from(&*property.value).encode(&mut data)?;

                    match &property.signature {
                        Some(signature) => {
                            true.encode(&mut data)?;
                            RandString::from(signature.clone()).encode(&mut data)?;
                        }
                        None => {
                            false.encode(&mut data)?;
                        }
                    }
                }

                // encode forwarding secret
                let mac_bytes = {
                    use hmac::{Hmac, Mac, NewMac};
                    use sha2::Sha256;
                    // Create alias for HMAC-SHA256
                    type HmacSha256 = Hmac<Sha256>;

                    let mut mac = HmacSha256::new_from_slice(forwarding_secret.as_bytes())
                        .expect("HMAC can take key of any size");

                    mac.update(&data);
                    mac.update(b""); // todo ask Tux what the hell to do

                    let result = mac.finalize();
                    result.into_bytes()
                }; // todo idk where this even goes inside the vec

                self.locker
                    .send_packet(
                        &mut LoginPluginResponse {
                            message_id: request.message_id,
                            successful: true,
                            data,
                        }
                        .to_resolved_packet(self.protocol)?,
                    )
                    .await?;
            } else {
                anyhow::bail!("Not expecting Velocity forwarding for server requesting it.");
            }
        } else {
            self.locker
                .send_packet(
                    &mut LoginPluginResponse {
                        message_id: request.message_id,
                        successful: false,
                        data: vec![],
                    }
                    .to_resolved_packet(self.protocol)?,
                )
                .await?;
        }
        Ok(())
    }
}
