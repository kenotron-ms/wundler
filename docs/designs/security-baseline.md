# Security Baseline — System Design

**Status:** DRAFT — POC stage, security review pending
**Depends on:** `versioned-runtime-control.md` (Phase 2 only)
**Owners:** ABS team
**Date:** 2026-05-15

---

## Summary

ABS today is unauthenticated, unrate-limited, and CORS-unrestricted. A compromised or
spoofed manifest is **equivalent to arbitrary JS execution in every client's browser** —
this is the highest-severity vector in the entire system.

This design specifies two phases:

- **Phase 1 — Perimeter (no VRC dependency).** Bearer token auth on mutating and
  data endpoints, CORS allowlist, per-IP rate limiting. Lands in parallel with VRC SCA.
- **Phase 2 — Integrity (depends on VRC stable `build_id`).** ed25519 manifest
  signing served as an HTTP header, SRI helper for HTML generators, CSP header.

The signing primitives **already exist** in `crates/cloudpack-abs/src/signing.rs`
(`ManifestSigner`, `ManifestVerifier`, `manifest_signature_bytes`). Phase 2 is wiring,
not invention. mTLS is explicitly deferred.

The recommended designs are **Phase 1 → C2** (static token + CORS + in-process rate limiter)
and **Phase 2 → C2** (signing + SRI helper + CSP report-only). Linux/Unix philosophy
governs every choice: ABS provides the *mechanism*; token issuance, key custody, origin
list, and CSP report destination are *policy* and live in `cloudpack.toml` or env.

---

## ANALYZE — System Map

### Actors and trust boundaries

```
                 untrusted                       │       trusted
                                                 │
   ┌─────────────────┐    HTTPS    ┌────────────┐│  loopback   ┌──────────┐
   │ Browser + SW    │◄───────────►│   ABS      ││◄───────────│ CI / deploy
   │ (cdn fetches)   │             │ HTTP server││            │ tooling    │
   └─────────────────┘             └────────────┘│            └──────────┘
                                         ▲       │                  │
                                         │ reads │                  │ writes
                                         │       │                  ▼
                                   ┌──────────┐  │            ┌──────────┐
                                   │ manifest │  │            │ archive  │
                                   │  on disk │  │            │ on disk  │
                                   └──────────┘  │            └──────────┘
                                                 │
   ┌─────────────────┐                  CDN      │       │
   │     CDN         │◄────────────────────────  pulls chunks by content hash
   │ (immutable CAS) │                           │       │
   └─────────────────┘                           │       │
```

Three trust boundaries:

1. **Browser ↔ ABS** — fully untrusted network. Browser hits `/manifest` over public HTTPS.
2. **CI/deploy ↔ ABS** — `POST /reload`, `POST /select`. Until external review, loopback-only
   (VRC interim). After Phase 1, may be exposed to localnet with bearer token.
3. **ABS ↔ CDN/disk** — assumed trusted. ABS reads `manifest.json` and the archive from
   local disk; CDN is content-addressed and immutable.

### Threat model

| # | Attacker                          | Capability              | Attack path                                       | Blast radius                                              |
|---|-----------------------------------|-------------------------|---------------------------------------------------|-----------------------------------------------------------|
| T1 | Compromised CI / deploy actor     | Push to manifest path   | Replace `manifest.json` on disk → ABS hot-reloads | **Full RCE in every browser.** Worst case in the system. |
| T2 | On-path network attacker (no TLS) | MITM HTTP responses     | Inject malicious chunk URLs into `/manifest` body | Full RCE in affected sessions until cache invalidated.    |
| T3 | Malicious internal client         | Network reachability    | Spam `POST /manifest` to exhaust CPU / FD         | DoS — server unavailable. No code execution.              |
| T4 | Same-origin cross-tenant          | XHR/fetch from a peer   | Read `/manifest` from foreign origin              | Information disclosure: chunk topology, internal paths.   |
| T5 | Operator with low-priv shell      | Local network access    | Call `POST /reload` or `/select` without creds   | Manifest swap → effectively T1 with smaller surface.      |
| T6 | Attacker who steals signing key   | Future key compromise   | Sign rogue manifest, push to clients              | Full RCE until key rotation + clients refetch.            |
| T7 | Attacker who steals bearer token  | Log file, env leak      | Authenticate as legit CI                          | T1.                                                       |

### What Phase 1 addresses

| Threat | Phase 1 effect |
|--------|----------------|
| T3     | Mitigated by per-IP rate limit on `/manifest`. |
| T4     | Mitigated by CORS allowlist (prevents browser XHR from foreign origins; does not stop server-side fetches). |
| T5     | Mitigated by bearer token on `/reload` and `/select` when they become non-loopback. |
| T7     | Surface reduced: token file 0600, never logged, env-var or file path only (never CLI arg). Still possible. |

