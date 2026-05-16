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

// ---------------------------------------------------------------------------
// require_bearer middleware (imports kept local to this section)
// ---------------------------------------------------------------------------

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use super::ResolvedSecurity;

// ---------------------------------------------------------------------------
// Paths exempt from bearer-token authentication
// ---------------------------------------------------------------------------

/// Routes that always bypass the bearer-token check, regardless of whether
/// security is enabled. These are publicly readable endpoints.
const EXEMPT_PATHS: &[&str] = &["/health", "/sw.js"];

// ---------------------------------------------------------------------------
// require_bearer middleware
// ---------------------------------------------------------------------------

/// Axum middleware that enforces bearer-token authentication.
///
/// * Paths in [`EXEMPT_PATHS`] (`/health`, `/sw.js`) always pass through.
/// * When `security.is_enabled()` is `false`, every request passes through
///   unchanged — zero behaviour change from today.
/// * With security enabled, the request **must** carry
///   `Authorization: Bearer <token>`. Any other value, a missing header, or
///   a wrong token all return **identical** `401 Unauthorized` responses.
pub async fn require_bearer(
    State(security): State<Arc<ResolvedSecurity>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Exempt paths bypass auth unconditionally.
    if EXEMPT_PATHS.contains(&path) {
        return next.run(request).await;
    }

    // If security is not configured, pass through.
    if !security.is_enabled() {
        return next.run(request).await;
    }

    // Extract the token from `Authorization: Bearer <token>`.
    let token_str = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    // IMPORTANT: both the "no header" and "wrong token" cases return the
    // same response to prevent leaking whether a token was present.
    match token_str {
        Some(t) if security.token.as_ref().unwrap().verify(t) => next.run(request).await,
        _ => unauthorized_response(),
    }
}

/// Build the standard 401 response.
fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Unauthorized",
    )
        .into_response()
}
