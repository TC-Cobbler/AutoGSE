//! Phase 12 §12.4: a small `String` newtype that zeroizes its backing memory
//! on drop (`zeroize::ZeroizeOnDrop`), for values that must also survive
//! `serde`/`Clone` derives on the structs that hold them (`Credentials`,
//! `RaCredentials`, `goldberg::AuthMode::Authenticated`) — `zeroize`'s own
//! `Zeroizing<String>` wrapper doesn't implement `Serialize`/`Deserialize`,
//! so it can't be dropped straight into those structs without breaking their
//! existing derives. `Debug` is hand-written to redact the value rather than
//! derived, so a stray `{:?}` (log line, panic message, `assert_eq!` failure
//! output) never prints a real credential.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(\"[REDACTED]\")")
    }
}

impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for SecretString {}

// Manual impls (rather than relying on `String`'s own derive) so the
// *outer* structs (`Credentials`, `RaCredentials`, ...) can keep deriving
// `Serialize`/`Deserialize` unchanged — the on-disk/wire shape is identical
// to a plain `String` field, only the in-memory lifetime behavior differs.
impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(SecretString)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_real_value() {
        let secret = SecretString::new("hunter2".to_string());
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("hunter2"));
        assert_eq!(rendered, "SecretString(\"[REDACTED]\")");
    }

    #[test]
    fn as_str_returns_the_real_value() {
        let secret = SecretString::new("hunter2".to_string());
        assert_eq!(secret.as_str(), "hunter2");
    }

    #[test]
    fn serializes_and_deserializes_as_a_plain_string() {
        let secret = SecretString::new("hunter2".to_string());
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(json, "\"hunter2\"");
        let round_tripped: SecretString = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.as_str(), "hunter2");
    }

    #[test]
    fn equality_compares_the_real_value() {
        assert_eq!(SecretString::new("a".to_string()), SecretString::new("a".to_string()));
        assert_ne!(SecretString::new("a".to_string()), SecretString::new("b".to_string()));
    }
}
