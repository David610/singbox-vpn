//! Generic hex serde helper for fixed-size byte arrays (serde only has a
//! built-in `Serialize`/`Deserialize` impl for arrays up to length 32).

use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<const N: usize, S: Serializer>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(bytes))
}

pub fn deserialize<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<[u8; N], D::Error> {
    let s = String::deserialize(d)?;
    let v = hex::decode(s).map_err(serde::de::Error::custom)?;
    v.try_into()
        .map_err(|_| serde::de::Error::custom(format!("expected {N} bytes")))
}
