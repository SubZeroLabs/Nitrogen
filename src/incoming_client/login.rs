use crate::authenticated_client::TransferInfo;
use crate::config::Players;
use crate::players::PlayerList;
use crate::Config;
use mc_packet_protocol::packet::{
    MovableAsyncRead, MovableAsyncWrite, PacketReadWriteLocker, WritablePacket,
};
use mc_packet_protocol::protocol_version::*;
use mc_packet_protocol::registry::{login::*, *};
use md5::Digest;
use minecraft_data_types::common::Chat;
use num_bigint::BigInt;
use reqwest::StatusCode;
use rsa::PaddingScheme;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::Mutex;

fn hash_server_id(server_id: &str, shared_secret: &[u8], public_key: &[u8]) -> String {
    let mut hasher = sha1::Sha1::new();
    hasher.update(server_id);
    hasher.update(shared_secret);
    hasher.update(public_key);
    digest(hasher.finalize().as_slice())
}

fn digest(bytes: &[u8]) -> String {
    let bigint = BigInt::from_signed_bytes_be(bytes);
    format!("{:x}", bigint)
}

#[derive(Default)]
pub struct LoginData {
    username: Option<String>,
    keys: Option<(rsa::RsaPrivateKey, Vec<u8>)>,
    verify: Option<Vec<u8>>,
    server_id: Option<String>,
}

pub struct ServerBoundLoginHandler<R: MovableAsyncRead, W: MovableAsyncWrite> {
    client_handle: Arc<Mutex<super::ClientHandle>>,
    locker: Arc<PacketReadWriteLocker<R, W>>,
    next_state: Option<Box<super::ClientState<R, W>>>,
    login_data: Box<LoginData>,
}

impl<R: MovableAsyncRead, W: MovableAsyncWrite> ServerBoundLoginHandler<R, W> {
    pub fn new(
        client_handle: Arc<Mutex<super::ClientHandle>>,
        locker: Arc<PacketReadWriteLocker<R, W>>,
    ) -> Self {
        Self {
            client_handle,
            locker,
            next_state: None,
            login_data: Box::new(LoginData::default()),
        }
    }

    pub(crate) fn next_state(self) -> super::ClientState<R, W> {
        *self.next_state.unwrap()
    }

    pub(crate) fn peek_state(&self) -> bool {
        self.next_state.is_some()
    }

    async fn disconnect_client(&mut self, reason: String) -> anyhow::Result<()> {
        let mut disconnect = client_bound::Disconnect {
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
        .to_resolved_packet(self.get_protocol_version().await)?;

        self.locker.send_packet(&mut disconnect).await?;
        self.next_state = Some(Box::new(super::ClientState::End));
        Ok(())
    }

    async fn address_string(&self) -> String {
        let locked_client_handle = self.client_handle.lock().await;
        let address_str = locked_client_handle.address.to_string();
        drop(locked_client_handle);
        address_str
    }

    async fn get_address(&self) -> Arc<SocketAddr> {
        let locked_client_handle = self.client_handle.lock().await;
        let address = Arc::clone(&locked_client_handle.address);
        drop(locked_client_handle);
        address
    }

    async fn get_config(&self) -> Arc<Config> {
        let locked_client_handle = self.client_handle.lock().await;
        let config = Arc::clone(&locked_client_handle.config);
        drop(locked_client_handle);
        config
    }

    async fn get_protocol_version(&self) -> MCProtocol {
        let locked_client_handle = self.client_handle.lock().await;
        let protocol_version = locked_client_handle.protocol_version;
        drop(locked_client_handle);
        protocol_version
    }

    async fn get_player_list(&self) -> Arc<PlayerList> {
        let locked_client_handle = self.client_handle.lock().await;
        let player_list = Arc::clone(&locked_client_handle.players);
        drop(locked_client_handle);
        player_list
    }
}

#[async_trait::async_trait]
impl<R: MovableAsyncRead, W: MovableAsyncWrite> login::server_bound::RegistryHandler
    for ServerBoundLoginHandler<R, W>
{
    async fn handle_unknown(&mut self, _: std::io::Cursor<Vec<u8>>) -> anyhow::Result<()> {
        anyhow::bail!("Unknown login packet found.");
    }

    async fn handle_default<T: MapDecodable, H: LazyHandle<T> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        handle.consume_bytes()
    }

    async fn handle_login_start<H: LazyHandle<server_bound::LoginStart> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let config = self.get_config().await;
        let player_list = self.get_player_list().await;
        match config.players {
            Players::Strict { max_players }
            | Players::Constant {
                max_players,
                fail_on_over_join: true,
                ..
            } if player_list.size.load(Ordering::SeqCst) >= max_players => {
                return self
                    .disconnect_client("The server is currently full.".to_string())
                    .await;
            }
            _ => (),
        }

