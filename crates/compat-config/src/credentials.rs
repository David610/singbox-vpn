//! Credential generation for compatibility (VLESS/Hysteria2) clients.
//! Uses the OS CSPRNG (`rand::rngs::OsRng`) exclusively — no custom
//! randomness construction, matching `crypto`'s "no custom crypto" rule.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use curve25519_dalek::montgomery::MontgomeryPoint;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Random UUIDv4, formatted per RFC 4122. Used as the VLESS user id.
pub fn generate_uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    // Set version (4) and variant (RFC 4122) bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// A compatibility-user id: `user_<uuidv4>` — 128 bits of CSPRNG entropy,
/// not the 32-bit `generate_short_id` (which is reserved for the REALITY
/// short_id and must not be reused as a general-purpose identifier).
/// Callers should still run `is_duplicate_user_id` before insert as
/// defense in depth even though a collision at 128 bits is not
/// realistically reachable.
pub fn generate_user_id() -> String {
    format!("user_{}", generate_uuid_v4())
}

/// A Hysteria2 user password: 24 random bytes, hex-encoded (192 bits).
pub fn generate_hysteria2_password() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A REALITY short_id: 0-8 hex digits per the sing-box spec; we always use
/// the full 8 (32 bits) and avoid low-entropy/obviously-patterned values
/// by construction (CSPRNG, never a fixed default).
pub fn generate_short_id() -> String {
    let mut bytes = [0u8; 4];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A high-entropy (160-bit) subscription token. Returned once to the
/// caller (e.g. printed by `vpn-admin user create`); only its hash is
/// persisted (see `hash_token`).
pub fn generate_subscription_token() -> String {
    let mut bytes = [0u8; 20];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// SHA-256 hex digest of a token. Tokens are CSPRNG-generated with 160
/// bits of entropy, so a fast hash (vs. a deliberately slow password KDF)
/// is appropriate here: the attack this defends against is "attacker who
/// obtains the users.json file learns the raw token", not "attacker
/// brute-forces a low-entropy secret offline" — 160 bits is infeasible to
/// brute force by either avenue.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

/// Constant-time comparison between a presented token and a stored hash,
/// so subscription lookups cannot be timed to leak information about
/// which prefix bytes matched.
pub fn verify_token(token: &str, stored_hash_hex: &str) -> bool {
    let computed = hash_token(token);
    let computed_bytes = computed.as_bytes();
    let stored_bytes = stored_hash_hex.as_bytes();
    if computed_bytes.len() != stored_bytes.len() {
        return false;
    }
    computed_bytes.ct_eq(stored_bytes).into()
}

/// Derive the REALITY/X25519 public key from the private key encoding used
/// by sing-box (`base64.RawURLEncoding`, i.e. URL-safe base64 without
/// padding). This is deliberately independent of the public-key file: a
/// comparison that merely renders and re-reads `public.key` cannot detect a
/// split keypair.
pub fn derive_reality_public_key(private_key: &str) -> Result<String, String> {
    let decoded = URL_SAFE_NO_PAD
        .decode(private_key.trim())
        .map_err(|e| format!("REALITY private key is not valid base64url without padding: {e}"))?;
    let private: [u8; 32] = decoded.try_into().map_err(|v: Vec<u8>| {
        format!(
            "REALITY private key decodes to {} bytes; X25519 requires exactly 32",
            v.len()
        )
    })?;
    let public = MontgomeryPoint::mul_base_clamped(private).to_bytes();
    Ok(URL_SAFE_NO_PAD.encode(public))
}

/// Validate both the public-key encoding and its cryptographic
/// correspondence to the private key.
pub fn validate_reality_keypair(private_key: &str, public_key: &str) -> Result<(), String> {
    let supplied = URL_SAFE_NO_PAD
        .decode(public_key.trim())
        .map_err(|e| format!("REALITY public key is not valid base64url without padding: {e}"))?;
    if supplied.len() != 32 {
        return Err(format!(
            "REALITY public key decodes to {} bytes; X25519 requires exactly 32",
            supplied.len()
        ));
    }
    let derived = derive_reality_public_key(private_key)?;
    if derived
        .as_bytes()
        .ct_eq(public_key.trim().as_bytes())
        .into()
    {
        Ok(())
    } else {
        Err("REALITY public key does not correspond to private.key".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_v4_has_expected_shape() {
        let id = generate_uuid_v4();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert_eq!(&parts[2][0..1], "4"); // version nibble
    }

    #[test]
    fn user_id_has_expected_shape_and_128_bits_of_entropy_source() {
        let id = generate_user_id();
        assert!(id.starts_with("user_"));
        let uuid_part = id.strip_prefix("user_").unwrap();
        assert_eq!(uuid_part.len(), 36);
    }

    #[test]
    fn user_ids_do_not_collide_across_ten_thousand_generations() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = generate_user_id();
            assert!(seen.insert(id), "user id collision within 10k generations");
        }
    }

    #[test]
    fn generated_values_are_not_trivially_repeated() {
        let a = generate_uuid_v4();
        let b = generate_uuid_v4();
        assert_ne!(a, b);
        let p1 = generate_hysteria2_password();
        let p2 = generate_hysteria2_password();
        assert_ne!(p1, p2);
        let s1 = generate_short_id();
        let s2 = generate_short_id();
        assert_ne!(s1, s2);
    }

    #[test]
    fn short_id_is_never_an_obvious_pattern() {
        for _ in 0..1000 {
            let id = generate_short_id();
            assert_ne!(id, "00000000");
            assert_ne!(id, "12345678");
        }
    }

    #[test]
    fn subscription_token_has_adequate_entropy_and_charset() {
        let t = generate_subscription_token();
        // 20 bytes base64url-no-pad => 27 chars, >=128 bits entropy (spec §26).
        assert_eq!(t.len(), 27);
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn token_hash_verifies_correct_token_only() {
        let token = generate_subscription_token();
        let hash = hash_token(&token);
        assert!(verify_token(&token, &hash));
        assert!(!verify_token("wrong-token", &hash));
    }

    #[test]
    fn verify_token_rejects_mismatched_length_hash() {
        assert!(!verify_token("abc", "not-a-real-hash"));
    }

    #[test]
    fn reality_public_key_is_derived_from_the_private_key() {
        // Deterministic vector independently generated with X25519 from a
        // 32-byte private scalar containing only 0x01.
        let private = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
        let public = "pOCSkrZRwni5dyxWn1-puxPZBrRqtoyd-dwrRAn4ogk";
        assert_eq!(derive_reality_public_key(private).unwrap(), public);
        assert!(validate_reality_keypair(private, public).is_ok());
    }

    #[test]
    fn reality_keypair_validation_rejects_mismatch_and_malformed_keys() {
        let private = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
        let other_public = "zo060cy2M-x7cMF4FKXHbs0CloUFDTRHRboFhw5YfVk";
        assert!(validate_reality_keypair(private, other_public).is_err());
        assert!(validate_reality_keypair("not-base64!", other_public).is_err());
        assert!(validate_reality_keypair(private, "short").is_err());
    }
}
