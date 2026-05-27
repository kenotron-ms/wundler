# Security P1.2 — CORS Allowlist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a strict CORS allowlist to the Asset Bundling Server (ABS) so cross-origin browser requests are accepted only from explicitly listed origins (no wildcard, ever).

**Architecture:** Extend the existing `SecurityConfig` / `ResolvedSecurity` pair with an `allowed_origins` field that is parsed from `cloudpack.toml` into pre-validated `HeaderValue`s at startup. A new `security::cors` module returns a `tower_http::cors::CorsLayer` that is wired as the **outermost** layer in `build_router` so browsers receive CORS headers even on 401 responses. Empty list = no `Access-Control-Allow-Origin` header → browsers block by default. No credentials, ever.

**Tech Stack:** Rust, Axum, `tower-http` (already vendored with `features = ["cors", "trace"]`), `axum_test = "20"` for integration tests, `serde` for TOML deserialization.

---

## File Structure

**Modify:**
- `crates/cloudpack-abs/src/security/mod.rs` — add `allowed_origins: Vec<String>` to `SecurityConfig`; add `allowed_origins: Vec<HeaderValue>` to `ResolvedSecurity`; extend `SecurityError`; parse origins in `from_config`.
- `crates/cloudpack-abs/src/server.rs` — wire `build_cors(&security)` as the outermost layer in `build_router`.

**Create:**
- `crates/cloudpack-abs/src/security/cors.rs` — `build_cors(sec: &ResolvedSecurity) -> CorsLayer`.
- `crates/cloudpack-abs/tests/cors_test.rs` — HTTP integration tests covering allowed/blocked/preflight/empty-list behaviour.

**Boundary — DO NOT TOUCH:**
- Rate limiter (P1.3) — separate plan.
- Phase 2 signing / SRI / CSP — separate plans.
- Env var token source — separate plan.

---

## Task 1: Extend `SecurityConfig` and `ResolvedSecurity` with `allowed_origins`

**Files:**
- Modify: `crates/cloudpack-abs/src/security/mod.rs`
- Test (inline `#[cfg(test)]` module at the bottom of the same file)

This task does **not** touch the router. It only extends the config types and the `from_config` resolver, plus unit tests for origin parsing. All existing call sites still compile because the new field has `#[serde(default)]` and `ResolvedSecurity` already has at least one public field constructor we keep working.

- [ ] **Step 1.1: Write the failing test for "empty origins → empty Vec"**

Append this block to the **bottom** of `crates/cloudpack-abs/src/security/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_origins_resolve_to_empty_vec() {
        let cfg = SecurityConfig::default();
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert!(resolved.allowed_origins.is_empty());
    }

    #[test]
    fn valid_origins_parse_to_header_values() {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec![
                "https://app.example.com".to_string(),
                "https://staging.example.com".to_string(),
            ],
        };
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert_eq!(resolved.allowed_origins.len(), 2);
        assert_eq!(
            resolved.allowed_origins[0].to_str().unwrap(),
            "https://app.example.com"
        );
        assert_eq!(
            resolved.allowed_origins[1].to_str().unwrap(),
            "https://staging.example.com"
        );
    }

    #[test]
    fn wildcard_origin_is_rejected() {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec!["*".to_string()],
        };
        let err = ResolvedSecurity::from_config(&cfg).expect_err("wildcard must fail");
        match err {
            SecurityError::InvalidOrigin(s) => assert_eq!(s, "*"),
            other => panic!("expected InvalidOrigin, got {other:?}"),
        }
    }

    #[test]
    fn invalid_header_value_origin_is_rejected() {
        // A bare newline is not a valid HTTP header value.
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec!["https://app.example.com\n".to_string()],
        };
        let err = ResolvedSecurity::from_config(&cfg).expect_err("newline must fail");
        assert!(matches!(err, SecurityError::InvalidOrigin(_)));
    }
}
```

