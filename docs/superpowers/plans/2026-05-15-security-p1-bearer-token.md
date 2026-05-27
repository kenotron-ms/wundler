# Security P1.1 — Bearer Token Auth Implementation Plan

> **Execution:** Use the subagent-driven-development workflow to implement this plan.

**Goal:** Add optional bearer token authentication as Axum middleware on all ABS routes except `/health` and `/sw.js`, with zero breaking changes when `[security]` is absent from config.

**Architecture:** New `crates/cloudpack-abs/src/security/` module containing `SecurityConfig` (parsed from cloudpack.toml), `ResolvedSecurity` (loaded at startup by reading the token file), `SecretToken` (constant-time compare via `subtle`, Debug/Display redacted), and `require_bearer` (Axum `from_fn_with_state` middleware). The middleware short-circuits to `next.run()` when security is disabled, preserving today's unauthenticated behavior exactly. `build_router` gains a third `Arc<ResolvedSecurity>` parameter; existing callers are updated to pass `ResolvedSecurity::from_config(&SecurityConfig::default())` (i.e., security off) so all existing tests continue to pass unchanged.

**Tech Stack:** Rust, axum 0.8, subtle = "2" (new, for constant-time comparison), serde/toml (already used), thiserror (already used), tempfile (already a dev-dependency).

---

## Scope boundary — this plan is P1.1 ONLY

Do **not** add any of the following (they are separate plans):
- CORS allowlist (P1.2)
- Per-IP rate limiter (P1.3)
- Env var token source (`CLOUDPACK_BEARER_TOKEN`)
- Two-token grace period (multiple tokens per file)
- X-Forwarded-For handling
- Any Phase 2 signing, SRI, or CSP work

---

## Pre-flight checks

Before starting Task 1, verify:

```bash
cd /Users/ken/workspace/ms/cloudpack
cargo test -p cloudpack-abs 2>&1 | tail -5
```

Expected: all tests pass, working tree clean.

---

### Task 1: Add `subtle` dependency and create the security module skeleton

**Files:**
- Modify: `crates/cloudpack-abs/Cargo.toml`
- Create: `crates/cloudpack-abs/src/security/mod.rs`
- Create: `crates/cloudpack-abs/src/security/auth.rs`
- Modify: `crates/cloudpack-abs/src/lib.rs`

**Step 1: Add `subtle` to Cargo.toml**

Open `crates/cloudpack-abs/Cargo.toml`. Add one line under `[dependencies]` (the `subtle` crate is not in the workspace — add it directly here):

```toml
subtle = "2"
```

The `[dependencies]` section should look like this afterward:

```toml
[dependencies]
cloudpack-core = { path = "../cloudpack-core" }
cloudpack-graph = { path = "../cloudpack-graph" }
axum = { version = "0.8", features = ["json"] }
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.6", features = ["cors", "trace"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ed25519-dalek = { version = "2", features = ["pem", "rand_core"] }
pkcs8 = { version = "0.10", features = ["pem"] }
rand = "0.8"
uuid = { version = "1", features = ["v4"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
subtle = "2"
```

**Step 2: Create the empty security module files**

Create `crates/cloudpack-abs/src/security/mod.rs` with this content:

```rust
//! Security middleware for the Asset Bundling Server.
//!
//! This module provides optional bearer-token authentication applied as an
//! Axum middleware layer. When the `[security]` section is absent from
//! `cloudpack.toml`, the middleware is a transparent pass-through — existing
//! behaviour is unchanged.

pub mod auth;
```

Create `crates/cloudpack-abs/src/security/auth.rs` with this content:

```rust
//! Bearer-token authentication middleware and secret-token type.
```

**Step 3: Wire `pub mod security;` into lib.rs**

Open `crates/cloudpack-abs/src/lib.rs`. Add the line:

```rust
pub mod security;
```

The file should look like:

```rust
//! # cloudpack-abs
//!
//! ... (existing doc comment) ...

pub mod manifest;
pub mod security;
pub mod server;
pub mod signing;
pub mod state;
pub mod telemetry;
pub mod types;
```

**Step 4: Verify compilation**

```bash
cargo check -p cloudpack-abs
```

Expected: compiles cleanly (zero errors, zero warnings).

**Step 5: Commit**