### Residual risk after Phase 1 (what Phase 2 must address)

| # | Residual                                                  | Why Phase 1 cannot fix it                                                                 |
|---|-----------------------------------------------------------|-------------------------------------------------------------------------------------------|
| R1 | T1 unmitigated — disk-level write authority = RCE        | Bearer token does not verify *content*, only *caller identity*. Any caller with write authority to the manifest path bypasses everything. |
| R2 | T2 unmitigated for clients behind broken/MITM'd TLS      | HTTP-layer auth doesn't bind manifest content to a key the client can verify out-of-band. |
| R3 | Stolen browser-cached manifest plus rogue CDN = RCE      | No client-side cryptographic check. SW just trusts the manifest it has cached.            |

Phase 2 addresses **R1–R3** by signing the manifest with ed25519: the SW (and any consumer)
verifies a *signature*, not just a *transport channel*. The signing key lives outside ABS;
disk-write authority is no longer sufficient.

### What Phase 2 still cannot protect against (honest gap)

| Gap | Reason |
|-----|--------|
| **Compromised build pipeline.** If the signing key is on the build host and the build host is compromised, the attacker signs the malicious manifest themselves. Phase 2 raises the bar from "any write" to "compromise the build host or steal the key" — not to zero. |
| **Compromised CDN.** Signing the manifest binds chunk content hashes. But if a chunk URL the manifest points to is mutated *and* the chunk's hash matches (collision), or if the CDN serves a chunk with a different hash (which SRI catches), the SW would still execute. SRI is the second layer. |
| **Stolen-and-not-yet-rotated key.** ed25519 signatures cached in browsers (with the manifest) remain valid until the SW refetches and sees a new key/signature. There is no revocation channel short of pushing a new build. We document this; key rotation is operational, not architectural. |
| **DoS at the network or CDN layer.** Per-IP rate limit is in-process — it does not protect against a botnet or against L3/L4 floods. That belongs in the deployment perimeter (Cloudflare, AWS WAF, etc.). |
| **Token compromise without rotation.** Phase 1 has no automatic rotation. We provide a rotation *mechanism* (two-token grace) but the *policy* of rotation cadence is the operator's. |
| **Authenticated insider with valid token + key.** Out of scope. This is the auditor's domain. |

---

## DESIGN — Phase 1: Perimeter

### Phase 1 candidates

#### C1 — Static bearer token only

- One token from `cloudpack.toml [security] bearer_token_file = "..."` or env `CLOUDPACK_BEARER_TOKEN`.
- Tower middleware applied to every route except `GET /health`.
- No CORS changes. No rate limit. Anyone with the token gets full access.

#### C2 — Static bearer + CORS allowlist + in-process rate limiter  ← **recommended**

- Same token as C1.
- `tower_http::cors::CorsLayer` configured from `[security] allowed_origins = [...]`. **No wildcard.**
  Origins absent from the list get a 403, never a permissive default.
- Per-IP rate limit on `POST /manifest` using `governor` (`Quota::per_second(N)` with a small burst).
  In-process. Resets on restart. Per-replica.

#### C3 — Per-client token registry + CORS + rate limiter

- A `tokens.toml` file: `{token_id, sha256_of_token, label, created_at, revoked}`.
- Reload on `SIGHUP` or `/reload`.
- Per-token rate limit instead of (or alongside) per-IP.
- Operator can revoke a leaked token without rotating everyone.

### Phase 1 — Component design (concrete Rust)

New files in `crates/cloudpack-abs/src/`:

```
src/
├── lib.rs                  # adds: pub mod security;
├── server.rs               # modified: apply security layers to router
├── security/
│   ├── mod.rs              # SecurityConfig, SecurityLayers
│   ├── auth.rs             # bearer-token middleware
│   ├── cors.rs             # CORS layer builder
│   ├── ratelimit.rs        # per-IP rate limiter (governor)
│   └── config.rs           # SecurityConfig from cloudpack.toml
```

#### `security/config.rs`

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SecurityConfig {
    /// Path to a file containing the bearer token (one line, no whitespace).
    /// If both this and `bearer_token_env` are set, the file wins.
    pub bearer_token_file: Option<PathBuf>,
    /// Env var name to read the bearer token from. Default: CLOUDPACK_BEARER_TOKEN.
    pub bearer_token_env: Option<String>,
    /// CORS allowlist. Empty = deny all cross-origin. No wildcard supported.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Per-IP rate limit, requests per second. None disables the limiter.
    pub manifest_rate_per_sec: Option<u32>,
    /// Burst size for the rate limiter. Default: 2× rate.
    pub manifest_rate_burst: Option<u32>,
    /// Endpoints exempt from auth. Default: ["/health"].
    #[serde(default = "default_exempt")]
    pub exempt_paths: Vec<String>,
}