- [ ] **Step 1.2: Run the test to confirm it fails to compile**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs --lib security::tests 2>&1 | head -40
```
Expected: compile error referencing `SecurityConfig` missing field `allowed_origins`, `ResolvedSecurity` missing field `allowed_origins`, and unknown variant `SecurityError::InvalidOrigin`.

- [ ] **Step 1.3: Add `allowed_origins` to `SecurityConfig`**

In `crates/cloudpack-abs/src/security/mod.rs`, replace the entire `SecurityConfig` struct definition with:

```rust
/// Configuration for the ABS security layer, parsed from `[security]` in
/// `cloudpack.toml`.
///
/// All fields are optional; `Default` produces a no-security configuration
/// that preserves today's unauthenticated behaviour.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct SecurityConfig {
    /// Path to a file containing the bearer token (one line, trimmed).
    ///
    /// Storing the token in a file (rather than inline in `cloudpack.toml`)
    /// prevents accidental commit and log leakage.
    ///
    /// If this field is absent, bearer-token authentication is disabled.
    pub bearer_token_file: Option<PathBuf>,

    /// Allowlist of cross-origin request `Origin` header values.
    ///
    /// Empty (default) means **no** cross-origin requests are accepted —
    /// the CORS layer returns no `Access-Control-Allow-Origin` header and
    /// browsers will block the response.
    ///
    /// Wildcards (`"*"`) are **never** accepted; see
    /// `docs/superpowers/security-baseline.md` (P1.2).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}
```

- [ ] **Step 1.4: Add the `InvalidOrigin` variant to `SecurityError`**

Replace the existing `SecurityError` enum in `crates/cloudpack-abs/src/security/mod.rs` with:

```rust
/// Errors that can occur while resolving a [`SecurityConfig`].
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// The bearer token file could not be read.
    #[error("failed to read bearer token file at {0}: {1}")]
    TokenFileRead(PathBuf, std::io::Error),

    /// An entry in `allowed_origins` is not a valid HTTP header value, or
    /// is the literal wildcard `"*"` (never accepted — see P1.2).
    #[error("invalid allowed_origins entry: {0:?}")]
    InvalidOrigin(String),
}
```

- [ ] **Step 1.5: Extend `ResolvedSecurity` and `from_config`**

Replace the entire `ResolvedSecurity` block (struct + `impl`) in `crates/cloudpack-abs/src/security/mod.rs` with:

```rust
/// The runtime-ready form of [`SecurityConfig`].
///
/// Created once at server startup by [`ResolvedSecurity::from_config`] and
/// then shared across all requests via `Arc`.
#[derive(Debug)]
pub struct ResolvedSecurity {
    /// The bearer token, or `None` if authentication is disabled.
    pub token: Option<SecretToken>,

    /// Pre-validated CORS allowlist. Empty means "deny all cross-origin".
    ///
    /// Stored as `HeaderValue` so the CORS layer can compare without
    /// re-parsing on every request.
    pub allowed_origins: Vec<axum::http::HeaderValue>,
}

impl ResolvedSecurity {
    /// Build a `ResolvedSecurity` from a [`SecurityConfig`].
    ///
    /// When `config.bearer_token_file` is `Some`, the file is read
    /// synchronously (startup path, not per-request) and the content is
    /// trimmed of surrounding whitespace before being stored.
    ///
    /// Each entry of `config.allowed_origins` is validated as an HTTP
    /// header value. The literal `"*"` is **explicitly rejected** — wildcard
    /// CORS is never accepted (see security-baseline.md, P1.2).
    ///
    /// # Errors
    ///
    /// * [`SecurityError::TokenFileRead`] — the token file cannot be read.
    /// * [`SecurityError::InvalidOrigin`] — an `allowed_origins` entry is
    ///   `"*"` or contains characters illegal in an HTTP header value.
    pub fn from_config(config: &SecurityConfig) -> Result<Self, SecurityError> {
        let token = match &config.bearer_token_file {
            None => None,
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| SecurityError::TokenFileRead(path.clone(), e))?;
                Some(SecretToken::new(raw.trim().as_bytes().to_vec()))
            }
        };

        let mut allowed_origins = Vec::with_capacity(config.allowed_origins.len());
        for origin in &config.allowed_origins {
            if origin == "*" {
                return Err(SecurityError::InvalidOrigin(origin.clone()));
            }
            let hv = axum::http::HeaderValue::from_str(origin)
                .map_err(|_| SecurityError::InvalidOrigin(origin.clone()))?;
            allowed_origins.push(hv);
        }

        Ok(Self {
            token,
            allowed_origins,
        })
    }

    /// Returns `true` if bearer-token authentication is active.
    pub fn is_enabled(&self) -> bool {
        self.token.is_some()
    }
}
```

- [ ] **Step 1.6: Run the tests to confirm they pass**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs --lib security::tests 2>&1 | tail -20
```
Expected: `test result: ok. 4 passed; 0 failed`.

