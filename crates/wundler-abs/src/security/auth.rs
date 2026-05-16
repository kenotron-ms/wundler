//! Bearer-token authentication middleware and secret-token type.

use std::fmt;

use subtle::ConstantTimeEq;

// ---------------------------------------------------------------------------
// SecretToken
// ---------------------------------------------------------------------------

/// A bearer token stored as raw bytes.
///
/// # Security properties
///
/// * [`fmt::Debug`] and [`fmt::Display`] **never** emit the token bytes —
///   they always print `[REDACTED]`. This prevents accidental token leakage
///   in `tracing` events, panic messages, or log lines.
/// * [`SecretToken::verify`] uses constant-time comparison via
///   [`subtle::ConstantTimeEq`] to prevent timing-oracle attacks.
pub struct SecretToken(Vec<u8>);

impl SecretToken {
    /// Wrap `bytes` in a `SecretToken`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Return `true` iff `provided` exactly matches the stored token.
    ///
    /// The comparison is **constant-time**: both a length mismatch and a
    /// byte mismatch take the same time to evaluate, preventing timing
    /// oracles.
    pub fn verify(&self, provided: &str) -> bool {
        // IMPORTANT: do not replace this with `==`.  `subtle::ConstantTimeEq`
        // is non-negotiable here — see RISKS section of security-baseline.md.
        bool::from(self.0.as_slice().ct_eq(provided.as_bytes()))
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretToken([REDACTED])")
    }
}

impl fmt::Display for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED]")
    }
}