pub struct ResolvedSecurity {
    /// Token bytes. Constant-time compared per request. Never Display'd.
    pub(crate) token: SecretToken,
    pub(crate) allowed_origins: Vec<HeaderValue>,
    pub(crate) rate: Option<(NonZeroU32, NonZeroU32)>,
    pub(crate) exempt_paths: HashSet<String>,
}

/// Wrapper that prevents `Debug`/`Display` from leaking the token.
pub struct SecretToken(Box<[u8]>);
impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretToken(<redacted, {} bytes>)", self.0.len())
    }
}
impl SecretToken {
    pub fn ct_eq(&self, other: &[u8]) -> bool {
        // subtle::ConstantTimeEq
        use subtle::ConstantTimeEq;
        self.0.ct_eq(other).into()
    }
}
```

#### `security/auth.rs` — tower middleware (axum 0.8)

```rust
pub async fn require_bearer(
    State(sec): State<Arc<ResolvedSecurity>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, StatusCode> {
    // 1. Exempt paths bypass.
    if sec.exempt_paths.contains(req.uri().path()) {
        return Ok(next.run(req).await);
    }
    // 2. Authorization: Bearer <token>
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let raw = header.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !sec.token.ct_eq(token.as_bytes()) {
        // tracing::warn!("auth failure"); // NEVER log the token itself
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}
```

Applied as `axum::middleware::from_fn_with_state(security_arc, require_bearer)`.

**Why tower middleware over an axum extractor:** the same logic must guard every
non-exempt route. An extractor would require updating every handler signature.

#### `security/cors.rs`

```rust
pub fn build_cors(sec: &ResolvedSecurity) -> tower_http::cors::CorsLayer {
    use tower_http::cors::CorsLayer;
    use axum::http::Method;
    if sec.allowed_origins.is_empty() {
        // Deny all cross-origin. Browsers get no CORS headers → fetch blocked.
        return CorsLayer::new();
    }
    CorsLayer::new()
        .allow_origin(sec.allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(300))
}
```

#### `security/ratelimit.rs`

```rust
use governor::{Quota, RateLimiter, clock::DefaultClock, state::keyed::DefaultKeyedStateStore};
use std::net::IpAddr;

pub type IpRateLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

pub fn build_limiter(rate: NonZeroU32, burst: NonZeroU32) -> Arc<IpRateLimiter> {
    let quota = Quota::per_second(rate).allow_burst(burst);
    Arc::new(RateLimiter::keyed(quota))
}

pub async fn rate_limit_mw(
    State(lim): State<Arc<IpRateLimiter>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if lim.check_key(&addr.ip()).is_err() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(req).await)
}
```

Only applied to `POST /manifest` via `.route_layer(...)`. Per-route, not global, so
`/health` and `/sw.js` are not rate-limited.

New `[dependencies]` in `crates/cloudpack-abs/Cargo.toml`:

```toml
governor = "0.7"
subtle   = "2"
```

#### `server.rs` — applying the layers

```rust
pub fn build_router(app: AppState, telemetry: TelemetryLogger, sec: Arc<ResolvedSecurity>) -> Router {
    let state = RouterState { app, telemetry };
    let limiter = sec.rate.map(|(r, b)| build_limiter(r, b));

    let mut manifest_route = post(post_manifest);
    let manifest = if let Some(l) = limiter.clone() {
        Router::new()
            .route("/manifest", manifest_route)
            .layer(axum::middleware::from_fn_with_state(l, rate_limit_mw))
    } else {
        Router::new().route("/manifest", manifest_route)
    };

    Router::new()
        .merge(manifest)
        .route("/health", get(get_health))      // exempt from auth via exempt_paths
        .route("/sw.js", get(get_service_worker))
        .layer(build_cors(&sec))
        .layer(axum::middleware::from_fn_with_state(sec, require_bearer))
        .with_state(state)
}
```

Note the layer order: **CORS outer, auth inner**. Browsers must receive CORS headers
even on 401 responses, otherwise the browser drops the response and the developer sees
a confusing "CORS error" instead of "auth error."

### Phase 1 — 8-dimension tradeoff matrix

Dimensions: **Simplicity**, **Performance**, **Scalability**, **Reliability**, **Security**,
**Operability**, **Evolvability**, **Cost**.

| Dimension     | C1 — token only                 | C2 — token + CORS + RL  ← rec  | C3 — token registry              |
|---------------|---------------------------------|---------------------------------|----------------------------------|
| Simplicity    | ★★★★★ (one constant)           | ★★★★☆ (3 layers, clear order)  | ★★★☆☆ (registry + reload)        |
| Performance   | ★★★★★ (1 hash compare)         | ★★★★☆ (governor is lock-free)  | ★★★☆☆ (registry lookup per req)  |
| Scalability   | ★★★☆☆ (no DoS protection)      | ★★★★☆ (per-replica RL)         | ★★★★☆ (per-token quota possible) |
| Reliability   | ★★★★☆ (less state, less to break) | ★★★★☆ (in-process state)    | ★★★☆☆ (registry file = new SPOF) |
| Security      | ★★☆☆☆ (no CORS, no RL)         | ★★★★☆ (defense in depth)        | ★★★★★ (revocation, per-actor)    |
| Operability   | ★★★★★ (one secret)             | ★★★★☆ (3 config keys)           | ★★☆☆☆ (registry lifecycle)       |
| Evolvability  | ★★☆☆☆ (one-token ceiling)      | ★★★★☆ (registry can be added)  | ★★★★★ (oauth/etc. fit naturally) |
| Cost          | ★★★★★ (no new deps)            | ★★★★☆ (+governor, +subtle)      | ★★☆☆☆ (file format, tests, docs) |

---

## DESIGN — Phase 2: Integrity

Depends on VRC SCA delivering a stable, deterministic `build_id`.

### Phase 2 candidates

#### C1 — ed25519 manifest signing only

- ABS signs `manifest_signature_bytes(manifest)` at load time, caches the signature.
- Every `POST /manifest` response carries `X-Cloudpack-Signature: base64(sig)` and
  `X-Cloudpack-Build-Id: ...`.
- SW fetches `GET /manifest/full.json` once per build_id, verifies, caches.
- No SRI. No CSP.

#### C2 — Signing + SRI helper + CSP report-only  ← **recommended**

- C1, plus:
- A small `cloudpack-html` helper crate (or module inside `cloudpack-graph`) that, given a
  `ChunkManifest` and entry-point, emits `<script src="..." integrity="sha256-..." crossorigin>`
  tags. Lives outside ABS because ABS does not generate HTML; consumers do.
- ABS attaches a CSP **report-only** header on `/sw.js` and (optionally) any HTML it serves
  in dev mode, derived from chunk hashes:
  `Content-Security-Policy-Report-Only: script-src 'sha256-...' 'sha256-...' ...; report-uri /csp-report`
- New `POST /csp-report` endpoint that appends to the existing JSONL telemetry log,
  rate-limited and bearer-exempt (browsers send these unauthenticated by spec).

#### C3 — Signing + SRI + CSP enforce + signature pinned in SW

- C2, plus:
- CSP in **enforcing** mode, not report-only.
- Service Worker source has the **public verifying key embedded at build time**, so a
  rogue ABS cannot serve a manifest signed by a different key. Pinning eliminates the
  "trust ABS to tell me which key to use" loop.

### Phase 2 — Component design (concrete Rust)

What already exists (no work needed):

- `crates/cloudpack-abs/src/signing.rs`
  - `ManifestSigner::from_pem(pem) → ManifestSigner`
  - `ManifestSigner::sign_manifest(&ChunkManifest) → Signature`
  - `ManifestVerifier::from_pem` / `verify`
  - `manifest_signature_bytes(&ChunkManifest)` — canonical bytes, deliberately excludes
    advisory PGO fields. **This is the contract the SW must replicate.**
- `crates/cloudpack-abs/src/state.rs::AppState::load_signed_from_disk` — already accepts an
  optional verifier and signature.
- `crates/cloudpack-graph/src/types.rs:89` — `ChunkManifest.build_id` (filled in by VRC SCA).

What is missing (Phase 2 work):

```
src/
├── state.rs                # extend AppState with `signature: Arc<RwLock<Option<Signature>>>`
├── server.rs               # add: GET /manifest/full.json, X-Cloudpack-Signature header
└── security/
    └── csp.rs              # build CSP value from manifest chunk hashes
```

And, separately:

```
crates/cloudpack-html/        # new tiny crate (≤300 LOC)
├── Cargo.toml
└── src/
    ├── lib.rs              # pub fn render_script_tags(manifest, entry) -> String
    └── sri.rs              # sha256→base64 wrapping; verifies hash format from CAS
```

#### `state.rs` changes

```rust
pub struct AppState {
    pub manifest: Arc<RwLock<ChunkManifest>>,
    pub signature: Arc<RwLock<Option<Signature>>>,   // NEW
    pub cdn_base_url: Arc<String>,
    pub ttl_seconds: u64,
}
```

`AppState::load_signed_from_disk` already verifies; extend it to *store* the signature
so it can be served. On `/reload`, both the manifest and its signature update atomically
(`tokio::sync::RwLock` write).

#### `server.rs` — new endpoint

```rust
async fn get_manifest_full(State(state): State<RouterState>) -> impl IntoResponse {
    let manifest = state.app.manifest.read().await;
    let sig = state.app.signature.read().await;

    let body = serde_json::to_vec(&*manifest).expect("manifest serializes");
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    headers.insert("x-cloudpack-build-id", manifest.build_id.parse().unwrap());
    if let Some(s) = sig.as_ref() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(s.to_bytes());
        headers.insert("x-cloudpack-signature", b64.parse().unwrap());
    }
    headers.insert(
        header::CACHE_CONTROL,
        format!("public, max-age={}, immutable", state.app.ttl_seconds).parse().unwrap(),
    );
    (headers, body)
}
```

`/manifest/full.json` is **publicly readable** (auth-exempt). The signature, not the
transport, is the trust anchor. Cacheable at the CDN.

The Service Worker (`crates/cloudpack-abs/assets/sw.js`) gains:

```js
// Pseudocode
async function getTrustedManifest() {
  const r = await fetch('/manifest/full.json');
  const sig = r.headers.get('x-cloudpack-signature');
  const buildId = r.headers.get('x-cloudpack-build-id');
  const bytes = await r.arrayBuffer();
  const ok = await verifyEd25519(PINNED_PUBKEY, canonicalBytesFromJson(bytes), b64decode(sig));
  if (!ok) throw new Error('manifest signature invalid');
  return { manifest: JSON.parse(decoder.decode(bytes)), buildId };
}
```

The JS-side `canonicalBytesFromJson` MUST exactly mirror Rust's `manifest_signature_bytes`.
We will ship a contract test that signs a fixture in Rust and verifies it in
Node + browser via the SW's verifier. **This is the most fragile contract in the system
and gets its own test file.**

#### `cloudpack-html` crate

```rust
pub fn render_script_tags(manifest: &ChunkManifest, entry: &str) -> String {
    let mut out = String::new();
    for chunk_id in manifest.entry_chunks.get(entry).unwrap_or(&vec![]) {
        let chunk = manifest.chunks.iter().find(|c| &c.id == chunk_id).unwrap();
        // CAS hash is already sha256 hex. SRI wants sha256-base64.
        let b64 = sri::hex_to_sri_b64(&chunk.hash);
        writeln!(out, r#"<script src="{}" integrity="sha256-{}" crossorigin defer></script>"#,
            chunk.url, b64).unwrap();
    }
    out
}
```

CAS already produces content hashes — SRI is a pure encoding transform. No new
hashing is done.

#### `security/csp.rs`

```rust
pub fn build_csp(manifest: &ChunkManifest, report_only: bool) -> (HeaderName, HeaderValue) {
    let mut hashes: Vec<String> = manifest.chunks.iter()
        .map(|c| format!("'sha256-{}'", sri::hex_to_sri_b64(&c.hash)))
        .collect();
    hashes.sort();
    hashes.dedup();
    let value = format!("script-src {}; report-uri /csp-report", hashes.join(" "));
    let name = if report_only {
        HeaderName::from_static("content-security-policy-report-only")
    } else {
        HeaderName::from_static("content-security-policy")
    };
    (name, value.parse().unwrap())
}
```

### Phase 2 — 8-dimension tradeoff matrix

| Dimension     | C1 — sign only                | C2 — sign + SRI + CSP report  ← rec | C3 — sign + SRI + CSP enforce + pin |
|---------------|-------------------------------|--------------------------------------|--------------------------------------|
| Simplicity    | ★★★★★ (one mechanism)        | ★★★☆☆ (three mechanisms)             | ★★☆☆☆ (key pin in client)            |
| Performance   | ★★★★★ (one verify per build) | ★★★★★ (CSP/SRI free at runtime)      | ★★★★★                                |
| Scalability   | ★★★★★                        | ★★★★★                                | ★★★★★                                |
| Reliability   | ★★★★☆ (one verify path)      | ★★★☆☆ (CSP misconfig → silent break) | ★★☆☆☆ (key rotation requires deploy) |
| Security      | ★★★☆☆ (one layer)            | ★★★★☆ (three layers; CSP signals)    | ★★★★★ (pin closes "trust ABS" loop) |
| Operability   | ★★★★★ (one rotation)         | ★★★★☆ (CSP reports = signal)         | ★★☆☆☆ (pin = redeploy to rotate)     |
| Evolvability  | ★★★★☆                        | ★★★★☆ (CSP can flip to enforce later)| ★★☆☆☆ (pin couples binary + key)     |
| Cost          | ★★★★★ (signing already exists)| ★★★★☆ (+cloudpack-html, +csp.rs)       | ★★★☆☆ (+SW build pipeline)           |

---

## RECOMMEND

### Phase 1 → C2

**Static bearer token + CORS allowlist + in-process per-IP rate limiter.**

- **Optimizes for:** defense in depth at minimal complexity. Three orthogonal mechanisms
  with distinct failure modes (auth bypass ≠ CORS bypass ≠ DoS). One config block.
- **Sacrifices:** no per-actor revocation (C3), no global rate budget across replicas.
  Both deferred to "operationally needed?" — at POC stage neither is.
- **Linux/Unix posture:** ABS provides the *mechanism* (verify a token, enforce an origin,
  count per IP). The *policy* — which tokens exist, when to rotate, which origins are
  legitimate, what rate to allow — lives entirely in `cloudpack.toml` and in operator
  process. Token issuance and storage are explicitly not ABS's job.

### Phase 2 → C2

**ed25519 signing + SRI helper + CSP in report-only mode.**

- **Optimizes for:** evidence-gated rollout. CSP report-only generates a flight of real-world
  CSP violation reports before we flip enforce, eliminating the "looked safe in dev, broke
  half the customers in prod" failure pattern. SRI catches CDN drift independently of the
  signature path.
- **Sacrifices:** until enforce mode flips, CSP is observational, not protective. SW key
  pinning (C3) is deferred — it's the right end state, but requires a stable SW build
  pipeline that POC doesn't have yet. We capture the migration in a follow-up.
- **Linux/Unix posture:** the signing primitive (already exists), the SRI encoder, and the
  CSP builder are three independent small tools. Each does one thing. Each can be
  consumed without the others. HTML emission lives in `cloudpack-html` because ABS does not
  emit HTML — the *consumer* chooses the policy of which entry points get SRI tags.

---

## RISKS — ranked by severity

| Rank | Risk | Severity | Phase | Mitigation in this design | Residual |
|------|------|----------|-------|----------------------------|----------|
| 1 | **Bearer token leaked via logs.** `tracing` emits request headers in debug builds. CI logs are world-readable in many setups. | High | 1 | `SecretToken` wrapper redacts `Debug`/`Display`. Explicit allow-list of headers in `tower_http::trace::TraceLayer`. Unit test: token never appears in `tracing` capture. | Operator can still cat the token file. Acceptable. |
| 2 | **CSP enforce mode breaks the app silently.** A single missed chunk hash and the page won't load JS — and the browser console message may not reach the operator. | High | 2 | Ship in **report-only first**. Add `/csp-report` endpoint; require ≥1 week of zero non-test reports before flipping to enforce. Document the procedure. | None — this is risk control, not elimination. |
| 3 | **ed25519 key rotation without client re-deployment.** SW pins or browser-cached signatures from old key remain "valid" to a cached SW. | High | 2 | C2 does *not* pin in SW — verifying key is fetched alongside the manifest. Rotation = swap key on disk + `POST /reload` → next build gets new signature, all clients refetch within `ttl_seconds`. Pinning is explicitly deferred to a later design. | If C3 is adopted later, rotation becomes a redeploy. Accepted tradeoff documented. |
| 4 | **Rate limiter in-process vs per-replica.** A 2-replica deployment with `manifest_rate_per_sec = 100` allows 200 rps under round-robin. | Medium | 1 | Documented. Configure rate **per replica**, not total. When HA lands, revisit with Redis-backed governor or perimeter rate limiting (Cloudflare/WAF). | None — for POC, in-process is sufficient. |
| 5 | **SW caching a valid manifest, then signing key compromised.** Cached manifest remains verifiable until the SW refetches. | Medium | 2 | `Cache-Control: max-age={ttl_seconds}` on `/manifest/full.json`. Operators set `ttl_seconds` according to their tolerance. After key rotation, push a new build; old key signatures become stale at next refetch. Document a "panic" procedure: unregister SW + force reload. | Window of vulnerability = `ttl_seconds`. Operator-tunable. |
| 6 | **SRI breaking on mutable chunk URLs.** SRI compares the *fetched* hash to the *declared* hash. If a chunk's URL maps to mutable bytes, every page load fails. | Medium | 2 | Cloudpack's CAS guarantees: a chunk URL contains the hash; the bytes at that URL are immutable. We add a startup invariant check in `cloudpack-html` that hash extracted from URL == hash in manifest. Refuse to start otherwise. | Only breaks if CAS contract is broken elsewhere. |
| 7 | **CORS misconfig allows credentialed cross-origin.** `Access-Control-Allow-Credentials: true` combined with a too-broad origin = session theft. | Medium | 1 | We **never** set `allow_credentials(true)` in C2's CORS layer. Bearer auth is server-to-server; no browser uses it. Wildcard origin is unrepresentable in config (no `*` accepted). | None. |
| 8 | **Two-replica auth race during token rotation.** Replica A loaded new token, replica B still has old; a single client request hits whichever, half fail. | Low | 1 | Support **two-token grace**: `bearer_token_file` may contain *one or two* tokens, newline-separated. Auth accepts either. Rotation procedure: add new, deploy, remove old. | Operator process, documented. |
| 9 | **Constant-time compare bypass via length oracle.** Naïve `==` on token bytes leaks length. | Low | 1 | `subtle::ConstantTimeEq` in `SecretToken::ct_eq`. Length is intentionally compared in constant time too (`subtle` handles this). | None. |
| 10 | **CSP report endpoint DoS.** Browsers send CSP reports unauthenticated and at high volume on a misconfigured site. | Low | 2 | `/csp-report` is rate-limited per IP with stricter quota than `/manifest`. Body size capped at 8 KiB. Logged to the existing JSONL telemetry file (no new storage path). | None. |

---

## SEQUENCING with Versioned Runtime Control

```
                ┌──────────────────────────────────┐
                │  VRC SCA (already designed)      │
                │  • compute_build_id()            │
                │  • POST /reload                  │
                │  • canonical_bytes()             │
                └──────────────┬───────────────────┘
                               │ produces stable build_id
                               │
       ┌───────────────────────┼───────────────────────────┐
       │ NO DEPENDENCY         │   DEPENDS ON SCA          │
       │                       │                           │
       ▼                       │                           ▼
  Phase 1: Perimeter           │                  Phase 2: Integrity
   • bearer token              │                   • manifest signing
   • CORS allowlist            │                   • SRI helper
   • per-IP rate limit         │                   • CSP report-only
       │                       │                           │
       │ Can land in parallel  │ Must wait for SCA          │
       │ with VRC SCA          │ (needs deterministic       │
       │                       │  build_id in manifest)     │
       ▼                       │                           ▼
   ── SHIP ──                  │                       ── SHIP ──
                               │
                               │ enables, but no hard dep
                               ▼
                       VRC archive + /versions