- [ ] **Step 1.7: Confirm the whole crate still builds and lints clean**

Run:
```bash
~/.cargo/bin/cargo build -p cloudpack-abs 2>&1 | tail -10
~/.cargo/bin/cargo clippy -p cloudpack-abs --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: both finish with no warnings, no errors. (Existing call sites that construct `ResolvedSecurity` via `from_config` are unaffected; no call site builds the struct literally.)

- [ ] **Step 1.8: Commit**

```bash
git add crates/cloudpack-abs/src/security/mod.rs
git commit -m "feat(abs/security): add allowed_origins to SecurityConfig and ResolvedSecurity

Parses and validates CORS allowlist origins at startup. Wildcard ('*')
and malformed header values are rejected with SecurityError::InvalidOrigin.
Default (empty list) preserves existing behaviour.

Part of P1.2 — CORS allowlist (security-baseline.md)."
```

---

## Task 2: Create `security/cors.rs` with `build_cors()`

**Files:**
- Create: `crates/cloudpack-abs/src/security/cors.rs`
- Modify: `crates/cloudpack-abs/src/security/mod.rs` (add `pub mod cors;`)

This task introduces the CORS layer builder and its unit tests. The router is **not** touched yet — that's Task 3.

- [ ] **Step 2.1: Write the failing test file**

Create `crates/cloudpack-abs/src/security/cors.rs` with the following content. The tests are written first; the `build_cors` function is a one-line stub that intentionally fails the assertion.

```rust
//! CORS allowlist layer for the Asset Bundling Server.
//!
//! Builds a `tower_http::cors::CorsLayer` from a resolved security config.
//! Empty allowlist = no `Access-Control-Allow-Origin` header is ever set →
//! browsers block the response. There is no wildcard mode.
//!
//! See `docs/superpowers/security-baseline.md` (P1.2).

use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::ResolvedSecurity;

/// Build the CORS layer for a given resolved security config.
///
/// * When `sec.allowed_origins` is empty, the returned layer **adds no
///   CORS headers**. Cross-origin requests will be blocked by the browser
///   because no `Access-Control-Allow-Origin` header is returned.
/// * When non-empty, the layer matches the request `Origin` against the
///   exact list (no patterns, no subdomain wildcards) and reflects the
///   match into `Access-Control-Allow-Origin`.
///
/// Fixed policy (not configurable, by design):
/// * Methods: `GET`, `POST`
/// * Request headers: `Authorization`, `Content-Type`
/// * Preflight cache: 5 minutes
/// * `Access-Control-Allow-Credentials`: **never** sent
pub fn build_cors(sec: &ResolvedSecurity) -> CorsLayer {
    if sec.allowed_origins.is_empty() {
        // Zero-config layer: no Access-Control-* headers will be emitted.
        return CorsLayer::new();
    }

    let origins: Vec<HeaderValue> = sec.allowed_origins.clone();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(300))
    // NOTE: deliberately no `.allow_credentials(true)` — see P1.2.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecurityConfig;

    fn resolved_with(origins: &[&str]) -> ResolvedSecurity {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: origins.iter().map(|s| s.to_string()).collect(),
        };
        ResolvedSecurity::from_config(&cfg).expect("resolve")
    }

    #[test]
    fn empty_allowlist_returns_a_layer() {
        // We can't introspect CorsLayer's internals; this test guards against
        // panics inside build_cors when the list is empty. Behavioural checks
        // live in tests/cors_test.rs.
        let sec = resolved_with(&[]);
        let _layer: CorsLayer = build_cors(&sec);
    }

    #[test]
    fn non_empty_allowlist_returns_a_layer() {
        let sec = resolved_with(&["https://app.example.com"]);
        let _layer: CorsLayer = build_cors(&sec);
    }

    #[test]
    fn multi_origin_allowlist_returns_a_layer() {
        let sec = resolved_with(&[
            "https://app.example.com",
            "https://staging.example.com",
        ]);
        let _layer: CorsLayer = build_cors(&sec);
    }
}
```

- [ ] **Step 2.2: Register the new module**

Replace the `pub mod auth;` line near the top of `crates/cloudpack-abs/src/security/mod.rs` with:

```rust
pub mod auth;
pub mod cors;
```

- [ ] **Step 2.3: Run the module unit tests**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs --lib security::cors 2>&1 | tail -15
```
Expected: `test result: ok. 3 passed; 0 failed`.

