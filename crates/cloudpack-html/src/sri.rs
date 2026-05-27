//! Convert hex-encoded SHA-256 digests (the CAS format) into the base64
//! form required by Subresource Integrity.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Convert a 64-character lowercase hex SHA-256 digest into a base64 SRI payload.
/// No `sha256-` prefix — callers prepend it.
///
/// # Panics
///
/// Panics if `hex` is not valid hexadecimal.
pub fn hex_to_sri_b64(hex: &str) -> String {
    let bytes = hex::decode(hex).expect("CAS hash must be valid hex");
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_sha256_matches_known_sri_value() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let b64 = hex_to_sri_b64(hex);
        assert_eq!(b64, "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=");
    }

    #[test]
    fn round_trip_via_base64_decode_matches_original_bytes() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let b64 = hex_to_sri_b64(hex);
        let decoded = STANDARD.decode(&b64).expect("our own output must be valid b64");
        assert_eq!(hex::encode(decoded), hex);
    }

    #[test]
    #[should_panic(expected = "CAS hash must be valid hex")]
    fn non_hex_input_panics() {
        let _ = hex_to_sri_b64("not-hex-at-all-zz");
    }
}