```bash
git add crates/cloudpack-abs/Cargo.toml \
        crates/cloudpack-abs/src/security/mod.rs \
        crates/cloudpack-abs/src/security/auth.rs \
        crates/cloudpack-abs/src/lib.rs \
        Cargo.lock
git commit -m "feat(abs/security): add security module skeleton and subtle dependency"
```

---

### Task 2: `SecretToken` type — unit tests first, then implementation

**Files:**
- Extend: `crates/cloudpack-abs/src/security/auth.rs`
- Create: `crates/cloudpack-abs/tests/auth_test.rs`

**Step 1: Write the failing unit tests**

Create `crates/cloudpack-abs/tests/auth_test.rs` with the following content. These tests will fail to compile because `SecretToken` does not exist yet.

```rust
//! Tests for bearer-token authentication — unit tests (SecretToken) and
//! HTTP integration tests (require_bearer middleware end-to-end).

use cloudpack_abs::security::auth::SecretToken;

// ── SecretToken unit tests ────────────────────────────────────────────────────

/// Debug output must never contain the raw token bytes.
///
/// This is a hard security requirement: if a `SecretToken` is accidentally
/// included in a `tracing` event or a `{:?}` panic message, the token must
/// not be visible.
#[test]
fn test_secret_token_debug_does_not_leak_value() {
    let token = SecretToken::new(b"super-secret-token-value".to_vec());
    let debug_str = format!("{:?}", token);

    assert!(
        !debug_str.contains("super-secret-token-value"),
        "Debug output must not contain the raw token; got: {debug_str}"
    );
    assert!(
        debug_str.contains("REDACTED"),
        "Debug output should contain 'REDACTED'; got: {debug_str}"
    );
}

/// Display output must never contain the raw token bytes.
#[test]
fn test_secret_token_display_does_not_leak_value() {
    let token = SecretToken::new(b"another-secret-12345".to_vec());
    let display_str = format!("{}", token);

    assert!(
        !display_str.contains("another-secret-12345"),
        "Display output must not contain the raw token; got: {display_str}"
    );
    assert!(
        display_str.contains("REDACTED"),
        "Display output should contain 'REDACTED'; got: {display_str}"
    );
}

/// verify() returns true when the provided string matches the stored token.
#[test]
fn test_verify_correct_token_returns_true() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        token.verify("correct-token"),
        "verify() should return true for the correct token"
    );
}

/// verify() returns false when the provided string does not match.
#[test]
fn test_verify_wrong_token_returns_false() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        !token.verify("wrong-token"),
        "verify() should return false for an incorrect token"
    );
}

/// verify() returns false for an empty string, even if the token is non-empty.
#[test]
fn test_verify_empty_string_returns_false() {
    let token = SecretToken::new(b"non-empty-token".to_vec());
    assert!(
        !token.verify(""),
        "verify() should return false for an empty provided string"
    );
}
```

**Step 2: Run the failing tests to confirm the error**

```bash
cargo test -p cloudpack-abs --test auth_test 2>&1 | head -20
```

Expected: compilation error — `cannot find module 'auth'` or `unresolved import`. This is the expected failure.

**Step 3: Implement `SecretToken` in `security/auth.rs`**

Replace the entire contents of `crates/cloudpack-abs/src/security/auth.rs` with:

```rust
//! Bearer-token authentication middleware and secret-token type.

use std::fmt;
use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

use super::ResolvedSecurity;

// ---------------------------------------------------------------------------
// Paths exempt from bearer-token authentication
// ---------------------------------------------------------------------------

/// Routes that always bypass the bearer-token check, regardless of whether
/// security is enabled. These are publicly readable endpoints.
const EXEMPT_PATHS: &[&str] = &["/health", "/sw.js"];

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
// require_bearer middleware
// ---------------------------------------------------------------------------

/// Axum middleware that enforces bearer-token authentication.
///
/// # Behaviour
///
/// * Paths in [`EXEMPT_PATHS`] (`/health`, `/sw.js`) always pass through,
///   even when security is enabled.
/// * When `security.is_enabled()` is `false` (no `[security]` section in
///   `cloudpack.toml`), every request passes through unchanged — zero
///   behaviour change from today.
/// * With security enabled, the request **must** carry
///   `Authorization: Bearer <token>`. Any other value, a missing header, or
///   a wrong token all return **identical** `401 Unauthorized` responses.
///   The response is intentionally identical to prevent leaking whether a
///   token was present but wrong vs. absent entirely.
///
/// # Usage
///
/// ```ignore
/// use std::sync::Arc;
/// use axum::middleware;
///
/// let router = Router::new()
///     .route(...)
///     .layer(middleware::from_fn_with_state(
///         Arc::new(resolved_security),
///         security::auth::require_bearer,
///     ))
///     .with_state(state);
/// ```
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

    // If security is not configured, pass through (additive change: no config = no auth).
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
    // same response.  Do not distinguish them — that would leak information.
    match token_str {
        Some(t) if security.token.as_ref().unwrap().verify(t) => next.run(request).await,
        _ => unauthorized_response(),
    }
}

