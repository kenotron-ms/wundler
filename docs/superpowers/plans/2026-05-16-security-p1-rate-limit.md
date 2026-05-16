# Security P1.3 — Per-IP Rate Limiter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in, per-IP, in-process rate limiter to the Asset Bundling Server (`wundler-abs`), applied **only** to `POST /manifest`, so that abusive clients receive `429 Too Many Requests` without affecting `/health` or `/sw.js`.

**Architecture:** A `governor`-backed `RateLimiter<IpAddr, …>` is constructed once at startup inside `ResolvedSecurity::from_config` when `manifest_rate_per_sec` is present in `[security]`. A small Axum middleware reads the remote IP from `ConnectInfo<SocketAddr>` (already wired in production via `into_make_service_with_connect_info`). The middleware is mounted as a `.route_layer(...)` on the single `POST /manifest` route — it is **not** a global layer — so exempt routes are exempt by construction. When no `ConnectInfo` is present (e.g. `axum_test::TestServer`, which does not run a real TCP listener), the middleware passes the request through; this preserves test ergonomics and avoids a fail-open security regression because the production startup path always installs `ConnectInfo`.

**Tech Stack:** Rust (edition from workspace), `axum = 0.8`, `tower-http = 0.6`, `governor = 0.7` (new), `tokio`, `serde`, `thiserror`. Dev: `axum_test = 20`, `tempfile`. Cargo binary at `~/.cargo/bin/cargo`.

---

## File Structure

**New files:**
- `crates/wundler-abs/src/security/ratelimit.rs` — `IpRateLimiter` type alias, `build_limiter`, `rate_limit_mw` middleware, unit tests.

**Modified files:**
- `crates/wundler-abs/Cargo.toml` — add `governor = "0.7"`.
- `crates/wundler-abs/src/security/mod.rs` — add `pub mod ratelimit;`, two new fields on `SecurityConfig`, one new field on `ResolvedSecurity`, build the limiter in `from_config`.
- `crates/wundler-abs/src/server.rs` — attach `rate_limit_mw` as `.route_layer(...)` on the `POST /manifest` route inside `build_router` when `security.rate_limiter` is `Some`.

**Test files:**
- Unit tests live inline in `security/ratelimit.rs` (`#[cfg(test)] mod tests`).
- Integration tests live inline in `server.rs`'s existing `#[cfg(test)] mod tests` block, alongside the bearer-token integration tests added in P1.1.

---

## Task 1 — Dependencies, Config Fields, and Resolution

**Files:**
- Modify: `crates/wundler-abs/Cargo.toml`
- Modify: `crates/wundler-abs/src/security/mod.rs`
- Test: `crates/wundler-abs/src/security/mod.rs` (inline `#[cfg(test)] mod tests`)

### Step 1.1 — Add `governor` dependency

- [ ] **Edit `crates/wundler-abs/Cargo.toml`**

Add this line to the `[dependencies]` block, alphabetised under `e` / before `pkcs8`:

```toml
governor = "0.7"
```

Final fragment (insertion shown in context):

```toml
ed25519-dalek = { version = "2", features = ["pem", "rand_core"] }
governor = "0.7"
pkcs8 = { version = "0.10", features = ["pem"] }
```

- [ ] **Verify it resolves**

```bash
~/.cargo/bin/cargo build -p wundler-abs
```

Expected: clean build (no compile errors, no usage of the new crate yet so it may emit an `unused_crate_dependencies` warning if that lint is on — there is no such workspace lint, so build should be clean).

- [ ] **Commit**

```bash
git add crates/wundler-abs/Cargo.toml Cargo.lock
git commit -m "build(abs): add governor 0.7 dependency for P1.3 rate limiter"
```

---

### Step 1.2 — Write the failing test for new config fields and limiter construction

- [ ] **Edit `crates/wundler-abs/src/security/mod.rs`**