- [ ] **Step 2.4: Clippy check**

Run:
```bash
~/.cargo/bin/cargo clippy -p cloudpack-abs --all-targets -- -D warnings 2>&1 | tail -10
```
Expected: clean — no warnings.

- [ ] **Step 2.5: Commit**

```bash
git add crates/cloudpack-abs/src/security/mod.rs crates/cloudpack-abs/src/security/cors.rs
git commit -m "feat(abs/security): add build_cors() CORS layer builder

Empty allowlist produces a no-op CorsLayer (browser blocks).
Non-empty produces exact-match AllowOrigin::list with fixed
GET/POST + Authorization/Content-Type policy and 5-min preflight cache.
Credentials are never allowed.

Part of P1.2 — CORS allowlist."
```

---

## Task 3: Wire `build_cors` into `build_router` (outermost layer) + integration tests

**Files:**
- Modify: `crates/cloudpack-abs/src/server.rs` (the `build_router` function only)
- Create: `crates/cloudpack-abs/tests/cors_test.rs`

The CORS layer must be **outer** — added with `.layer(build_cors(...))` **after** the bearer-token layer in the builder chain — so browsers receive `Access-Control-Allow-Origin` even when the response is a `401` from the auth layer. Tower's layer-application order is "last `.layer()` wraps first", so the *last* `.layer()` call in the chain ends up *outermost* at runtime.

- [ ] **Step 3.1: Write the failing integration test file**

Create `crates/cloudpack-abs/tests/cors_test.rs` with:

```rust
//! HTTP integration tests for the CORS allowlist (P1.2).
//!
//! These exercise `build_router` end-to-end via `axum_test::TestServer`.

use std::path::PathBuf;
use std::sync::Arc;

use axum_test::TestServer;
use serde_json::json;
use cloudpack_abs::security::{ResolvedSecurity, SecurityConfig};
use cloudpack_abs::server::build_router;
use cloudpack_abs::state::AppState;
use cloudpack_abs::telemetry::TelemetryLogger;
use cloudpack_graph::ChunkManifest;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn empty_manifest_json() -> String {
    serde_json::to_string(&ChunkManifest {
        build_id: "test-build".to_string(),
        chunks: Default::default(),
        entry_chunks: Default::default(),
    })
    .expect("serialize empty manifest")
}

async fn make_server(allowed_origins: Vec<String>) -> TestServer {
    // Manifest file
    let tmp = tempfile::tempdir().expect("tmpdir");
    let manifest_path = tmp.path().join("manifest.json");
    std::fs::write(&manifest_path, empty_manifest_json()).expect("write manifest");

    let app = AppState::load_from_disk(
        &manifest_path,
        "https://cdn.example.com".to_string(),
        300,
    )
    .await
    .expect("load app state");

    let telem_path = tmp.path().join("telemetry.jsonl");
    let telemetry = TelemetryLogger::new(&telem_path).expect("telemetry");

    let security = ResolvedSecurity::from_config(&SecurityConfig {
        bearer_token_file: None,
        allowed_origins,
    })
    .expect("resolve security");

    // Keep tmpdir alive for the lifetime of the test by leaking it.
    // axum_test does not give us a hook to drop it later, and these temp
    // files are tiny.
    std::mem::forget(tmp);

    let router = build_router(app, telemetry, Arc::new(security));
    TestServer::new(router).expect("test server")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_allowlist_emits_no_cors_header_on_simple_request() {
    let server = make_server(vec![]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://app.example.com")
        .await;

    resp.assert_status_ok();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "empty allowlist must not emit Access-Control-Allow-Origin"
    );
}

#[tokio::test]
async fn allowlisted_origin_is_reflected_in_response() {
    let server = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://app.example.com")
        .await;

    resp.assert_status_ok();
    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("Access-Control-Allow-Origin must be present")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");
}

#[tokio::test]
async fn non_allowlisted_origin_gets_no_cors_header() {
    let server = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .get("/health")
        .add_header("origin", "https://evil.example.com")
        .await;

    // The request still succeeds (CORS is enforced by the *browser*, not the
    // server) but no Access-Control-Allow-Origin header is set, so the
    // browser will refuse to expose the body to the page.
    resp.assert_status_ok();
    assert!(
        resp.headers().get("access-control-allow-origin").is_none(),
        "non-allowlisted origin must NOT receive Access-Control-Allow-Origin"
    );
}

#[tokio::test]
async fn preflight_options_for_allowlisted_origin_returns_cors_headers() {
    let server = make_server(vec!["https://app.example.com".to_string()]).await;

    let resp = server
        .method(axum::http::Method::OPTIONS, "/manifest")
        .add_header("origin", "https://app.example.com")
        .add_header("access-control-request-method", "POST")
        .add_header("access-control-request-headers", "authorization,content-type")
        .await;

    // tower-http's CORS layer answers preflight OPTIONS directly with 200.
    resp.assert_status_ok();

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("preflight must echo Access-Control-Allow-Origin")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");

    let methods = resp
        .headers()
        .get("access-control-allow-methods")
        .expect("preflight must include Access-Control-Allow-Methods")
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    assert!(methods.contains("GET"), "GET must be allowed, got: {methods}");
    assert!(methods.contains("POST"), "POST must be allowed, got: {methods}");

    let max_age = resp
        .headers()
        .get("access-control-max-age")
        .expect("preflight must include Access-Control-Max-Age")
        .to_str()
        .unwrap();
    assert_eq!(max_age, "300", "max_age must be 300s (5 minutes)");

    assert!(
        resp.headers().get("access-control-allow-credentials").is_none(),
        "Access-Control-Allow-Credentials must NEVER be set"
    );
}

#[tokio::test]
async fn cors_layer_is_outer_so_401_responses_still_carry_cors_header() {
    // Enable bearer auth AND CORS. A request from the allowlisted origin
    // without an Authorization header should get 401 from the auth layer
    // but still get Access-Control-Allow-Origin from the outer CORS layer.
    let tmp = tempfile::tempdir().expect("tmpdir");
    let token_path: PathBuf = tmp.path().join("token");
    std::fs::write(&token_path, "s3cret").expect("write token");

    let manifest_path = tmp.path().join("manifest.json");
    std::fs::write(&manifest_path, empty_manifest_json()).expect("write manifest");

    let app = AppState::load_from_disk(
        &manifest_path,
        "https://cdn.example.com".to_string(),
        300,
    )
    .await
    .expect("load app state");

    let telem_path = tmp.path().join("telemetry.jsonl");
    let telemetry = TelemetryLogger::new(&telem_path).expect("telemetry");

    let security = ResolvedSecurity::from_config(&SecurityConfig {
        bearer_token_file: Some(token_path),
        allowed_origins: vec!["https://app.example.com".to_string()],
    })
    .expect("resolve security");

    std::mem::forget(tmp);

    let router = build_router(app, telemetry, Arc::new(security));
    let server = TestServer::new(router).expect("test server");

    // POST /manifest with allowlisted Origin but no Authorization → 401.
    let resp = server
        .post("/manifest")
        .add_header("origin", "https://app.example.com")
        .json(&json!({
            "entry_point": "main",
            "cached_hashes": [],
            "build_id": null,
        }))
        .await;

    assert_eq!(resp.status_code(), axum::http::StatusCode::UNAUTHORIZED);

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("401 response must still carry Access-Control-Allow-Origin (CORS is outer)")
        .to_str()
        .unwrap();
    assert_eq!(acao, "https://app.example.com");
}
```