/// Build the standard 401 response.
///
/// Extracted into a function to guarantee that every rejection code path
/// returns exactly the same bytes.
fn unauthorized_response() -> Response {
    // NEVER log the provided token here, even at trace level.
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Unauthorized",
    )
        .into_response()
}
```

**Step 4: Run the unit tests and verify they pass**

```bash
cargo test -p cloudpack-abs --test auth_test test_secret_token 2>&1
cargo test -p cloudpack-abs --test auth_test test_verify 2>&1
```

Expected output:
```
test test_secret_token_debug_does_not_leak_value ... ok
test test_secret_token_display_does_not_leak_value ... ok
test test_verify_correct_token_returns_true ... ok
test test_verify_wrong_token_returns_false ... ok
test test_verify_empty_string_returns_false ... ok
```

**Step 5: Commit**

```bash
git add crates/cloudpack-abs/src/security/auth.rs \
        crates/cloudpack-abs/tests/auth_test.rs
git commit -m "feat(abs/security): add SecretToken type with constant-time verify and Debug redaction"
```

---

### Task 3: `SecurityError`, `SecurityConfig`, and `ResolvedSecurity` — tests first

**Files:**
- Extend: `crates/cloudpack-abs/tests/auth_test.rs`
- Extend: `crates/cloudpack-abs/src/security/mod.rs`

**Step 1: Write the failing tests**

Append the following to `crates/cloudpack-abs/tests/auth_test.rs`:

```rust
// ── SecurityConfig / ResolvedSecurity unit tests ──────────────────────────────

use std::io::Write as _;
use tempfile::NamedTempFile;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig, SecurityError};

/// When no `bearer_token_file` is set, security is disabled (pass-through).
#[test]
fn test_no_config_security_is_not_enabled() {
    let config = SecurityConfig::default();
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        !security.is_enabled(),
        "security should be disabled when no token file is configured"
    );
}

/// When a token file exists and is non-empty, security is enabled.
#[test]
fn test_token_file_present_security_is_enabled() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    write!(tmp, "my-bearer-token").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        security.is_enabled(),
        "security should be enabled when a token file is configured"
    );
}

/// Token files may have a trailing newline (common from `echo` or editors).
/// The token is trimmed before storage.
#[test]
fn test_token_file_with_trailing_newline_is_trimmed() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    write!(tmp, "trimmed-token\n").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    // The stored token should verify against the trimmed string, not the newline-terminated one.
    assert!(
        security.token.as_ref().unwrap().verify("trimmed-token"),
        "token should be trimmed of trailing whitespace"
    );
    assert!(
        !security.token.as_ref().unwrap().verify("trimmed-token\n"),
        "token should not verify against the un-trimmed version"
    );
}

/// A non-existent token file path produces a `SecurityError::TokenFileRead`.
#[test]
fn test_missing_token_file_returns_error() {
    let config = SecurityConfig {
        bearer_token_file: Some("/nonexistent/path/that/does/not/exist/token.txt".into()),
    };
    let result = ResolvedSecurity::from_config(&config);
    assert!(
        result.is_err(),
        "from_config should fail when the token file does not exist"
    );
    // The error message should reference the path.
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("nonexistent"),
        "error message should mention the path; got: {err_str}"
    );
}
```

**Step 2: Run failing tests to confirm the error**

```bash
cargo test -p cloudpack-abs --test auth_test test_no_config 2>&1 | head -10
```

Expected: compilation error — `cannot find type SecurityConfig` (or similar). This is the expected failure.

**Step 3: Implement `SecurityError`, `SecurityConfig`, and `ResolvedSecurity`**

Replace the entire contents of `crates/cloudpack-abs/src/security/mod.rs` with:

```rust
//! Security middleware for the Asset Bundling Server.
//!
//! This module provides optional bearer-token authentication applied as an
//! Axum middleware layer. When the `[security]` section is absent from
//! `cloudpack.toml`, the middleware is a transparent pass-through — existing
//! behaviour is unchanged.