Add (or extend) an inline test module at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_no_rate_limit() {
        let cfg = SecurityConfig::default();
        assert!(cfg.manifest_rate_per_sec.is_none());
        assert!(cfg.manifest_rate_burst.is_none());

        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve default");
        assert!(resolved.rate_limiter.is_none());
    }

    #[test]
    fn rate_limit_config_produces_limiter() {
        let cfg = SecurityConfig {
            manifest_rate_per_sec: Some(5),
            manifest_rate_burst: Some(10),
            ..SecurityConfig::default()
        };
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert!(
            resolved.rate_limiter.is_some(),
            "limiter must be built when manifest_rate_per_sec is set"
        );
    }

    #[test]
    fn rate_limit_burst_defaults_to_rate_when_absent() {
        // When burst is omitted, it should fall back to the per-second rate so
        // configurators don't have to specify both.
        let cfg = SecurityConfig {
            manifest_rate_per_sec: Some(7),
            manifest_rate_burst: None,
            ..SecurityConfig::default()
        };
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert!(resolved.rate_limiter.is_some());
    }

    #[test]
    fn zero_rate_disables_limiter() {
        // A rate of 0 is nonsensical for `NonZeroU32`; we treat it the same as
        // "absent" rather than panicking at startup.
        let cfg = SecurityConfig {
            manifest_rate_per_sec: Some(0),
            manifest_rate_burst: Some(0),
            ..SecurityConfig::default()
        };
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert!(
            resolved.rate_limiter.is_none(),
            "rate=0 must disable the limiter, not panic"
        );
    }
}
```

- [ ] **Run the test — it must fail to compile**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib security::tests 2>&1 | head -40
```

Expected: compile errors like:
```
error[E0560]: struct `SecurityConfig` has no field named `manifest_rate_per_sec`
error[E0609]: no field `rate_limiter` on type `ResolvedSecurity`
```

This confirms the test exercises behaviour that does not yet exist.

---

### Step 1.3 — Add new fields to `SecurityConfig` and `ResolvedSecurity`

- [ ] **Edit `crates/wundler-abs/src/security/mod.rs`**

Replace the existing `SecurityConfig` struct with:

```rust
/// Configuration for the ABS security layer, parsed from `[security]` in
/// `wundler.toml`.
///
/// All fields are optional; `Default` produces a no-security configuration
/// that preserves today's unauthenticated behaviour.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct SecurityConfig {
    /// Path to a file containing the bearer token (one line, trimmed).
    ///
    /// Storing the token in a file (rather than inline in `wundler.toml`)
    /// prevents accidental commit and log leakage.
    ///
    /// If this field is absent, bearer-token authentication is disabled.
    pub bearer_token_file: Option<PathBuf>,

    /// Sustained per-IP request rate for `POST /manifest`, in requests per
    /// second. When `None` (or `Some(0)`), the rate limiter is disabled and
    /// today's unthrottled behaviour is preserved.
    pub manifest_rate_per_sec: Option<u32>,

    /// Burst budget for `POST /manifest`. Allows short spikes above
    /// `manifest_rate_per_sec`. When `None`, defaults to the same value as
    /// `manifest_rate_per_sec`.
    pub manifest_rate_burst: Option<u32>,
}
```

Replace `ResolvedSecurity` with:

```rust
/// The runtime-ready form of [`SecurityConfig`].
///
/// Created once at server startup by [`ResolvedSecurity::from_config`] and
/// then shared across all requests via `Arc`.
#[derive(Debug)]
pub struct ResolvedSecurity {
    /// The bearer token, or `None` if authentication is disabled.
    pub token: Option<SecretToken>,

    /// In-process, per-IP rate limiter for `POST /manifest`, or `None` if
    /// rate limiting is disabled.
    pub rate_limiter: Option<Arc<ratelimit::IpRateLimiter>>,
}
```

Add the `Arc` import and a forward declaration of the module. Replace the existing top-of-file imports and the `pub mod auth;` line with:

```rust
//! Security middleware for the Asset Bundling Server.
//!
//! This module provides optional bearer-token authentication and an optional
//! per-IP rate limiter, applied as Axum middleware layers. When the
//! `[security]` section is absent from `wundler.toml`, every middleware is a
//! transparent pass-through — existing behaviour is unchanged.

pub mod auth;
pub mod ratelimit;

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use crate::security::auth::SecretToken;
```

Replace the body of `ResolvedSecurity::from_config` with:

```rust
    /// Build a `ResolvedSecurity` from a [`SecurityConfig`].
    ///
    /// * When `config.bearer_token_file` is `Some`, the file is read
    ///   synchronously (startup path, not per-request) and trimmed.
    /// * When `config.manifest_rate_per_sec` is `Some` and non-zero, an
    ///   in-process [`ratelimit::IpRateLimiter`] is constructed. A burst
    ///   value of `None` falls back to the same number as the rate.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityError::TokenFileRead`] if the token file cannot be
    /// read.
    pub fn from_config(config: &SecurityConfig) -> Result<Self, SecurityError> {
        let token = match &config.bearer_token_file {
            None => None,
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| SecurityError::TokenFileRead(path.clone(), e))?;
                Some(SecretToken::new(raw.trim().as_bytes().to_vec()))
            }
        };

        let rate_limiter = match config.manifest_rate_per_sec {
            // `None` or `Some(0)` both disable the limiter; we never panic on
            // bad config — that's the operator's startup signal, but we treat
            // 0 as "off" rather than aborting.
            None | Some(0) => None,
            Some(rate) => {
                let rate = NonZeroU32::new(rate).expect("rate > 0 already checked");
                let burst = config
                    .manifest_rate_burst
                    .and_then(NonZeroU32::new)
                    .unwrap_or(rate);
                Some(ratelimit::build_limiter(rate, burst))
            }
        };

        Ok(Self {
            token,
            rate_limiter,
        })
    }
```

Update `is_enabled` to reflect the broader meaning of "any security feature on":

```rust
    /// Returns `true` if any security feature (bearer auth or rate limiting)
    /// is active.
    pub fn is_enabled(&self) -> bool {
        self.token.is_some() || self.rate_limiter.is_some()
    }
```

> **Note:** `is_enabled` is used today by `require_bearer` to short-circuit when auth is off. The semantic shift is harmless: with rate-limit-only configs, `is_enabled()` will now be `true`, but `require_bearer` falls through whenever `self.token.is_none()` is true via the `match` arm in `auth.rs` — it dereferences `self.token.as_ref().unwrap()` only after that check. Re-read `auth.rs` and confirm before continuing.

- [ ] **Read `auth.rs` to verify the `is_enabled` semantic shift is safe**

```bash
sed -n '100,120p' crates/wundler-abs/src/security/auth.rs
```

If the current code is:

```rust
    // If security is not configured, pass through.
    if !security.is_enabled() {
        return next.run(request).await;
    }
    …
    match token_str {
        Some(t) if security.token.as_ref().unwrap().verify(t) => next.run(request).await,
        _ => unauthorized_response(),
    }
```

then changing `is_enabled()` to also return `true` for rate-limit-only configs would cause `require_bearer` to hit the `match` with `security.token` being `None` and then call `.unwrap()` — a panic. **We must guard against that.**

- [ ] **Patch `require_bearer` to gate on `token.is_some()` directly**

Edit `crates/wundler-abs/src/security/auth.rs`, replace:

```rust
    // If security is not configured, pass through.
    if !security.is_enabled() {
        return next.run(request).await;
    }
```

with:

```rust
    // If bearer-token auth is not configured, pass through. (Other security
    // features such as the rate limiter are mounted as their own middleware
    // layers and are independent of this check.)
    if security.token.is_none() {
        return next.run(request).await;
    }
```

This makes `require_bearer` only sensitive to its own concern (the token), which is the correct factoring.

---

### Step 1.4 — Add a stub `ratelimit` module so the crate compiles

`from_config` references `ratelimit::IpRateLimiter` and `ratelimit::build_limiter`. We need a minimal stub so Task 1 compiles before Task 2 fleshes the module out.

- [ ] **Create `crates/wundler-abs/src/security/ratelimit.rs`** with this stub:

```rust
//! Per-IP, in-process rate limiter for `POST /manifest`.
//!
//! Full implementation (middleware + tests) lives in Task 2 of the
//! `2026-05-16-security-p1-rate-limit` plan. This file is a stub that
//! provides only the type alias and constructor needed by `from_config`.

use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{
    clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter,
};

/// Per-IP rate limiter alias.
///
/// `DefaultKeyedStateStore` is an in-memory dashmap keyed by `IpAddr`;
/// `DefaultClock` uses `quanta`'s monotonic clock.
pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Build a rate limiter that allows `rate` requests per second per IP, with a
/// short burst of up to `burst` requests.
///
/// Both arguments are `NonZeroU32` so misconfiguration is a startup-time
/// type error rather than a runtime divide-by-zero.
pub fn build_limiter(rate: NonZeroU32, burst: NonZeroU32) -> Arc<IpRateLimiter> {
    let quota = Quota::per_second(rate).allow_burst(burst);
    Arc::new(RateLimiter::keyed(quota))
}
```

- [ ] **Run the four new tests — they must pass**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib security::tests
```

Expected output (order may vary):

```
test security::tests::default_config_has_no_rate_limit ... ok
test security::tests::rate_limit_config_produces_limiter ... ok
test security::tests::rate_limit_burst_defaults_to_rate_when_absent ... ok
test security::tests::zero_rate_disables_limiter ... ok
```