- [ ] **Step 3.2: Ensure `tempfile` is available as a dev-dependency**

Run:
```bash
~/.cargo/bin/cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | grep -o '"tempfile"' | head -1
```
If the output is empty, add tempfile to dev-deps:
```bash
~/.cargo/bin/cargo add -p cloudpack-abs --dev tempfile
```
Otherwise skip the add. (Most workspaces already pull in tempfile.)

- [ ] **Step 3.3: Run the integration tests and confirm they fail**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs --test cors_test 2>&1 | tail -40
```
Expected: tests compile but `allowlisted_origin_is_reflected_in_response`, `preflight_options_for_allowlisted_origin_returns_cors_headers`, and `cors_layer_is_outer_so_401_responses_still_carry_cors_header` **FAIL** because `build_router` does not yet apply the CORS layer. The `empty_allowlist_*` and `non_allowlisted_*` tests should pass already (they assert absence of the header).

- [ ] **Step 3.4: Wire `build_cors` into `build_router`**

In `crates/cloudpack-abs/src/server.rs`, update the imports near the top:

Replace:
```rust
use crate::security::auth::require_bearer;
use crate::security::{ResolvedSecurity, SecurityConfig};
```
with:
```rust
use crate::security::auth::require_bearer;
use crate::security::cors::build_cors;
use crate::security::{ResolvedSecurity, SecurityConfig};
```

Then replace the entire `build_router` function with:

```rust
/// Build and return the Axum [`Router`] without starting a listener.
///
/// Separating router construction from server startup makes the router
/// independently testable without needing a live TCP socket.
///
/// Layer order (outer → inner at runtime):
///   1. CORS allowlist (P1.2)  ← outermost, so 401 responses still carry
///                                Access-Control-Allow-Origin.
///   2. Bearer-token auth      ← inner.
///
/// The `security` parameter controls both bearer-token authentication and
/// the CORS allowlist. When neither is configured (the default when no
/// `[security]` section is present in `cloudpack.toml`) both middlewares are
/// transparent pass-throughs and existing behaviour is unchanged.
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    security: Arc<ResolvedSecurity>,
) -> Router {
    let state = RouterState { app, telemetry };

    // Build the CORS layer up front; `build_cors` only reads `allowed_origins`.
    let cors = build_cors(&security);

    Router::new()
        .route("/manifest", post(post_manifest))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        // Inner: bearer-token authentication.
        .layer(middleware::from_fn_with_state(security, require_bearer))
        // Outer: CORS allowlist. Applied last → wraps the auth layer, so
        // even a 401 from `require_bearer` carries Access-Control-* headers.
        .layer(cors)
        .with_state(state)
}
```

- [ ] **Step 3.5: Run the integration tests and confirm they all pass**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs --test cors_test 2>&1 | tail -20
```
Expected: `test result: ok. 5 passed; 0 failed`.

- [ ] **Step 3.6: Run the full crate test suite**

Run:
```bash
~/.cargo/bin/cargo test -p cloudpack-abs 2>&1 | tail -30
```
Expected: every existing test (auth_test, http_integration_test, unit tests, cors_test) passes — `0 failed` across all binaries.

If `tests/auth_test.rs` or `tests/http_integration_test.rs` constructs `ResolvedSecurity` via a struct literal (rather than `from_config`), they will fail to compile because of the new `allowed_origins` field. **Fix by switching to `from_config`**, or by adding `allowed_origins: vec![]` to the literal:

