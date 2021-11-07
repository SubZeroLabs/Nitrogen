use std::sync::Arc;

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub(crate) struct Network {
    pub(crate) bind: String,
    pub(crate) port: u16,
    pub(crate) compression_threshold: i32,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub(crate) struct ServerInfo {
    pub(crate) motd: String,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
#[serde(tag = "type")]
pub(crate) enum Players {
    Moving,
    Strict {
        max_players: usize,
    },
    Constant {
        players: usize,
        max_players: usize,
        fail_on_over_join: bool,
    },
}

#[derive(serde_derive::Deserialize, std::fmt::Debug, Clone)]
#[serde(tag = "type")]
pub(crate) enum ForwardingMode {
    Velocity { forwarding_secret: String },
    Legacy,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug, Clone)]
pub(crate) struct Server {
    pub(crate) name: String,
    pub(crate) ip: String,
    pub(crate) port: u16,
    pub(crate) forwarding_mode: ForwardingMode,
}

#[derive(serde_derive::Deserialize, std::fmt::Debug)]
pub(crate) struct Config {
    pub(crate) network: Network,
    pub(crate) server_info: ServerInfo,
    pub(crate) players: Players,
    #[serde(alias = "server")]
    pub(crate) servers: Vec<Server>,
}