pub mod auth;

use std::path::PathBuf;

use crate::security::auth::SecretToken;

// ---------------------------------------------------------------------------
// SecurityConfig
// ---------------------------------------------------------------------------

/// Configuration for the ABS security layer, parsed from `[security]` in
/// `cloudpack.toml`.
///
/// All fields are optional; `Default` produces a no-security configuration
/// that preserves today's unauthenticated behaviour.
#[derive(Debug, Default, serde::Deserialize)]
pub struct SecurityConfig {
    /// Path to a file containing the bearer token (one line, trimmed).
    ///
    /// Storing the token in a file (rather than inline in `cloudpack.toml`)
    /// prevents accidental commit and log leakage.
    ///
    /// If this field is absent, bearer-token authentication is disabled.
    pub bearer_token_file: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// SecurityError
// ---------------------------------------------------------------------------

/// Errors that can occur while resolving a [`SecurityConfig`].
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// The bearer token file could not be read.
    #[error("failed to read bearer token file at {0}: {1}")]
    TokenFileRead(PathBuf, std::io::Error),
}

// ---------------------------------------------------------------------------
// ResolvedSecurity
// ---------------------------------------------------------------------------

/// The runtime-ready form of [`SecurityConfig`].
///
/// Created once at server startup by [`ResolvedSecurity::from_config`] and
/// then shared across all requests via `Arc`.
pub struct ResolvedSecurity {
    /// The bearer token, or `None` if authentication is disabled.
    pub token: Option<SecretToken>,
}

impl ResolvedSecurity {
    /// Build a `ResolvedSecurity` from a [`SecurityConfig`].
    ///
    /// When `config.bearer_token_file` is `Some`, the file is read
    /// synchronously (startup path, not per-request) and the content is
    /// trimmed of surrounding whitespace before being stored.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityError::TokenFileRead`] if the file cannot be read.
    pub fn from_config(config: &SecurityConfig) -> Result<Self, SecurityError> {
        let token = match &config.bearer_token_file {
            None => None,
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| SecurityError::TokenFileRead(path.clone(), e))?;
                Some(SecretToken::new(raw.trim().as_bytes().to_vec()))
            }
        };
        Ok(Self { token })
    }

    /// Returns `true` if bearer-token authentication is active.
    pub fn is_enabled(&self) -> bool {
        self.token.is_some()
    }
}
```

**Step 4: Run the new tests and verify they pass**

```bash
cargo test -p cloudpack-abs --test auth_test 2>&1
```

Expected: all 9 tests in the file pass (5 SecretToken tests + 4 config tests).

**Step 5: Commit**

```bash
git add crates/cloudpack-abs/src/security/mod.rs \
        crates/cloudpack-abs/tests/auth_test.rs