```

### What can ship before VRC SCA

- **Phase 1.1 — bearer token middleware.** Pure HTTP-layer concern. Zero touch to manifest.
- **Phase 1.2 — CORS allowlist.** Pure HTTP-layer concern.
- **Phase 1.3 — per-IP rate limiter.** Pure HTTP-layer concern.

All three are additive: a missing `[security]` section means the previous unauthenticated
behavior. We will *not* default-enable any of them; the change to defaults happens in a
separate, documented release.

### What requires VRC SCA first

- **Phase 2.1 — manifest signing.** The signing canonical-bytes function already commits
  to a stable `build_id` (it's the first line). VRC SCA must guarantee that `build_id`
  is **deterministic from the input set**, not a UUID. Without that, a re-build of the
  same source produces a different signature and clients see false-positive integrity
  events.
- **Phase 2.2 — SRI / CSP.** Both derive from chunk hashes. CAS already produces those,
  so technically they don't *strictly* require SCA. But: shipping SRI before signing
  means we have integrity at the chunk level but not the manifest level. Attackers
  pivot to the manifest. Order matters: sign first, then SRI as defense in depth.
  Therefore Phase 2.1 → 2.2 → CSP, all post-SCA.

### Concurrent build order

```
Week of:  [ SCA ]  [ P1.1 ]  [ P1.2 ]  [ P1.3 ]  [ P2.1 ]  [ P2.2 ]  [ CSP ]
   1       █████
   2       █████   ████████
   3               ████████  ████████
   4                         ████████  ████████
   5                                             ████████
   6                                                       ████████
   7                                                                 ████████
