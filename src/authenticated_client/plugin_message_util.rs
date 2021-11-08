use minecraft_data_types::common::Identifier;
use minecraft_data_types::auto_string;
use minecraft_data_types::strings::McString;
use minecraft_data_types::encoder::{Decodable, Encodable};
use std::convert::TryInto;
use std::io::Cursor;

auto_string!(BigString, 32767);

const BRAND: &str = "Nitrogen";
const BRAND_IDENTIFIER: &str = "minecraft:brand";
const LEGACY_BRAND_IDENTIFIER: &str = "MC|Brand";

pub(crate) fn is_brand(identifier: &Identifier) -> bool {
    identifier.string().eq(BRAND_IDENTIFIER) || identifier.string().eq(LEGACY_BRAND_IDENTIFIER)
}

pub(crate) fn rewrite_brand(vec: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let current_brand = BigString::decode(&mut Cursor::new(vec))?;
    let new_brand = BigString::from(format!("{} ({})", current_brand.string(), BRAND));
    let mut new_vec = Vec::with_capacity(new_brand.size()?.try_into()?);
    new_brand.encode(&mut new_vec)?;
    Ok(new_vec)
}