git commit -m "feat(abs/security): add SecurityConfig, ResolvedSecurity, and SecurityError"
```

---

### Task 4: Update `build_router` — new signature, middleware wiring, and fix all callers

This is the integration seam. `build_router` gains a third parameter; the middleware is wired into the router; and two existing test helpers are updated so they compile unchanged.

**Files:**
- Modify: `crates/cloudpack-abs/src/server.rs`
- Modify: `crates/cloudpack-abs/tests/server_smoke_test.rs`
- Modify: `crates/cloudpack-abs/tests/http_integration_test.rs`

**Step 1: Update `build_router` in `server.rs`**

Open `crates/cloudpack-abs/src/server.rs`. Make the following changes:

**a) Update imports at the top of `server.rs`**

Change the existing `use anyhow::Result;` line to:

```rust
use anyhow::{Context, Result};
```

Then add these new import lines alongside the existing ones:

```rust
use std::sync::Arc;
use axum::middleware;
use crate::security::auth::require_bearer;
use crate::security::{ResolvedSecurity, SecurityConfig};
```

**b) Change the `build_router` signature and body** — replace the existing `build_router` function:

```rust
// OLD (replace this):
pub fn build_router(app: AppState, telemetry: TelemetryLogger) -> Router {
    let state = RouterState { app, telemetry };

    Router::new()
        .route("/manifest", post(post_manifest))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .with_state(state)
}
```

```rust
// NEW:
/// Build and return the Axum [`Router`] without starting a listener.
///
/// Separating router construction from server startup makes the router
/// independently testable without needing a live TCP socket.
///
/// The `security` argument is applied as a [`middleware::from_fn_with_state`]
/// layer that wraps all routes. When `security.is_enabled()` is `false`
/// (the default when no `[security]` block is present in `cloudpack.toml`),
/// the middleware is a transparent pass-through.
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    security: Arc<ResolvedSecurity>,
) -> Router {
    let state = RouterState { app, telemetry };

    Router::new()
        .route("/manifest", post(post_manifest))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .layer(middleware::from_fn_with_state(security, require_bearer))
        .with_state(state)
}
```

**c) Update `run()` in `server.rs`** — replace the body of the `run` function to pass security:

```rust
// OLD (replace this):
pub async fn run(config: AbsConfig) -> Result<()> {
    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
    )
    .await?;

    let telemetry = TelemetryLogger::new(&config.telemetry_log)?;
    let router = build_router(app, telemetry);
    // ...
}
```

```rust
// NEW:
pub async fn run(config: AbsConfig) -> Result<()> {
    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
    )
    .await?;

    let telemetry = TelemetryLogger::new(&config.telemetry_log)?;

    let security = ResolvedSecurity::from_config(&config.security)
        .context("failed to initialise security config")?;

    let router = build_router(app, telemetry, Arc::new(security));

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("cloudpack-abs listening on {}", local_addr);

    axum::serve(listener, router).await?;

    Ok(())
}
```

**d) Add `security` field to `AbsConfig`** — add one field to the struct and one line to `Default`:

```rust
// In AbsConfig struct, add:
/// Security configuration (bearer token, etc.).
///
/// Defaults to no security (unauthenticated), which preserves existing
/// behaviour when the `[security]` section is absent.
pub security: SecurityConfig,

// In AbsConfig::default(), add:
security: SecurityConfig::default(),
```

The full updated `AbsConfig` struct:

```rust
#[derive(Debug, Clone)]
pub struct AbsConfig {
    /// Path to the `ChunkManifest` JSON file on disk.
    pub manifest_path: PathBuf,

    /// Base URL of the CDN that serves chunk assets.
    pub cdn_base_url: String,

    /// Path to the append-only JSONL telemetry log.
    pub telemetry_log: PathBuf,

    /// Path to an Ed25519 signing key in PEM format, or `None` to disable
    /// request signing.
    pub signing_key_pem: Option<PathBuf>,

    /// TCP port the server listens on.
    pub port: u16,

    /// How long clients should cache a manifest response, in seconds.
    pub ttl_seconds: u64,

    /// Security configuration (bearer token, etc.).
    ///
    /// Defaults to no security (unauthenticated), which preserves existing
    /// behaviour when the `[security]` section is absent.
    pub security: SecurityConfig,
}

impl Default for AbsConfig {
    fn default() -> Self {
        Self {
            manifest_path: PathBuf::from("dist/manifest.json"),
            cdn_base_url: "https://cdn.example.com".to_string(),
            telemetry_log: PathBuf::from("/tmp/cloudpack-telemetry.jsonl"),
            signing_key_pem: None,
            port: 8080,
            ttl_seconds: 300,
            security: SecurityConfig::default(),
        }
    }
}
```

Note: `AbsConfig` derives `Clone` but `SecurityConfig` does not have `Clone` yet — add `#[derive(Debug, Default, Clone, serde::Deserialize)]` to `SecurityConfig` in `security/mod.rs`.

**Step 2: Fix `server_smoke_test.rs`**

Open `crates/cloudpack-abs/tests/server_smoke_test.rs`. Update the `build_router` call:

