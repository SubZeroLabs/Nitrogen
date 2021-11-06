use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Player {
    pub(crate) name: String,
    pub(crate) id: Uuid,
}

pub(crate) struct PlayerList {
    // todo more with this
    pub(crate) size: Arc<AtomicUsize>,
}