- [ ] **Run the existing auth tests — they must still pass**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib
```

Expected: every existing test still green.

- [ ] **Run clippy — it must be clean**

```bash
~/.cargo/bin/cargo clippy -p wundler-abs --all-targets -- -D warnings
```

Expected: no warnings, no errors.

- [ ] **Commit**

```bash
git add crates/wundler-abs/Cargo.toml \
        crates/wundler-abs/src/security/mod.rs \
        crates/wundler-abs/src/security/auth.rs \
        crates/wundler-abs/src/security/ratelimit.rs
git commit -m "feat(abs/security): add rate-limit config fields and limiter builder

- SecurityConfig: add manifest_rate_per_sec, manifest_rate_burst
- ResolvedSecurity: add rate_limiter: Option<Arc<IpRateLimiter>>
- from_config: build limiter when rate is set (0 disables, no panic)
- auth: gate require_bearer on token presence directly, not is_enabled()
- security/ratelimit.rs: stub with IpRateLimiter alias and build_limiter()"
```

---

## Task 2 — `rate_limit_mw` Middleware and Unit Tests

**Files:**
- Modify: `crates/wundler-abs/src/security/ratelimit.rs`
- Test: inline `#[cfg(test)] mod tests` in the same file

### Step 2.1 — Write the failing middleware tests