```rust
// OLD:
let _router = build_router(app, telemetry);

// NEW:
use std::sync::Arc;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};

let security = ResolvedSecurity::from_config(&SecurityConfig::default())
    .expect("default security config should succeed");
let _router = build_router(app, telemetry, Arc::new(security));
```

The full updated `router_builds_without_panic` test:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};
use cloudpack_abs::server::{build_router, AbsConfig};
use cloudpack_abs::state::AppState;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_graph::ChunkManifest;

/// Verify that `build_router` runs to completion without panicking.
///
/// This test does **not** start an HTTP listener; it only exercises the
/// router-construction code path so that wiring errors (missing state,
/// bad route configuration, etc.) surface at test time rather than at
/// server startup.
#[test]
fn router_builds_without_panic() {
    let manifest = ChunkManifest {
        build_id: "smoke-build-id".to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    };

    let app = AppState {
        manifest: Arc::new(RwLock::new(manifest)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    };

    let tmp = tempfile::NamedTempFile::new().expect("failed to create temp file");
    let telemetry = TelemetryLogger::new(tmp.path()).expect("failed to create TelemetryLogger");

    let security = ResolvedSecurity::from_config(&SecurityConfig::default())
        .expect("default security config should succeed");

    // Should not panic.
    let _router = build_router(app, telemetry, Arc::new(security));
}

/// Verify that `AbsConfig::default()` produces the expected port and TTL.
#[test]
fn abs_config_has_sensible_defaults() {
    let config = AbsConfig::default();
    assert_eq!(config.port, 8080, "default port should be 8080");
    assert_eq!(config.ttl_seconds, 300, "default TTL should be 300 seconds");
}
```

**Step 3: Fix `http_integration_test.rs`**

Open `crates/cloudpack-abs/tests/http_integration_test.rs`. Update the `make_server` helper:

```rust
// OLD:
async fn make_server() -> (TestServer, TempDir) {
    // ...
    let router = build_router(app, telemetry);
    // ...
}

// NEW — add these imports at the top of the file:
use std::sync::Arc;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};

// Then update make_server:
async fn make_server() -> (TestServer, TempDir) {
    let manifest = manifest_file();
    let tmp_dir = TempDir::new().expect("failed to create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");

    let app = AppState::load_from_disk(
        manifest.path(),
        "https://cdn.example.com".to_string(),
        300,
    )
    .await
    .expect("failed to load AppState from manifest");

    let telemetry = TelemetryLogger::new(&log_path).expect("failed to create TelemetryLogger");

    let security = ResolvedSecurity::from_config(&SecurityConfig::default())
        .expect("default security config should succeed");

    let router = build_router(app, telemetry, Arc::new(security));
    let server = TestServer::new(router);

    std::mem::forget(manifest);

    (server, tmp_dir)
}
```

**Step 4: Run all existing tests — they must all pass**

```bash
cargo test -p cloudpack-abs 2>&1
```

Expected: all previously passing tests continue to pass. Zero failures. The only new passing tests are the ones from Tasks 2 and 3.

**Step 5: Commit**

```bash
git add crates/cloudpack-abs/src/server.rs \
        crates/cloudpack-abs/src/security/mod.rs \
        crates/cloudpack-abs/tests/server_smoke_test.rs \
        crates/cloudpack-abs/tests/http_integration_test.rs
git commit -m "feat(abs/security): wire require_bearer middleware into build_router; update all callers"
```

---

### Task 5: HTTP integration tests — write all 9 auth scenarios

**Files:**
- Extend: `crates/cloudpack-abs/tests/auth_test.rs`

**Step 1: Write all 9 HTTP tests**

Append the following to `crates/cloudpack-abs/tests/auth_test.rs`. These tests exercise the full HTTP stack via `axum_test::TestServer`.

```rust
// ── HTTP integration tests ────────────────────────────────────────────────────
//
// These tests exercise the require_bearer middleware end-to-end through the
// Axum router stack, without opening a TCP socket.

use std::sync::Arc;
use axum::http::{header, HeaderValue, StatusCode};
use axum_test::TestServer;
use tempfile::TempDir;
use cloudpack_abs::server::build_router;
use cloudpack_abs::state::AppState;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_abs::types::ManifestRequest;

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Minimal manifest JSON fixture for auth tests.
///
/// Auth tests don't care about manifest content — they only test whether
/// the middleware allows or rejects requests.
fn auth_test_manifest() -> NamedTempFile {
    let json = serde_json::json!({
        "build_id": "auth-test-build",
        "chunks": [],
        "entry_chunks": {},
        "module_index": {}
    });
    let mut file = NamedTempFile::new().expect("create temp manifest");
    write!(file, "{}", json).expect("write manifest");
    file
}

