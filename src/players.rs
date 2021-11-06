use uuid::Uuid;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Player {
    pub(crate) name: String,
    pub(crate) id: Uuid,
}

pub(crate) struct PlayerList {
    pub(crate) size: Arc<AtomicUsize>,
    pub(crate) players: Arc<Mutex<Vec<Player>>>,
}