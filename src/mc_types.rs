use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct Property {
    pub(crate) name: String,
    pub(crate) value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signature: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Serialize, Debug)]
pub(crate) struct GameProfile {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) properties: Vec<Property>,
}