        let login_start = handle.decode_type()?;
        {
            let target = &self.address_string().await;
            log::info!(target: target, "Logging in {:?}", &login_start.name);
        }
        self.login_data.username = Some(login_start.name.into());

        let (private, _, packet) = client_bound::EncryptionRequest::new()?;

        let mut resolved = packet.to_resolved_packet(self.get_protocol_version().await)?;

        self.locker.send_packet(&mut resolved).await?;
        self.login_data.keys = Some((private, packet.public_key.1));
        self.login_data.verify = Some(packet.verify_token.1);
        self.login_data.server_id = Some(packet.server_id.into());
        Ok(())
    }

    async fn handle_encryption_response<H: LazyHandle<server_bound::EncryptionResponse> + Send>(
        &mut self,
        handle: H,
    ) -> anyhow::Result<()> {
        let response = handle.decode_type()?;
        let (private, public) = self.login_data.keys.as_ref().unwrap();
        let decoded_shared_secret = rsa::RsaPrivateKey::decrypt(
            private,
            PaddingScheme::PKCS1v15Encrypt,
            &response.shared_secret.1,
        )?;
        let decoded_verify = rsa::RsaPrivateKey::decrypt(
            private,
            PaddingScheme::PKCS1v15Encrypt,
            &response.verify_token.1,
        )?;

        // enable encryption
        match mc_packet_protocol::encryption::Codec::from_response(
            &decoded_verify,
            &decoded_shared_secret,
            self.login_data.verify.as_ref().unwrap(),
        ) {
            Ok((codec_read, codec_write)) => {
                let (mut lock_read, mut lock_write) = (
                    self.locker.lock_reader().await,
                    self.locker.lock_writer().await,
                );
                lock_read.enable_decryption(codec_read);
                lock_write.enable_encryption(codec_write);
                drop(lock_read);
                drop(lock_write);
            }
            Err(err) => {
                self.disconnect_client(format!("{:?}", err)).await?;
                return Ok(());
            }
        }

        // authenticate with mojang
        let hash = hash_server_id(
            self.login_data.server_id.as_ref().unwrap(),
            &decoded_shared_secret,
            public,
        );

        // optimize later
        let session_server: String = if let Ok(var) = std::env::var("NITROGEN_SESSION_SERVER") {
            var
        } else {
            String::from("https://sessionserver.mojang.com/session/minecraft/hasJoined")
        };
        let url = format!(
            "{}?username={}&serverId={}",
            session_server,
            self.login_data.username.as_ref().unwrap(),
            hash
        );
        let response = reqwest::get(url).await?;
        if response.status() == StatusCode::from_u16(204)? {
            let target = &self.address_string().await;
            log::debug!(target: target, "Received 204 status code for client");
            self.disconnect_client(String::from("Failed to authenticate with Mojang."))
                .await?;
            return Ok(());
        } else if response.status() != StatusCode::from_u16(200)? {
            let target = &self.address_string().await;
            log::debug!(
                target: target,
                "Received code {}({}) for call.",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("Unknown")
            );
            self.disconnect_client(format!(
                "Received {}({}) from Mojang authentication.",
                response.status().as_u16(),
                response.status().canonical_reason().unwrap_or("Unknown")
            ))
            .await?;
            return Ok(());
        }

        let game_profile = response.json::<crate::mc_types::GameProfile>().await?;

        let config = self.get_config().await;
        let compression_threshold = config.network.compression_threshold;
        if compression_threshold != -1 {
            let mut compression_packet = client_bound::SetCompression {
                threshold: minecraft_data_types::nums::VarInt::from(compression_threshold),
            }
            .to_resolved_packet(self.get_protocol_version().await)?;
            self.locker.send_packet(&mut compression_packet).await?;

            let (mut lock_read, mut lock_write) = (
                self.locker.lock_reader().await,
                self.locker.lock_writer().await,
            );
            lock_read.enable_decompression();
            lock_write.enable_compression(compression_threshold);
            drop(lock_read);
            drop(lock_write);
        }

        self.next_state = Some(Box::new(super::ClientState::Transfer(TransferInfo {
            profile: game_profile,
            protocol_version: self.get_protocol_version().await,
            config: Arc::clone(&config),
            address: self.get_address().await,
            read_write_locker: Arc::clone(&self.locker),
        })));
        Ok(())
    }
}