/// Build a `TestServer` with security **disabled** (no token required).
async fn make_open_server() -> (TestServer, TempDir) {
    let manifest = auth_test_manifest();
    let tmp_dir = TempDir::new().expect("create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");

    let app = AppState::load_from_disk(manifest.path(), "https://cdn.example.com".to_string(), 300)
        .await
        .expect("load AppState");
    let telemetry = TelemetryLogger::new(&log_path).expect("TelemetryLogger");
    let security = ResolvedSecurity::from_config(&SecurityConfig::default())
        .expect("default security");

    let router = build_router(app, telemetry, Arc::new(security));
    std::mem::forget(manifest);
    (TestServer::new(router), tmp_dir)
}

/// Build a `TestServer` with security **enabled** using `token` as the bearer token.
async fn make_secured_server(token: &str) -> (TestServer, TempDir, NamedTempFile) {
    let manifest = auth_test_manifest();
    let tmp_dir = TempDir::new().expect("create temp dir");
    let log_path = tmp_dir.path().join("telemetry.jsonl");

    // Write the token to a temp file (the real from_config code path).
    let mut token_file = NamedTempFile::new().expect("create token file");
    write!(token_file, "{token}").expect("write token");

    let app = AppState::load_from_disk(manifest.path(), "https://cdn.example.com".to_string(), 300)
        .await
        .expect("load AppState");
    let telemetry = TelemetryLogger::new(&log_path).expect("TelemetryLogger");
    let config = SecurityConfig {
        bearer_token_file: Some(token_file.path().to_path_buf()),
    };
    let security = ResolvedSecurity::from_config(&config).expect("ResolvedSecurity");

    let router = build_router(app, telemetry, Arc::new(security));
    std::mem::forget(manifest);
    (TestServer::new(router), tmp_dir, token_file)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// When no `[security]` config is present, unauthenticated requests to
/// `/manifest` are accepted — exactly today's behaviour.
#[tokio::test]
async fn test_no_security_config_allows_unauthenticated_requests() {
    let (server, _dir) = make_open_server().await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "unauthenticated request should not receive 401 when security is disabled"
    );
}

/// A correct `Authorization: Bearer <token>` header is accepted.
#[tokio::test]
async fn test_valid_bearer_token_allows_request() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer my-secret-token"),
        )
        .json(&req)
        .await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "correct bearer token should not receive 401"
    );
}

/// A missing `Authorization` header returns 401.
#[tokio::test]
async fn test_missing_authorization_header_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "missing Authorization header should return 401"
    );
    assert_eq!(resp.text(), "Unauthorized");
}

/// A wrong bearer token returns 401 with the same body as a missing header.
///
/// Both cases must be indistinguishable to prevent leaking whether a token
/// was present but wrong vs. absent entirely.
#[tokio::test]
async fn test_wrong_bearer_token_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-token"),
        )
        .json(&req)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "wrong bearer token should return 401"
    );
    assert_eq!(resp.text(), "Unauthorized");
}

/// An `Authorization: Basic ...` header (wrong scheme) returns 401.
#[tokio::test]
async fn test_wrong_auth_scheme_returns_401() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server
        .post("/manifest")
        .add_header(
            header::AUTHORIZATION,
            HeaderValue::from_static("Basic dXNlcjpwYXNz"),
        )
        .json(&req)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "Basic auth scheme should return 401"
    );
}

/// `GET /health` is always accessible, even when security is enabled.
#[tokio::test]
async fn test_health_always_accessible_without_token() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let resp = server.get("/health").await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "/health should be exempt from auth"
    );
    resp.assert_status_ok();
}

/// `GET /sw.js` is always accessible, even when security is enabled.
#[tokio::test]
async fn test_sw_js_always_accessible_without_token() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let resp = server.get("/sw.js").await;

    assert_ne!(
        resp.status_code(),
        StatusCode::UNAUTHORIZED,
        "/sw.js should be exempt from auth"
    );
    resp.assert_status_ok();
}