- [ ] **Edit `crates/wundler-abs/src/security/ratelimit.rs`** — append this test module to the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        middleware,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use tower::ServiceExt; // for `oneshot`

    /// Build a tiny app: an outer `inject_ip` layer optionally seeds
    /// `ConnectInfo<SocketAddr>` into request extensions, then `rate_limit_mw`
    /// runs against a trivial `GET /` handler.
    fn app(limiter: Arc<IpRateLimiter>, ip: Option<IpAddr>) -> Router {
        let router = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                limiter,
                rate_limit_mw,
            ));

        match ip {
            None => router,
            Some(ip) => router.layer(middleware::from_fn(move |mut req: Request<Body>, next: middleware::Next| {
                let ip = ip;
                async move {
                    let ci: ConnectInfo<SocketAddr> =
                        ConnectInfo(SocketAddr::new(ip, 49_152));
                    req.extensions_mut().insert(ci);
                    next.run(req).await
                }
            })),
        }
    }

    async fn hit(app: &Router) -> StatusCode {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn skips_when_no_connect_info() {
        // 1 rps, burst 1 — would be very easy to trip if the limiter ran.
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, None);

        for _ in 0..20 {
            assert_eq!(hit(&app).await, StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn blocks_when_quota_exceeded_for_same_ip() {
        // 1 rps, burst 1 — one request fits, the next is denied.
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));

        assert_eq!(hit(&app).await, StatusCode::OK);

        let mut denied = 0;
        for _ in 0..10 {
            if hit(&app).await == StatusCode::TOO_MANY_REQUESTS {
                denied += 1;
            }
        }
        assert!(denied >= 5, "expected ≥5 denials within burst window, got {denied}");
    }

    #[tokio::test]
    async fn allows_burst_then_blocks() {
        // 1 rps, burst 5 — first five fit immediately.
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(5).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));

        for i in 0..5 {
            assert_eq!(
                hit(&app).await,
                StatusCode::OK,
                "burst request {i} must succeed"
            );
        }
        assert_eq!(hit(&app).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn different_ips_have_independent_buckets() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app_a = app(lim.clone(), Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        let app_b = app(lim,         Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));

        // First request from each IP fits.
        assert_eq!(hit(&app_a).await, StatusCode::OK);
        assert_eq!(hit(&app_b).await, StatusCode::OK);

        // Second request from each is denied (their own buckets are empty).
        assert_eq!(hit(&app_a).await, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(hit(&app_b).await, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn body_is_json_error_on_429() {
        let lim = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let app = app(lim, Some(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));

        // Consume the bucket.
        let _ = hit(&app).await;

        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes)
            .expect("429 body must be JSON");
        assert_eq!(json["error"], "rate limit exceeded");
    }

    // `_ = IntoResponse::into_response;` keeps the import alive when the
    // middleware module is edited and re-edited.
    #[allow(dead_code)]
    fn _force_import_into_response() -> impl IntoResponse { "" }
}
```

The test module needs `tower` as a dev-dep for `ServiceExt::oneshot`.

- [ ] **Add `tower` to `[dev-dependencies]` in `crates/wundler-abs/Cargo.toml`**

```toml
tower = { version = "0.5", features = ["util"] }
```

Final `[dev-dependencies]` block:

```toml
[dev-dependencies]
axum-test = "20"
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tower = { version = "0.5", features = ["util"] }
```

- [ ] **Run the new tests — they must fail (no `rate_limit_mw` yet)**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib security::ratelimit 2>&1 | head -40
```

Expected: compile errors such as:

```
error[E0425]: cannot find function `rate_limit_mw` in this scope
```

---

### Step 2.2 — Implement `rate_limit_mw`

- [ ] **Edit `crates/wundler-abs/src/security/ratelimit.rs`**

Replace the whole file with the following (preserves `IpRateLimiter` and `build_limiter` from Step 1.4, then adds the middleware):

```rust
//! Per-IP, in-process rate limiter for `POST /manifest`.
//!
//! # Design
//!
//! * Backed by [`governor::RateLimiter`] with a keyed in-memory state store.
//! * Keyed by the **socket peer address** (`ConnectInfo<SocketAddr>`). We do
//!   NOT honour `X-Forwarded-For`: spoofable upstream headers are a
//!   well-known foot-gun for IP-based throttling. Operators running behind a
//!   trusted proxy should terminate TLS / set their own throttle there.
//! * In-process only — no Redis, no cross-replica coordination. Two replicas
//!   each allow `rate` rps for the same IP. This is acceptable for the ABS,
//!   which is single-instance in every documented deployment.
//! * Applied as a `route_layer` on `POST /manifest` only, never globally.
//!   `/health` and `/sw.js` therefore cannot be throttled by construction.
//! * If the request has no `ConnectInfo` extension (only possible under
//!   `axum_test::TestServer`, which never opens a TCP socket), the middleware
//!   passes the request through. In production, `into_make_service_with_connect_info`
//!   guarantees this extension is always present.

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use governor::{
    clock::DefaultClock, state::keyed::DefaultKeyedStateStore, Quota, RateLimiter,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Type alias
// ---------------------------------------------------------------------------

/// Per-IP rate limiter alias.
///
/// `DefaultKeyedStateStore` is an in-memory dashmap keyed by `IpAddr`;
/// `DefaultClock` uses `quanta`'s monotonic clock.
pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a rate limiter that allows `rate` requests per second per IP, with a
/// short burst of up to `burst` requests.
///
/// Both arguments are `NonZeroU32` so misconfiguration is a startup-time type
/// error rather than a runtime divide-by-zero.
pub fn build_limiter(rate: NonZeroU32, burst: NonZeroU32) -> Arc<IpRateLimiter> {
    let quota = Quota::per_second(rate).allow_burst(burst);
    Arc::new(RateLimiter::keyed(quota))
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Axum middleware that throttles requests per source IP.
///
/// Behaviour:
/// * No `ConnectInfo<SocketAddr>` extension → pass through (test ergonomics;
///   never occurs in production).
/// * Otherwise consult the limiter for the peer IP; on `Ok` → continue, on
///   `Err` → return `429 Too Many Requests` with body
///   `{"error": "rate limit exceeded"}`.
pub async fn rate_limit_mw(
    State(lim): State<Arc<IpRateLimiter>>,
    req: Request,
    next: Next,
) -> Response {
    // Pull the peer socket address out of the request extensions. This is the
    // same data `MaybeConnectAddr` reads in `server.rs`.
    let ip: Option<IpAddr> = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    let Some(ip) = ip else {
        // No IP available — typically the `axum_test` in-memory transport.
        // Fail open in tests, never reachable in production.
        return next.run(req).await;
    };

    match lim.check_key(&ip) {
        Ok(()) => next.run(req).await,
        Err(_negative) => too_many_requests(),
    }
}

/// Build the standard 429 response.
fn too_many_requests() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({ "error": "rate limit exceeded" })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests (Step 2.1 wrote these against the stub; they live below.)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // ... (the test module from Step 2.1 is preserved here verbatim) ...
}
```

> When pasting, keep the entire `#[cfg(test)] mod tests { ... }` block from Step 2.1 exactly as it was. Only the non-test portion of the file is replaced.

- [ ] **Run the rate-limit tests — all five must pass**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib security::ratelimit::tests
```

Expected:

```
test security::ratelimit::tests::skips_when_no_connect_info ... ok
test security::ratelimit::tests::blocks_when_quota_exceeded_for_same_ip ... ok
test security::ratelimit::tests::allows_burst_then_blocks ... ok
test security::ratelimit::tests::different_ips_have_independent_buckets ... ok
test security::ratelimit::tests::body_is_json_error_on_429 ... ok
```

- [ ] **Run the full crate test suite — nothing else regressed**

```bash
~/.cargo/bin/cargo test -p wundler-abs
```

- [ ] **Clippy clean**

```bash
~/.cargo/bin/cargo clippy -p wundler-abs --all-targets -- -D warnings
```

- [ ] **Commit**

```bash
git add crates/wundler-abs/Cargo.toml \
        crates/wundler-abs/src/security/ratelimit.rs
git commit -m "feat(abs/security): add per-IP rate_limit_mw middleware

- rate_limit_mw extracts ConnectInfo<SocketAddr>, consults governor limiter
- 429 with JSON body {\"error\":\"rate limit exceeded\"} on deny
- Passes through when no ConnectInfo present (axum_test fallback)
- Tests cover: pass-through, burst, exhaustion, per-IP isolation, JSON body"
```

---

## Task 3 — Wire into `build_router` and Add HTTP Integration Tests

**Files:**
- Modify: `crates/wundler-abs/src/server.rs` (function `build_router`, plus the inline `#[cfg(test)] mod tests`)

### Step 3.1 — Write the failing integration tests

- [ ] **Read the existing test module in `server.rs` first**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib server::tests -- --list 2>&1 | head -20
sed -n '1,40p' crates/wundler-abs/src/server.rs
grep -n "mod tests" crates/wundler-abs/src/server.rs
```

This locates the existing `#[cfg(test)] mod tests` block (added during P1.1).

- [ ] **Append these tests to that existing test module** (do NOT create a new module). They use the same `tower::ServiceExt::oneshot` pattern as Task 2 so they don't need real TCP either.

```rust
    // -----------------------------------------------------------------
    // P1.3 — rate limiter integration tests
    // -----------------------------------------------------------------
    //
    // We can't use `axum_test::TestServer` here because it doesn't
    // populate `ConnectInfo<SocketAddr>`. Instead we drive the router
    // directly with `tower::ServiceExt::oneshot` and wrap it in a tiny
    // outer middleware that injects `ConnectInfo` into the request
    // extensions — exactly what `into_make_service_with_connect_info`
    // does in production.

    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::num::NonZeroU32;

    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        middleware,
    };
    use tower::ServiceExt;

    use crate::security::ratelimit::build_limiter;
    use crate::security::ResolvedSecurity;

    /// Build the production router and wrap it in a layer that injects the
    /// given IP as `ConnectInfo`. This mirrors what
    /// `into_make_service_with_connect_info::<SocketAddr>()` does at the
    /// real socket boundary.
    fn router_with_ip(
        app: super::AppState,
        telemetry: super::TelemetryLogger,
        security: std::sync::Arc<ResolvedSecurity>,
        ip: IpAddr,
    ) -> axum::Router {
        let inner = super::build_router(app, telemetry, security);
        inner.layer(middleware::from_fn(move |mut req: Request<Body>, next: middleware::Next| {
            let ip = ip;
            async move {
                let ci: ConnectInfo<SocketAddr> = ConnectInfo(SocketAddr::new(ip, 49_152));
                req.extensions_mut().insert(ci);
                next.run(req).await
            }
        }))
    }

    /// Minimal `AppState` + `TelemetryLogger` factory for these tests.
    ///
    /// **Implementer step:** before writing this helper, run
    /// `grep -n "AppState {" crates/wundler-abs/src/server.rs` and
    /// `grep -n "TelemetryLogger::new" crates/wundler-abs/src/server.rs`
    /// to find how the P1.1 bearer-token tests build these. If a helper
    /// already exists (e.g. `fn build_test_state()`), call it. Otherwise
    /// build them inline using the same construction the existing tests
    /// use — typically:
    ///
    /// ```ignore
    /// use std::sync::Arc;
    /// use tokio::sync::RwLock;
    /// use tempfile::NamedTempFile;
    /// use crate::manifest::ChunkManifest;
    ///
    /// fn test_state() -> (super::AppState, super::TelemetryLogger) {
    ///     let app = super::AppState {
    ///         manifest: Arc::new(RwLock::new(ChunkManifest::default())),
    ///         cdn_base_url: Arc::new(String::new()),
    ///         ttl_seconds: 60,
    ///     };
    ///     let log_file = NamedTempFile::new().unwrap();
    ///     let telemetry = super::TelemetryLogger::new(log_file.path()).unwrap();
    ///     (app, telemetry)
    /// }
    /// ```
    ///
    /// Match the **exact** types used by the P1.1 tests rather than guessing
    /// — `ChunkManifest::default()` vs `::new()` vs `::empty()` etc. is a
    /// detail that only the actual codebase knows.
    fn test_state() -> (super::AppState, super::TelemetryLogger) {
        // Replace this body with the actual construction discovered above.
        unimplemented!("see doc comment — copy from P1.1 test helpers")
    }

    #[tokio::test]
    async fn manifest_post_is_rate_limited() {
        let limiter = build_limiter(
            NonZeroU32::new(2).unwrap(),
            NonZeroU32::new(2).unwrap(),
        );
        let security = std::sync::Arc::new(ResolvedSecurity {
            token: None,
            rate_limiter: Some(limiter),
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        // First two POSTs fit in the burst.
        for i in 0..2 {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/manifest")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "request #{i} should not be rate limited"
            );
        }

        // Third must be 429.
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/manifest")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let bytes = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["error"], "rate limit exceeded");
    }

    #[tokio::test]
    async fn health_is_not_rate_limited() {
        // Very tight quota — if /health were behind the limiter it'd 429
        // immediately on the second request.
        let limiter = build_limiter(
            NonZeroU32::new(1).unwrap(),
            NonZeroU32::new(1).unwrap(),
        );
        let security = std::sync::Arc::new(ResolvedSecurity {
            token: None,
            rate_limiter: Some(limiter),
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        for i in 0..20 {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/health")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "/health request #{i} unexpectedly returned {}",
                resp.status()
            );
        }
    }

    #[tokio::test]
    async fn default_config_has_no_rate_limit() {
        // No rate_limiter in ResolvedSecurity → no throttling, regardless of
        // how many requests we send.
        let security = std::sync::Arc::new(ResolvedSecurity {
            token: None,
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let router = router_with_ip(
            app,
            telemetry,
            security,
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
        );

        for _ in 0..50 {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/manifest")
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                resp.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit must be off when ResolvedSecurity.rate_limiter is None"
            );
        }
    }
```

> **Implementer note on `test_state`:** The existing test module from P1.1 already has helpers that build an `AppState` (with an empty `ChunkManifest`) and a no-op `TelemetryLogger`. Find them by `grep -n "AppState {" crates/wundler-abs/src/server.rs` and reuse them. If they're named differently, alias them inside a small `mod test_helpers { … }` submodule rather than duplicating construction.

- [ ] **Run the new tests — they must fail**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib server::tests::manifest_post_is_rate_limited \
                                          server::tests::health_is_not_rate_limited \
                                          server::tests::default_config_has_no_rate_limit 2>&1 | tail -40
```

Expected: the `manifest_post_is_rate_limited` test fails because no 429 is ever produced — `build_router` does not yet attach the limiter. The other two should already pass (no throttling configured, or `/health` not on the limited route), but they exist now so we lock the contract in.

---

### Step 3.2 — Wire `rate_limit_mw` into `build_router`

- [ ] **Edit `crates/wundler-abs/src/server.rs`**

Locate `pub fn build_router(...)` at around line 193. Today it looks roughly like:

```rust
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    security: Arc<ResolvedSecurity>,
) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/sw.js", get(sw_js))
        .route("/manifest", post(post_manifest))
        .route("/reload", post(post_reload))
        // ... bearer auth + CORS layers ...
        .with_state(app)
}
```

Replace the `.route("/manifest", post(post_manifest))` line with a version that conditionally attaches a `route_layer` carrying `rate_limit_mw`:

```rust
    // Manifest route, optionally wrapped in the per-IP rate limiter. We use
    // `route_layer` (not `layer`) so the limiter only sees `POST /manifest`
    // traffic — `/health` and `/sw.js` are exempt by construction.
    let manifest_route = {
        let route = post(post_manifest);
        match security.rate_limiter.clone() {
            None => Router::new().route("/manifest", route),
            Some(limiter) => Router::new().route("/manifest", route).route_layer(
                axum::middleware::from_fn_with_state(
                    limiter,
                    crate::security::ratelimit::rate_limit_mw,
                ),
            ),
        }
    };
```

Then merge it into the main router:

```rust
    Router::new()
        .route("/health", get(health))
        .route("/sw.js", get(sw_js))
        .merge(manifest_route)
        .route("/reload", post(post_reload))
        // ... bearer auth + CORS layers, exactly as today ...
        .with_state(app)
```

The order matters:

1. `merge(manifest_route)` brings in `POST /manifest` already wrapped by `rate_limit_mw` via its `route_layer`.
2. The outer `Router` then applies global layers (bearer auth, CORS, trace) on top, so:
   * Bearer auth runs **before** rate limiting (an unauthenticated request is rejected with 401 without spending bucket capacity).
   * CORS preflights for `/manifest` go through CORS but never hit the rate limiter because `OPTIONS` is handled by `tower-http::cors` and bypasses route handlers.

> If the existing `build_router` already uses a different ordering for its bearer/CORS layers, **preserve** that ordering — only swap the `/manifest` route definition. Run the existing P1.1 bearer-auth tests after the edit to confirm 401 still wins over 429 (Step 3.3).

- [ ] **Run the three new integration tests — they must pass**

```bash
~/.cargo/bin/cargo test -p wundler-abs --lib server::tests::manifest_post_is_rate_limited \
                                          server::tests::health_is_not_rate_limited \
                                          server::tests::default_config_has_no_rate_limit
```

Expected: three `ok` lines.

---

### Step 3.3 — Full suite + clippy + commit

- [ ] **Run the entire crate test suite — nothing regressed**

```bash
~/.cargo/bin/cargo test -p wundler-abs
```

Expected: all tests green, including the P1.1 bearer-token suite and the P1.2 CORS suite.

- [ ] **Run clippy across the workspace for this crate**

```bash
~/.cargo/bin/cargo clippy -p wundler-abs --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Run rustfmt** (to keep diffs minimal in review)

```bash
~/.cargo/bin/cargo fmt -p wundler-abs
git diff --stat
```

- [ ] **Smoke-test the production startup path compiles**

```bash
~/.cargo/bin/cargo build -p wundler-abs --release 2>&1 | tail -10
```

Expected: clean build (release profile catches a different class of lints).

- [ ] **Commit**

```bash
git add crates/wundler-abs/src/server.rs
git commit -m "feat(abs/security): mount per-IP rate limiter on POST /manifest

- build_router attaches rate_limit_mw as a route_layer on /manifest only
- /health and /sw.js are exempt by construction (never reach the layer)
- When ResolvedSecurity.rate_limiter is None, behaviour is unchanged
- Integration tests cover 429 on burst exhaustion, /health exemption,
  and the no-config baseline"
```

- [ ] **Push (or hand off for PR per project convention)**

```bash
git log --oneline -3
```

Expected three commits, one per task.

---

## Acceptance Check

Run all four acceptance criteria one final time and paste the output into the PR description:

```bash
# 1. crate tests
~/.cargo/bin/cargo test -p wundler-abs

# 2. clippy clean (per security-baseline standard)
~/.cargo/bin/cargo clippy -p wundler-abs --all-targets -- -D warnings

# 3. specific behavioural assertions (these were green in Task 3 but
#    re-run them by name so the PR shows the named acceptance tests)
~/.cargo/bin/cargo test -p wundler-abs --lib \
    server::tests::manifest_post_is_rate_limited \
    server::tests::health_is_not_rate_limited \
    server::tests::default_config_has_no_rate_limit
```

Expected: every command exits 0, every targeted test prints `ok`.

---

## Notes for the Reviewer / Future Work (out of scope here)

* **No `X-Forwarded-For` support.** Deliberate. Spoofable upstream headers make per-IP throttling worthless. When the ABS is fronted by a trusted reverse proxy, do throttling there (e.g. NGINX `limit_req_zone`, Cloud LB).
* **No distributed coordination.** Two ABS replicas each allow `rate` rps per IP independently. Acceptable for the single-instance deployment posture; revisit if/when ABS scales horizontally.
* **`/reload` is not rate-limited.** It's an internal admin endpoint and is already gated by bearer auth in P1.1. Adding throttling there is a separate, simpler change once we agree on a quota.
* **No `Retry-After` header on 429.** `governor` does expose the wait duration via the `NegativeMultiDecision` error; we elide it here to keep the body minimal and avoid signalling exact bucket state to abusive clients. Trivial follow-up if operators want it.
* **`is_enabled()` semantic shift.** Previously meant "bearer auth is on"; now means "any security feature is on". `require_bearer` was patched in Step 1.3 to gate on `token.is_some()` directly so it does not depend on this broader meaning.