```bash
grep -n 'ResolvedSecurity {' crates/cloudpack-abs/tests/ crates/cloudpack-abs/src/ 2>/dev/null
```
For each match that is a struct literal, add `, allowed_origins: vec![]` before the closing brace. Re-run the test suite until it is green.

- [ ] **Step 3.7: Run clippy across all targets with `-D warnings`**

Run:
```bash
~/.cargo/bin/cargo clippy -p cloudpack-abs --all-targets -- -D warnings 2>&1 | tail -15
```
Expected: clean — finishes with no warnings emitted at the deny level.

- [ ] **Step 3.8: Commit**

```bash
git add crates/cloudpack-abs/src/server.rs crates/cloudpack-abs/tests/cors_test.rs
# Include any test-file edits made in Step 3.6:
git add -u crates/cloudpack-abs/tests/
# If `cargo add --dev tempfile` modified Cargo.toml / Cargo.lock:
git add crates/cloudpack-abs/Cargo.toml Cargo.lock 2>/dev/null || true

git commit -m "feat(abs/security): wire CORS allowlist as outermost router layer

build_router now applies build_cors(&security) after the bearer-token
layer, so .layer() ordering places CORS outermost at runtime. Browsers
receive Access-Control-Allow-Origin even on 401 responses.

Integration tests cover:
  * empty allowlist → no ACAO header
  * allowlisted origin → ACAO reflected
  * non-allowlisted origin → no ACAO header
  * OPTIONS preflight returns methods/max-age, no credentials
  * 401 from auth layer still carries ACAO (outer-layer guarantee)

Closes P1.2 — CORS allowlist (security-baseline.md)."
```

---

## Self-Review Checklist (completed during plan authoring)

1. **Spec coverage**
   - `allowed_origins: Vec<String>` with `#[serde(default)]` on `SecurityConfig` — Task 1, Step 1.3.
   - `allowed_origins: Vec<HeaderValue>` on `ResolvedSecurity` — Task 1, Step 1.5.
   - Parsing into `Vec<HeaderValue>` with wildcard rejection — Task 1, Step 1.5 + tests in Step 1.1.
   - `security/cors.rs` with `build_cors(sec: &ResolvedSecurity) -> CorsLayer` — Task 2, Step 2.1.
   - Empty list → `CorsLayer::new()` — Task 2, Step 2.1 + integration test in Task 3, Step 3.1.
   - Non-empty → `AllowOrigin::list(...)` — Task 2, Step 2.1.
   - Methods GET/POST, headers Authorization/Content-Type, `max_age = 300s`, no credentials — Task 2, Step 2.1 + preflight test in Task 3, Step 3.1.
   - CORS as outermost layer — Task 3, Step 3.4, with explicit 401-carries-CORS test in Step 3.1.

2. **Acceptance criteria mapping**
   - `cargo test -p cloudpack-abs` passes — Task 3, Step 3.6.
   - `cargo clippy -p cloudpack-abs -- -D warnings` passes — Task 3, Step 3.7.
   - Non-allowlisted origin → no ACAO — `non_allowlisted_origin_gets_no_cors_header`.
   - Allowlisted origin → ACAO reflected — `allowlisted_origin_is_reflected_in_response`.
   - Default config → no ACAO, no breaking change — `empty_allowlist_emits_no_cors_header_on_simple_request` + Step 3.6 ensures all pre-existing tests still pass.

3. **Out-of-scope guarded**
   - Rate limiter (P1.3): not added.
   - Signing / SRI / CSP: not added.
   - Env var token source: not added.

4. **Type / signature consistency**
   - `SecurityConfig.allowed_origins: Vec<String>` consistent across Tasks 1–3.
   - `ResolvedSecurity.allowed_origins: Vec<axum::http::HeaderValue>` consistent across Tasks 1–3 (cors.rs imports `axum::http::HeaderValue`, which is the same re-export as `http::HeaderValue`).
   - `build_cors(sec: &ResolvedSecurity) -> CorsLayer` consistent between Task 2 definition and Task 3 use site.
   - `SecurityError::InvalidOrigin(String)` defined in Step 1.4, asserted in Step 1.1.