```

Phase 1 runs in parallel with SCA on a separate engineer. Phase 2 starts the week SCA
lands. No serialization across phases beyond the SCA gate.

---

## SIMPLEST CREDIBLE ALTERNATIVE

If "do the minimum that materially reduces risk without requiring a security audit":

> **Phase 1.1 alone** — Static bearer token via `CLOUDPACK_BEARER_TOKEN` env var,
> middleware on every route except `/health`, constant-time compare, no logging.
>
> One file: `crates/cloudpack-abs/src/security/auth.rs`. ~80 LOC. One new dep: `subtle`.
> One config knob: the env var name. Zero changes to manifest, signing, SW, or HTML.

Why this is *credible* and not just *cheap*:

- It closes T5 (low-priv local network access calling `/select`) and the most common
  T2 footgun (publishing ABS to the open internet by accident).
- It does not introduce CSP / SRI / signing — none of which need *audit* per se, but
  all of which need *operator literacy*. Bearer auth needs only "keep this string secret."
- It has a well-understood failure mode (token leak) with a well-understood mitigation
  (rotate the env var, restart). No cryptographic state, no key custody, no SW changes.

What it explicitly does *not* do, and why that's honest:

- It does **not** address T1 (disk-write authority = RCE) at all. That requires Phase 2
  signing. We document that the simplest alternative is a transport-layer perimeter, not
  a content-integrity guarantee.
- It does **not** address T3 (DoS). A bored attacker still wins.
- It does **not** address T4 (cross-origin info disclosure). Same-network browsers can
  still XHR the manifest.

Use this option **only** if the team's bandwidth is below what C2 requires and we
explicitly accept T1/T3/T4 until Phase 2 lands.

---

## What this design deliberately does not do

| Decision | Rationale |
|----------|-----------|
| **No JWT / OAuth / OIDC.** | "No external auth infrastructure" is a stated constraint. Static bearer is sufficient for a server-to-server perimeter at POC scale. |
| **No mTLS.** | Topology unknown — there is no internal service network for ABS to talk to. Defer until a real internal data plane exists. |
| **No automatic key rotation.** | Cryptographic state lives outside ABS; rotation is an operator workflow. Two-token grace is provided as a *mechanism*; rotation cadence is *policy*. |
| **No SW-side public key pinning (yet).** | Pinning is the right end state but couples binary builds to key custody. We document the migration path in Phase 2 → C3 and revisit when the SW has a real build pipeline. |
| **No CSP enforce on day one.** | Report-only first; flip to enforce on evidence. This single decision prevents the most common CSP-rollout failure mode. |
| **No logging of bearer tokens, ever.** | Hard constraint. Enforced by `SecretToken` wrapper + a unit test that scans `tracing` capture. |
| **`/select` and `/reload` stay loopback-bound until external review.** | VRC's interim posture. Bearer auth lets them go off-loopback *after* review, not unilaterally. |

---

## Open questions (resolve before implementation kicks off)

1. **CSP report destination — `/csp-report` on ABS, or external?** Default to ABS for POC
   simplicity; flag if any operator wants to ship to a SIEM.
2. **Per-IP keying when behind a reverse proxy.** `X-Forwarded-For` parsing must be opt-in
   and source-restricted. Otherwise a hostile client spoofs IPs and bypasses the limiter.
   `governor` keyed by `ConnectInfo` is correct only if ABS is the edge.
3. **Two-token grace file format.** Newline-separated plaintext or a TOML list? Plaintext
   is simpler and aligns with the "policy out of the binary" stance. Confirm.
4. **Does the bench harness need to bypass auth?** The bench currently hits ABS directly.
   We expose `[security.bench_bypass_token = "..."]` that auto-generates on `cloudpack bench`
   for the duration of the run; never persists to telemetry; not in default config.

---

## Acceptance criteria

### Phase 1
- [ ] Default `cloudpack.toml` with no `[security]` section preserves existing behavior (no migration forced).
- [ ] With `[security] bearer_token_file = "..."`, every non-exempt route returns 401 without `Authorization: Bearer <token>`.
- [ ] `tracing` capture from a failing-auth request never contains the token bytes.
- [ ] `subtle::ConstantTimeEq` is the only comparison path; lint rule forbids `==` on `SecretToken`.
- [ ] CORS test: cross-origin request from non-allowlisted origin gets 403 with no `Access-Control-Allow-Origin` header.
- [ ] Rate limit test: 100 requests in <1s from one IP at `rate=10` produces ≥90 `429`s.
- [ ] Two-token grace: token file with two lines, requests authenticated with either succeed; with neither, 401.

### Phase 2
- [ ] `AppState` exposes `signature: Option<Signature>`; `/manifest/full.json` returns it as `X-Cloudpack-Signature` header (base64).
- [ ] SW fixture test: Rust signs a fixture manifest; Node-side verifier (mirroring `manifest_signature_bytes` byte-for-byte) verifies.
- [ ] `cloudpack-html::render_script_tags` emits `integrity="sha256-..."` matching the chunk hash, base64-encoded from CAS hex.
- [ ] CSP report-only: violating a derived CSP produces a `/csp-report` log line; no enforce header is ever set in this milestone.
- [ ] Key rotation drill: swap signing key on disk, `POST /reload`, new signature in next response, all clients refresh within `ttl_seconds`.
