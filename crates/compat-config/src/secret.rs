//! Same redaction discipline as `crypto::Secret<T>` (see
//! `crates/crypto/src/secret.rs`), but persistable: compatibility
//! credentials (VLESS UUIDs, Hysteria2 passwords, REALITY private keys)
//! must round-trip through an on-disk `users.json` / key file, which
//! `crypto::Secret<T>` deliberately does not support. `SecretString`
//! serializes transparently (the file itself is the protected boundary —
//! mode 0600/0700, see `store.rs`) but never implements `Debug`/`Display`,
//! so a stray `{:?}` in a log line cannot leak it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretString(REDACTED)")
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_value() {
        let s = SecretString::new("super-secret-password");
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("super-secret-password"));
        assert_eq!(dbg, "SecretString(REDACTED)");
    }

    #[test]
    fn serialize_round_trips_plain_value() {
        let s = SecretString::new("abc123");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"abc123\"");
        let back: SecretString = serde_json::from_str(&json).unwrap();
        assert_eq!(back.expose(), "abc123");
    }
}