/// The 401 response carries a `WWW-Authenticate: Bearer` header.
#[tokio::test]
async fn test_401_response_carries_www_authenticate_header() {
    let (server, _dir, _token_file) = make_secured_server("my-secret-token").await;

    let req = ManifestRequest {
        entry_point: "app".to_string(),
        cached_hashes: vec![],
        build_id: None,
    };
    let resp = server.post("/manifest").json(&req).await;

    assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED);
    let www_auth = resp
        .headers()
        .get("www-authenticate")
        .expect("WWW-Authenticate header should be present on 401")
        .to_str()
        .expect("WWW-Authenticate should be valid UTF-8");
    assert_eq!(www_auth, "Bearer", "WWW-Authenticate value should be 'Bearer'");
}
```

**Step 2: Run the tests and verify they all pass**

```bash
cargo test -p cloudpack-abs --test auth_test 2>&1
```

Expected: all tests pass, including the 5 SecretToken unit tests, 4 config unit tests, and 8 HTTP integration tests.

If any test fails, read the error message carefully — the most common causes are:
- Import not found: check that `use cloudpack_abs::...` paths match the actual module structure
- `add_header` not found on request builder: check `axum_test` v20 docs for the correct method name (may be `header(name, value)` instead)
- Status code mismatch: check that `require_bearer` is correctly wired in `build_router`

**Step 3: Run the full test suite one final time**

```bash
cargo test -p cloudpack-abs 2>&1
```

Expected: all tests pass. Zero regressions.

**Step 4: Commit**

```bash
git add crates/cloudpack-abs/tests/auth_test.rs
git commit -m "test(abs/security): add bearer token auth integration tests"
```

---

### Task 6: Final verification and clean-up commit

**Step 1: Check for any compiler warnings**

```bash
cargo clippy -p cloudpack-abs -- -D warnings 2>&1
```

Fix any warnings before continuing. Common ones to expect:
- Unused imports in `auth.rs` (e.g. if `fmt::Display` import is redundant — use `use std::fmt::{self, Display}`)
- Dead code warnings if `SecurityError` variants aren't used in tests

**Step 2: Confirm the full workspace builds cleanly**

```bash
cargo build --workspace 2>&1 | grep -E "^error|^warning"
```

Expected: only the pre-existing warnings (if any), no new ones.

**Step 3: Run the full test suite one more time**

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: all tests pass across all crates.

**Step 4: Commit any clippy fixes**

```bash
git add -A
git commit -m "fix(abs/security): address clippy warnings in security module"
```

(Only needed if Step 1 produced warnings that required changes.)

---

## Acceptance checklist

Before calling this complete, verify every item:

- [ ] `cargo test -p cloudpack-abs` passes with zero failures
- [ ] `cargo clippy -p cloudpack-abs -- -D warnings` passes with zero warnings
- [ ] `format!("{:?}", SecretToken::new(b"x".to_vec()))` does **not** contain `"x"`
- [ ] `format!("{}", SecretToken::new(b"x".to_vec()))` does **not** contain `"x"`
- [ ] `POST /manifest` without `Authorization` header returns 401 when security is enabled
- [ ] `POST /manifest` with correct `Authorization: Bearer <token>` returns non-401
- [ ] `GET /health` returns 200 even when security is enabled
- [ ] `GET /sw.js` returns 200 even when security is enabled
- [ ] Default `AbsConfig` (no `[security]` section) preserves existing unauthenticated behaviour
- [ ] `subtle::ConstantTimeEq` is the only comparison in `SecretToken::verify` — no `==` on bytes
- [ ] Token is read from a **file** path, never inline in config or environment

---

## What this plan does NOT include (deferred to separate plans)

| Feature | Plan |
|---|---|
| CORS allowlist | `2026-05-15-security-p1-cors.md` (not yet written) |
| Per-IP rate limiter (`governor`) | `2026-05-15-security-p1-rate-limit.md` (not yet written) |
| `CLOUDPACK_BEARER_TOKEN` env var | Deferred — file-based is sufficient at POC scale |
| Two-token grace period | Deferred — single-token for now, documented in design |
| Phase 2 signing / SRI / CSP | Blocked on VRC SCA deterministic `build_id` |
