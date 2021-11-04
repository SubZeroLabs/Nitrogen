use serde_derive::Deserialize;
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub(crate) struct Property {
    name: String,
    value: String,
    signature: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub(crate) struct GameProfile {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) properties: Vec<Property>,
}
