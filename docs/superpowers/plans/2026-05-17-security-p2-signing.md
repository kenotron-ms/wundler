# Security P2 — ed25519 Manifest Signing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface ed25519 manifest signatures to clients via a new `GET /manifest/full.json` endpoint, ship an SRI-aware HTML rendering crate, and add a tamper-evidence `POST /csp-report` sink — finishing the server-side half of the Phase 2 supply-chain story.

**Architecture:** The `ManifestSigner` / `ManifestVerifier` machinery already exists in `wundler-abs::signing`. This plan threads a fresh `Option<Signature>` slot through `AppState` so the most-recently-loaded signature can be served alongside the manifest, wires a `ManifestSigner` into the `POST /reload` path so operator-driven swaps re-sign, and exposes the signature as an `X-Wundler-Signature` header on a new public read endpoint. A new `wundler-html` crate produces `<script integrity="sha256-…">` tags from a manifest, and a `security/csp.rs` module supplies report-only CSP headers and a small `/csp-report` ingester. A cross-language contract test pins down the deterministic byte format of `manifest_signature_bytes` so a future JavaScript verifier can re-derive the same bytes.

**Tech Stack:** Rust 2021, axum 0.8, tokio 1, ed25519-dalek 2, base64 0.22, hex 0.4, anyhow 1, serde 1, tower 0.5 (tests), axum-test 20 (tests), tempfile 3.

**Scope Boundary:**
- **In scope:** `AppState.signature` field + `snapshot_signature()`, `GET /manifest/full.json` route, `POST /csp-report` route, `wundler-abs::security::csp`, new `wundler-html` crate (`render_script_tags`, `hex_to_sri_b64`), Rust half of the cross-language signing contract test, wiring the signing key into `POST /reload`.
- **Out of scope:** Service-Worker ed25519 verification, SW public-key pinning, CSP enforce mode (we only emit `Content-Security-Policy-Report-Only`), JWT / OAuth / mTLS, automatic key rotation, the JavaScript half of the contract test, and the pre-existing `session_id`-carries-`build_id` bug — do **not** touch that.

---

## File Structure

**Created:**
- `crates/wundler-html/Cargo.toml`
- `crates/wundler-html/src/lib.rs` — `render_script_tags(manifest, entry, cdn_base_url) -> String`
- `crates/wundler-html/src/sri.rs` — `hex_to_sri_b64(hex) -> String`
- `crates/wundler-abs/src/security/csp.rs` — `build_csp_report_only`, `hex_to_sri_b64` (local copy for ABS)
- `crates/wundler-abs/tests/signing_contract_test.rs` — Rust half of cross-language contract test

**Modified:**
- `Cargo.toml` (workspace) — add `crates/wundler-html` to `members`
- `crates/wundler-abs/Cargo.toml` — add `base64 = "0.22"`, `hex = "0.4"`, `wundler-html` path dep
- `crates/wundler-abs/src/state.rs` — add `signature` field, `snapshot_signature`, `set_signature`; reset on `swap_to`
- `crates/wundler-abs/src/server.rs` — add `signer` to `RouterState`, change `build_router` signature, add `GET /manifest/full.json` and `POST /csp-report` handlers, wire signing into `POST /reload`
- `crates/wundler-abs/src/security/auth.rs` — extend `EXEMPT_PATHS`
- `crates/wundler-abs/src/security/mod.rs` — declare `pub mod csp`

---

## Task 1: Add `signature` to `AppState`

**Files:**
- Modify: `crates/wundler-abs/src/state.rs`
- Modify: `crates/wundler-abs/src/server.rs` (test helper only — `test_state()` in `mod tests`)

### Step 1.1 — Write the failing unit test for `snapshot_signature`

- [ ] Add this test inside `crates/wundler-abs/src/state.rs` (append a `#[cfg(test)] mod tests` block at the end if one doesn't exist):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};
    use rand::rngs::OsRng;
    use std::collections::HashMap;
    use wundler_graph::ChunkManifest;

    fn empty_manifest(build_id: &str) -> ChunkManifest {
        ChunkManifest {
            build_id: build_id.to_string(),
            chunks: vec![],
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        }
    }

    fn fresh_state(build_id: &str) -> AppState {
        let tmp = Box::leak(Box::new(tempfile::TempDir::new().expect("archive tempdir")));
        let archive = ManifestArchive::open(tmp.path(), 10).expect("open archive");
        let manifest = empty_manifest(build_id);
        archive.install(&manifest).expect("seed");
        archive.set_current(build_id).expect("current");
        AppState {
            manifest: Arc::new(RwLock::new(Arc::new(manifest))),
            archive: Arc::new(archive),
            reload_lock: Arc::new(Mutex::new(())),
            signature: Arc::new(RwLock::new(None)),
            cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
            ttl_seconds: 60,
        }
    }

    #[tokio::test]
    async fn snapshot_signature_is_none_by_default() {
        let state = fresh_state("b0");
        assert!(state.snapshot_signature().await.is_none());
    }

    #[tokio::test]
    async fn set_signature_stores_value_observable_by_snapshot() {
        let state = fresh_state("b0");
        let sk = SigningKey::generate(&mut OsRng);
        let sig = sk.sign(b"hello");
        state.set_signature(Some(sig)).await;
        let snap = state.snapshot_signature().await.expect("signature stored");
        assert_eq!(snap.to_bytes(), sig.to_bytes());
    }

    #[tokio::test]
    async fn swap_to_resets_signature_to_none() {
        let state = fresh_state("b0");
        // Seed a second build into the archive so we can swap to it.
        let b1 = empty_manifest("b1");
        state.archive.install(&b1).expect("install b1");

        let sk = SigningKey::generate(&mut OsRng);
        let sig = sk.sign(b"old-build-bytes");
        state.set_signature(Some(sig)).await;
        assert!(state.snapshot_signature().await.is_some());

        state.swap_to("b1").await.expect("swap");
        assert!(
            state.snapshot_signature().await.is_none(),
            "swap_to must clear the previous signature"
        );
    }
}
```

- [ ] **Step 1.2 — Run the test and confirm it fails to compile**

```bash
cargo test -p wundler-abs --lib state::tests
```

Expected: compile error — `signature`, `snapshot_signature`, `set_signature` do not exist.

### Step 1.3 — Add the field and methods to `AppState`

- [ ] Edit `crates/wundler-abs/src/state.rs`. Replace the existing `AppState` struct and impl block. Final shape:

```rust
//! Application state for the Asset Bundling Server.
//!
//! [`AppState`] wraps the current [`ChunkManifest`] in
//! `Arc<RwLock<Arc<ChunkManifest>>>` so that:
//!
//! * **Readers** clone the inner `Arc` under a short read guard — O(1) and never blocks.
//! * **Writers** swap the inner `Arc` atomically under a write guard.
//! * A separate `reload_lock` (`tokio::sync::Mutex`) serializes concurrent swaps.
//!
//! In addition to the manifest itself, `AppState` carries the **detached
//! ed25519 signature** of the currently-loaded manifest (or `None`). The
//! signature is set by:
//!
//! * `load_signed_from_disk` when a `(verifier, sig)` pair is provided at startup;
//! * `POST /reload` after re-signing with the configured signing key.
//!
//! Signatures are not persisted across swaps: `swap_to` resets the slot to
//! `None`, and the operator is expected to re-sign via `POST /reload`.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ed25519_dalek::Signature;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use wundler_graph::ChunkManifest;

use crate::archive::ManifestArchive;
use crate::signing::ManifestVerifier;

/// Result of a successful swap.
#[derive(Debug, Clone, Serialize)]
pub struct SwapReport {
    pub previous: String,
    pub current: String,
}

/// Shared application state carried by every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub manifest: Arc<RwLock<Arc<ChunkManifest>>>,
    pub archive: Arc<ManifestArchive>,
    pub reload_lock: Arc<Mutex<()>>,
    /// Detached signature for the **currently-loaded** manifest, or `None`
    /// when no verified signature is available (fresh start without a
    /// signing key, or post-swap before a re-sign).
    pub signature: Arc<RwLock<Option<Signature>>>,
    pub cdn_base_url: Arc<String>,
    pub ttl_seconds: u64,
}

impl AppState {
    /// Load a manifest from disk, seed the archive, and return `AppState`.
    ///
    /// When `verify` is `Some`, the signature is verified before the manifest
    /// is accepted, **and** the signature is stored in the `signature` slot so
    /// the server can re-serve it to clients.
    pub async fn load_signed_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
        archive: ManifestArchive,
        verify: Option<(&ManifestVerifier, &Signature)>,
    ) -> Result<Self> {
        let bytes = tokio::fs::read(manifest_path)
            .await
            .with_context(|| format!("failed to read manifest from {}", manifest_path.display()))?;

        let manifest: ChunkManifest = serde_json::from_slice(&bytes).with_context(|| {
            format!("failed to parse manifest JSON from {}", manifest_path.display())
        })?;

        let initial_sig = match verify {
            Some((verifier, sig)) => {
                verifier
                    .verify(&manifest, sig)
                    .context("manifest signature verification failed")?;
                Some(*sig)
            }
            None => None,
        };

        let build_id = manifest.build_id.clone();
        archive.install(&manifest).context("seed archive")?;
        archive
            .set_current(&build_id)
            .context("point archive `current` at seed manifest")?;

        Ok(Self {
            manifest: Arc::new(RwLock::new(Arc::new(manifest))),
            archive: Arc::new(archive),
            reload_lock: Arc::new(Mutex::new(())),
            signature: Arc::new(RwLock::new(initial_sig)),
            cdn_base_url: Arc::new(cdn_base_url),
            ttl_seconds,
        })
    }

    /// Convenience wrapper that skips signature verification.
    pub async fn load_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
        archive: ManifestArchive,
    ) -> Result<Self> {
        Self::load_signed_from_disk(manifest_path, cdn_base_url, ttl_seconds, archive, None).await
    }

    /// Return an `Arc<ChunkManifest>` snapshot — O(1), never blocks writers.
    pub async fn snapshot(&self) -> Arc<ChunkManifest> {
        let guard = self.manifest.read().await;
        Arc::clone(&*guard)
    }

    /// Return a cloned copy of the current signature, if any.
    pub async fn snapshot_signature(&self) -> Option<Signature> {
        let guard = self.signature.read().await;
        *guard
    }

    /// Replace the stored signature (or clear it with `None`).
    pub async fn set_signature(&self, sig: Option<Signature>) {
        let mut guard = self.signature.write().await;
        *guard = sig;
    }

    /// Swap the active manifest to the archive entry for `build_id`.
    ///
    /// Resets the stored signature to `None`: signatures don't carry across
    /// builds. Callers that want the new manifest to be signed must invoke
    /// `set_signature` after a successful swap (this is exactly what
    /// `POST /reload` does when `signing_key_pem` is configured).
    pub async fn swap_to(&self, build_id: &str) -> Result<SwapReport> {
        let _lock = self.reload_lock.lock().await;

        let new_manifest = self
            .archive
            .load(build_id)
            .with_context(|| format!("swap_to: load build_id={build_id}"))?;

        if new_manifest.build_id != build_id {
            return Err(anyhow::anyhow!(
                "swap_to: archive file for {build_id} has mismatched build_id={}",
                new_manifest.build_id
            ));
        }

        self.archive
            .set_current(build_id)
            .with_context(|| format!("swap_to: set_current {build_id}"))?;

        let new_arc = Arc::new(new_manifest);
        let mut guard = self.manifest.write().await;
        let previous = guard.build_id.clone();
        *guard = new_arc;
        drop(guard);

        // Reset the signature: the previous one belongs to the previous build.
        let mut sig_guard = self.signature.write().await;
        *sig_guard = None;
        drop(sig_guard);

        Ok(SwapReport { previous, current: build_id.to_string() })
    }

    /// Re-read `archive.current` and swap to it.
    pub async fn reload_from_current(&self) -> Result<SwapReport> {
        let current = self
            .archive
            .current()
            .context("read archive `current` symlink")?
            .ok_or_else(|| anyhow::anyhow!("archive has no `current` symlink"))?;
        self.swap_to(&current).await
    }
}
```

- [ ] **Step 1.4 — Update the `test_state()` helper in `server.rs`**

In `crates/wundler-abs/src/server.rs`, find the test helper around line 611 and add the `signature` field:

```rust
        let app = AppState {
            manifest: Arc::new(RwLock::new(Arc::new(manifest))),
            archive: Arc::new(archive),
            reload_lock: Arc::new(tokio::sync::Mutex::new(())),
            signature: Arc::new(RwLock::new(None)),
            cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
            ttl_seconds: 60,
        };
```

- [ ] **Step 1.5 — Run the new tests and the existing suite**

```bash
cargo test -p wundler-abs --lib
```

Expected: all tests pass, including the three new `state::tests` cases.

- [ ] **Step 1.6 — Run clippy on wundler-abs (zero new warnings policy)**

```bash
cargo clippy -p wundler-abs --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 1.7 — Commit**

```bash
git add crates/wundler-abs/src/state.rs crates/wundler-abs/src/server.rs
git commit -m "feat(abs): add Signature slot to AppState

- AppState.signature: Arc<RwLock<Option<Signature>>> (None by default)
- snapshot_signature() / set_signature() async helpers
- swap_to() resets signature to None — signatures don't carry between builds
- load_signed_from_disk() now stores the initial signature when verify is Some

No behavior change for existing endpoints; the field is unused until P2 tasks 3 & 5."
```

---

## Task 2: Create the `wundler-html` crate

**Files:**
- Create: `crates/wundler-html/Cargo.toml`
- Create: `crates/wundler-html/src/lib.rs`
- Create: `crates/wundler-html/src/sri.rs`
- Modify: `Cargo.toml` (workspace) — add `crates/wundler-html` to `members`

### Step 2.1 — Add the crate to the workspace

- [ ] Edit `Cargo.toml` (workspace root). Replace the `members` block with:

```toml
[workspace]
resolver = "2"
members = [
    "crates/wundler-core",
    "crates/wundler-cli",
    "crates/wundler-graph",
    "crates/wundler-transform",
    "crates/wundler-pipeline",
    "crates/wundler-abs",
    "crates/wundler-pgo",
    "crates/wundler-bench",
    "crates/wundler-html",
]
```

### Step 2.2 — Create `crates/wundler-html/Cargo.toml`

- [ ] Write:

```toml
[package]
name = "wundler-html"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
wundler-graph = { path = "../wundler-graph" }
hex = "0.4"
base64 = "0.22"

[dev-dependencies]
wundler-core = { path = "../wundler-core" }
```

### Step 2.3 — Write the failing test for `hex_to_sri_b64`

- [ ] Create `crates/wundler-html/src/sri.rs`:

```rust
//! Convert hex-encoded SHA-256 digests (the CAS format) into the base64
//! form required by Subresource Integrity (`integrity="sha256-…"`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Convert a 64-character lowercase hex SHA-256 digest into a 44-character
/// base64 SRI payload (no `sha256-` prefix — callers prepend it).
///
/// # Panics
///
/// Panics if `hex` is not valid hexadecimal. CAS hashes are produced by
/// `wundler-core::ContentHash::from_bytes`, which always emits valid hex, so
/// this is a programmer-error guard, not a runtime input validator.
pub fn hex_to_sri_b64(hex: &str) -> String {
    let bytes = hex::decode(hex).expect("CAS hash must be valid hex");
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer test:
    ///   sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    ///   base64 of those raw bytes = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
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
```

- [ ] Create `crates/wundler-html/src/lib.rs` with **just** the module declaration so the test compiles:

```rust
//! HTML rendering helpers for Wundler-bundled apps.
//!
//! Today this crate exposes a single function — [`render_script_tags`] — which
//! emits `<script src="…" integrity="sha256-…" crossorigin defer>` tags for
//! every chunk in a named entry point, plus the SRI helpers in [`sri`].

pub mod sri;

pub use sri::hex_to_sri_b64;
```

### Step 2.4 — Run the SRI tests; confirm they pass

```bash
cargo test -p wundler-html
```

Expected: 3 tests pass.

### Step 2.5 — Write the failing test for `render_script_tags`

- [ ] Append to `crates/wundler-html/src/lib.rs` (at the bottom, before any future code):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wundler_core::types::ContentHash;
    use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

    fn manifest_with_two_chunks() -> ChunkManifest {
        let h0 = ContentHash("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string());
        let h1 = ContentHash("a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447".to_string());
        let chunks = vec![
            Chunk {
                id: "chunk-a".to_string(),
                modules: vec![],
                hash: h0,
                load_condition: LoadCondition::Initial,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            },
            Chunk {
                id: "chunk-b".to_string(),
                modules: vec![],
                hash: h1,
                load_condition: LoadCondition::Initial,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            },
        ];
        let mut entry_chunks = HashMap::new();
        entry_chunks.insert("main".to_string(), vec!["chunk-a".to_string(), "chunk-b".to_string()]);
        ChunkManifest {
            build_id: "b0".to_string(),
            chunks,
            entry_chunks,
            module_index: HashMap::new(),
        }
    }

    #[test]
    fn render_script_tags_emits_one_tag_per_chunk_in_entry_order() {
        let m = manifest_with_two_chunks();
        let html = render_script_tags(&m, "main", "https://cdn.example.com");

        // Two <script ...> tags, no more, no less.
        assert_eq!(html.matches("<script").count(), 2);

        // Order matches entry_chunks order.
        let a_pos = html.find("chunk-a").expect("chunk-a in output");
        let b_pos = html.find("chunk-b").expect("chunk-b in output");
        assert!(a_pos < b_pos, "chunks must appear in entry order");

        // First tag contains the SRI for the first chunk.
        let expected_sri_a = "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
        assert!(
            html.contains(&format!("integrity=\"{expected_sri_a}\"")),
            "missing SRI hash for chunk-a in:\n{html}"
        );

        // Tag shape: src + integrity + crossorigin + defer.
        assert!(html.contains("crossorigin"));
        assert!(html.contains("defer"));

        // CDN URL is built from cdn_base_url + chunk-id-derived path.
        assert!(html.contains("https://cdn.example.com/chunks/"));
    }

    #[test]
    fn render_script_tags_unknown_entry_returns_empty_string() {
        let m = manifest_with_two_chunks();
        let html = render_script_tags(&m, "does-not-exist", "https://cdn.example.com");
        assert_eq!(html, "");
    }

    #[test]
    #[should_panic(expected = "manifest references unknown chunk id")]
    fn render_script_tags_panics_when_entry_lists_unknown_chunk() {
        // Startup invariant: the manifest is corrupt if entry_chunks names a
        // chunk that isn't in `chunks`. We panic loudly rather than emit a
        // tag with an undefined integrity hash.
        let mut m = manifest_with_two_chunks();
        m.entry_chunks
            .get_mut("main")
            .unwrap()
            .push("ghost-chunk".to_string());
        let _ = render_script_tags(&m, "main", "https://cdn.example.com");
    }
}
```

### Step 2.6 — Run the failing test

```bash
cargo test -p wundler-html
```

Expected: compile error — `render_script_tags` is undefined.

### Step 2.7 — Implement `render_script_tags`

- [ ] Replace `crates/wundler-html/src/lib.rs` with:

```rust
//! HTML rendering helpers for Wundler-bundled apps.
//!
//! Today this crate exposes a single function — [`render_script_tags`] — which
//! emits `<script src="…" integrity="sha256-…" crossorigin defer>` tags for
//! every chunk in a named entry point, plus the SRI helpers in [`sri`].

use std::collections::HashMap;

use wundler_graph::types::{Chunk, ChunkManifest};

pub mod sri;

pub use sri::hex_to_sri_b64;

/// Render the `<script>` tag block for a named entry point.
///
/// One tag is emitted per chunk in the entry's chunk list, in order. Each
/// tag has:
///
/// * `src="{cdn_base_url}/chunks/{hash8}.js"` where `hash8` is the first
///   eight hex chars of the chunk's content hash (matching the existing SW
///   convention in `assets/sw.js`).
/// * `integrity="sha256-{base64}"` derived from the full CAS hash.
/// * `crossorigin` (required by browsers when integrity is set on a
///   cross-origin URL).
/// * `defer` so scripts run in document order after parsing completes.
///
/// Tags are separated by `\n`. If `entry` is not in `manifest.entry_chunks`,
/// returns an empty string.
///
/// # Panics
///
/// Panics if a chunk id listed in `entry_chunks` is not present in
/// `manifest.chunks`. This is a **startup invariant**: a manifest that
/// references a missing chunk is corrupt and the binary should refuse to
/// emit a `<script integrity>` tag whose hash we cannot compute.
pub fn render_script_tags(
    manifest: &ChunkManifest,
    entry: &str,
    cdn_base_url: &str,
) -> String {
    let Some(chunk_ids) = manifest.entry_chunks.get(entry) else {
        return String::new();
    };

    let chunks_by_id: HashMap<&str, &Chunk> = manifest
        .chunks
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    let cdn = cdn_base_url.trim_end_matches('/');

    let mut out = String::new();
    for id in chunk_ids {
        let chunk = chunks_by_id
            .get(id.as_str())
            .copied()
            .unwrap_or_else(|| panic!("manifest references unknown chunk id: {id}"));

        let hash_hex = chunk.hash.as_str();
        let hash8 = &hash_hex[..hash_hex.len().min(8)];
        let sri_b64 = hex_to_sri_b64(hash_hex);

        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!(
            "<script src=\"{cdn}/chunks/{hash8}.js\" \
integrity=\"sha256-{sri_b64}\" crossorigin defer></script>"
        ));
    }
    out
}
```

### Step 2.8 — Run the wundler-html test suite

```bash
cargo test -p wundler-html
```

Expected: all 6 tests pass.

### Step 2.9 — Clippy

```bash
cargo clippy -p wundler-html --all-targets -- -D warnings
```

Expected: clean.

### Step 2.10 — Commit

```bash
git add Cargo.toml crates/wundler-html
git commit -m "feat(html): new wundler-html crate with SRI script-tag renderer

- hex_to_sri_b64(): CAS hex -> base64 (SRI body)
- render_script_tags(manifest, entry, cdn_base_url): emits
  <script src integrity=sha256-... crossorigin defer> per chunk
- Panics on dangling chunk id in entry_chunks (startup invariant)
- Added to workspace members"
```

---

## Task 3: `GET /manifest/full.json` endpoint

**Files:**
- Modify: `crates/wundler-abs/Cargo.toml` — add `base64 = "0.22"`
- Modify: `crates/wundler-abs/src/security/auth.rs` — extend `EXEMPT_PATHS`
- Modify: `crates/wundler-abs/src/server.rs` — add route + handler

### Step 3.1 — Add `base64` to wundler-abs

- [ ] Edit `crates/wundler-abs/Cargo.toml`. Append below `governor = "0.7"`:

```toml
base64 = "0.22"
hex = "0.4"
```

### Step 3.2 — Extend `EXEMPT_PATHS`

- [ ] In `crates/wundler-abs/src/security/auth.rs`, replace the `EXEMPT_PATHS` constant:

```rust
/// Routes that always bypass the bearer-token check, regardless of whether
/// security is enabled. These are publicly readable / unauthenticated by
/// design.
///
/// * `/health`              — liveness probe, no secrets.
/// * `/sw.js`               — service worker source, served to any origin.
/// * `/manifest/full.json`  — full manifest JSON; trust anchor is the
///   detached ed25519 signature in the `X-Wundler-Signature` header, not
///   the bearer token.
/// * `/csp-report`          — browsers POST CSP violations unauthenticated.
const EXEMPT_PATHS: &[&str] = &[
    "/health",
    "/sw.js",
    "/manifest/full.json",
    "/csp-report",
];
```

### Step 3.3 — Write the failing integration test for `GET /manifest/full.json`

- [ ] In `crates/wundler-abs/src/server.rs`, append these tests inside `mod tests`:

```rust
    use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
    use ed25519_dalek::{Signer as _, SigningKey};
    use rand::rngs::OsRng;
    use wundler_graph::ChunkManifest;

    fn unsecured(app: AppState, telemetry: TelemetryLogger) -> axum::Router {
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        super::build_router(app, telemetry, security, None)
    }

    #[tokio::test]
    async fn manifest_full_json_returns_manifest_body_and_build_id_header() {
        let (app, telemetry) = test_state();
        let router = unsecured(app, telemetry);

        let resp = router
            .oneshot(Request::builder()
                .uri("/manifest/full.json")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            "application/json"
        );
        assert_eq!(
            resp.headers()
                .get("x-wundler-build-id")
                .expect("X-Wundler-Build-Id present")
                .to_str()
                .unwrap(),
            "test-build"
        );
        // No signing key configured → no signature header.
        assert!(resp.headers().get("x-wundler-signature").is_none());

        let cache = resp.headers().get(CACHE_CONTROL).unwrap().to_str().unwrap();
        assert!(cache.contains("public"));
        assert!(cache.contains("immutable"));
        assert!(cache.contains("max-age=60"));

        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let parsed: ChunkManifest = serde_json::from_slice(&bytes).expect("body is manifest JSON");
        assert_eq!(parsed.build_id, "test-build");
    }

    #[tokio::test]
    async fn manifest_full_json_emits_signature_header_when_signature_present() {
        let (app, telemetry) = test_state();

        // Sign the test manifest and stash the signature.
        let sk = SigningKey::generate(&mut OsRng);
        let manifest = app.snapshot().await;
        let bytes = crate::signing::manifest_signature_bytes(&manifest);
        let sig = sk.sign(&bytes);
        app.set_signature(Some(sig)).await;

        let router = unsecured(app, telemetry);
        let resp = router
            .oneshot(Request::builder()
                .uri("/manifest/full.json")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let header = resp
            .headers()
            .get("x-wundler-signature")
            .expect("X-Wundler-Signature present when signature is stored")
            .to_str()
            .unwrap()
            .to_string();

        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&header)
            .expect("signature header is valid base64");
        assert_eq!(decoded.len(), 64, "ed25519 signatures are 64 bytes");
        assert_eq!(decoded.as_slice(), sig.to_bytes().as_slice());
    }

    #[tokio::test]
    async fn manifest_full_json_bypasses_bearer_auth() {
        // Build a router with bearer auth ENABLED but no token in the request.
        let token = SecretTokenForTest::build();
        let security = Arc::new(ResolvedSecurity {
            token: Some(token),
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let router = super::build_router(app, telemetry, security, None);

        // No Authorization header → must still 200 because /manifest/full.json
        // is in EXEMPT_PATHS.
        let resp = router
            .oneshot(Request::builder()
                .uri("/manifest/full.json")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Internal helper to construct a `SecretToken` from a fixed string
    /// (we can't `impl Clone` on `SecretToken` without leaking the bytes).
    struct SecretTokenForTest;
    impl SecretTokenForTest {
        fn build() -> crate::security::auth::SecretToken {
            crate::security::auth::SecretToken::new(b"unit-test-token".to_vec())
        }
    }
```

> Note: this test calls `super::build_router(app, telemetry, security, None)` — a 4-arg signature. The existing 3-arg call in `router_with_ip` must also be updated. Do that in Step 3.5.

### Step 3.4 — Run the failing test

```bash
cargo test -p wundler-abs --lib server::tests::manifest_full_json_returns_manifest_body_and_build_id_header
```

Expected: compile error — `build_router` takes 3 args, not 4; route doesn't exist.

### Step 3.5 — Update `build_router` and `RouterState`; add the handler

- [ ] In `crates/wundler-abs/src/server.rs`, change the `RouterState` struct:

```rust
/// State threaded through every Axum handler.
///
/// Cheap to `Clone` — every field is internally reference-counted.
#[derive(Clone)]
struct RouterState {
    app: AppState,
    telemetry: TelemetryLogger,
    /// Active signer for `POST /reload`. `None` disables auto-re-signing.
    signer: Option<Arc<crate::signing::ManifestSigner>>,
}
```

- [ ] Update `build_router` to take an optional signer and register the new route:

```rust
/// Build and return the Axum [`Router`] without starting a listener.
pub fn build_router(
    app: AppState,
    telemetry: TelemetryLogger,
    security: Arc<ResolvedSecurity>,
    signer: Option<Arc<crate::signing::ManifestSigner>>,
) -> Router {
    let state = RouterState { app, telemetry, signer };
    let cors = build_cors(&security);

    let manifest_route = match security.rate_limiter.clone() {
        None => Router::new().route("/manifest", post(post_manifest)),
        Some(limiter) => Router::new()
            .route("/manifest", post(post_manifest))
            .route_layer(axum::middleware::from_fn_with_state(
                limiter,
                crate::security::ratelimit::rate_limit_mw,
            )),
    };

    Router::new()
        .merge(manifest_route)
        .route("/manifest/full.json", get(get_manifest_full_json))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        .route("/select", post(post_select))
        .route("/versions", get(get_versions))
        // Inner: bearer-token authentication.
        .layer(middleware::from_fn_with_state(security, require_bearer))
        // Outer: CORS — applied last so it wraps the auth layer.
        .layer(cors)
        .with_state(state)
}
```

- [ ] Update the `run()` function to pass `None` for the signer (Task 5 wires the real one in):

```rust
pub async fn run(config: AbsConfig) -> Result<()> {
    let archive = crate::archive::ManifestArchive::open(
        &config.archive_dir,
        config.archive_retention,
    )
    .context("failed to open manifest archive")?;

    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
        archive,
    )
    .await?;

    let telemetry = TelemetryLogger::new(&config.telemetry_log)?;
    let security = ResolvedSecurity::from_config(&config.security)
        .context("failed to initialise security config")?;

    // Task 5 will load a signer here when config.signing_key_pem is set.
    let signer: Option<Arc<crate::signing::ManifestSigner>> = None;

    let router = build_router(app, telemetry, Arc::new(security), signer);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("wundler-abs listening on {}", local_addr);

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}
```

- [ ] Add the handler. Place it next to `get_health` in `server.rs`:

```rust
/// `GET /manifest/full.json` — return the full manifest JSON.
///
/// Public endpoint (no bearer required). The trust anchor is the detached
/// signature returned in `X-Wundler-Signature`, not the transport.
///
/// Response headers:
/// * `Content-Type: application/json`
/// * `X-Wundler-Build-Id: <build_id>`
/// * `X-Wundler-Signature: <base64(sig.to_bytes())>` (only when a signature
///    is currently stored in `AppState`).
/// * `Cache-Control: public, max-age={ttl_seconds}, immutable`
async fn get_manifest_full_json(State(state): State<RouterState>) -> Response {
    use base64::Engine as _;

    let manifest = state.app.snapshot().await;
    let sig = state.app.snapshot_signature().await;

    let body = match serde_json::to_vec(&*manifest) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to serialise manifest: {e}"),
            )
                .into_response();
        }
    };

    let build_id_header = match axum::http::HeaderValue::from_str(&manifest.build_id) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "manifest build_id is not a valid HTTP header value",
            )
                .into_response();
        }
    };

    let cache_value = format!(
        "public, max-age={}, immutable",
        state.app.ttl_seconds
    );

    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-wundler-build-id", build_id_header)
        .header(header::CACHE_CONTROL, cache_value)
        .body(axum::body::Body::from(body))
        .expect("static-shape response must build");

    if let Some(sig) = sig {
        let b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
        if let Ok(hv) = axum::http::HeaderValue::from_str(&b64) {
            resp.headers_mut().insert("x-wundler-signature", hv);
        }
    }

    resp
}
```

- [ ] Inside `mod tests`, update `router_with_ip` to pass the new 4th argument:

```rust
    fn router_with_ip(
        app: AppState,
        telemetry: TelemetryLogger,
        security: Arc<ResolvedSecurity>,
        ip: IpAddr,
    ) -> axum::Router {
        let inner = super::build_router(app, telemetry, security, None);
        inner.layer(middleware::from_fn(
            move |mut req: Request<Body>, next: middleware::Next| async move {
                let ci: ConnectInfo<SocketAddr> = ConnectInfo(SocketAddr::new(ip, 49_152));
                req.extensions_mut().insert(ci);
                next.run(req).await
            },
        ))
    }
```

### Step 3.6 — Run the new tests

```bash
cargo test -p wundler-abs --lib server::tests::manifest_full_json
```

Expected: all three `manifest_full_json_*` tests pass.

### Step 3.7 — Full wundler-abs suite

```bash
cargo test -p wundler-abs
```

Expected: clean.

### Step 3.8 — Clippy

```bash
cargo clippy -p wundler-abs --all-targets -- -D warnings
```

Expected: clean.

### Step 3.9 — Commit

```bash
git add crates/wundler-abs
git commit -m "feat(abs): GET /manifest/full.json public endpoint

- Returns full manifest JSON with X-Wundler-Build-Id header
- X-Wundler-Signature header (base64) when a signature is stored
- Cache-Control: public, max-age=<ttl>, immutable
- Exempt from bearer auth (EXEMPT_PATHS)
- build_router signature gains optional ManifestSigner arg (Task 5 wires it in)"
```

---

## Task 4: CSP module + `POST /csp-report`

**Files:**
- Create: `crates/wundler-abs/src/security/csp.rs`
- Modify: `crates/wundler-abs/src/security/mod.rs` — declare module
- Modify: `crates/wundler-abs/src/server.rs` — register route + handler

### Step 4.1 — Declare the new module

- [ ] In `crates/wundler-abs/src/security/mod.rs`, change the module declarations near the top to:

```rust
pub mod auth;
pub mod cors;
pub mod csp;
pub mod ratelimit;
```

### Step 4.2 — Write the failing unit tests for `csp.rs`

- [ ] Create `crates/wundler-abs/src/security/csp.rs`:

```rust
//! Content-Security-Policy helpers.
//!
//! Phase 2 emits **report-only** CSP — never enforcement mode — so a
//! mis-configured policy can never break the app. The header carries one
//! `'sha256-…'` hash per chunk in the manifest plus a `report-uri` pointing
//! at our `POST /csp-report` ingester.

use axum::http::{HeaderName, HeaderValue};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use wundler_graph::ChunkManifest;

/// Convert a hex-encoded SHA-256 digest (CAS format) to the base64 form
/// that CSP `script-src` directives expect inside `'sha256-…'`.
///
/// # Panics
///
/// Panics if `hex` is not valid hexadecimal — CAS hashes are produced by
/// `wundler-core::ContentHash::from_bytes` and are always valid hex.
pub fn hex_to_sri_b64(hex: &str) -> String {
    let bytes = hex::decode(hex).expect("CAS hash must be valid hex");
    STANDARD.encode(bytes)
}

/// Build a `Content-Security-Policy-Report-Only` header for the given
/// manifest.
///
/// Format:
/// ```text
/// script-src 'sha256-<b64>' 'sha256-<b64>' ... ; report-uri /csp-report
/// ```
///
/// The header name returned is always
/// `content-security-policy-report-only` — never the enforcement variant.
pub fn build_csp_report_only(manifest: &ChunkManifest) -> (HeaderName, HeaderValue) {
    let mut hashes: Vec<String> = manifest
        .chunks
        .iter()
        .map(|c| format!("'sha256-{}'", hex_to_sri_b64(c.hash.as_str())))
        .collect();
    // Deterministic ordering (chunk vec order is already deterministic but
    // sort defends against future reordering).
    hashes.sort();

    let value = if hashes.is_empty() {
        "script-src 'none'; report-uri /csp-report".to_string()
    } else {
        format!("script-src {}; report-uri /csp-report", hashes.join(" "))
    };

    let name = HeaderName::from_static("content-security-policy-report-only");
    // Any non-ASCII byte in a chunk hash would be a programmer error.
    let value = HeaderValue::from_str(&value)
        .expect("CSP header value must be ASCII (b64+digits+spaces only)");
    (name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wundler_core::types::ContentHash;
    use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

    fn manifest_with_chunks(hashes: &[&str]) -> ChunkManifest {
        let chunks = hashes
            .iter()
            .enumerate()
            .map(|(i, h)| Chunk {
                id: format!("chunk-{i}"),
                modules: vec![],
                hash: ContentHash((*h).to_string()),
                load_condition: LoadCondition::Initial,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            })
            .collect();
        ChunkManifest {
            build_id: "b0".to_string(),
            chunks,
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        }
    }

    #[test]
    fn hex_to_sri_b64_matches_known_value() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(hex_to_sri_b64(hex), "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=");
    }

    #[test]
    fn build_csp_returns_report_only_header_name() {
        let m = manifest_with_chunks(&[]);
        let (name, _) = build_csp_report_only(&m);
        assert_eq!(name.as_str(), "content-security-policy-report-only");
    }

    #[test]
    fn empty_manifest_uses_script_src_none() {
        let m = manifest_with_chunks(&[]);
        let (_, value) = build_csp_report_only(&m);
        let s = value.to_str().unwrap();
        assert!(s.contains("script-src 'none'"));
        assert!(s.contains("report-uri /csp-report"));
    }

    #[test]
    fn non_empty_manifest_emits_one_sha256_per_chunk() {
        let m = manifest_with_chunks(&[
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447",
        ]);
        let (_, value) = build_csp_report_only(&m);
        let s = value.to_str().unwrap();
        assert_eq!(s.matches("'sha256-").count(), 2);
        assert!(s.contains("'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU='"));
        assert!(s.ends_with("; report-uri /csp-report"));
    }
}
```

### Step 4.3 — Run the CSP unit tests

```bash
cargo test -p wundler-abs --lib security::csp
```

Expected: 4 tests pass.

### Step 4.4 — Write the failing integration tests for `POST /csp-report`

- [ ] In `crates/wundler-abs/src/server.rs`, append to `mod tests`:

```rust
    #[tokio::test]
    async fn csp_report_accepts_small_body_and_returns_200() {
        let (app, telemetry) = test_state();
        let router = unsecured(app, telemetry);

        let body = serde_json::json!({
            "csp-report": {
                "document-uri": "https://app.example.com/",
                "violated-directive": "script-src",
                "blocked-uri": "inline"
            }
        });

        let resp = router
            .oneshot(Request::builder()
                .method("POST")
                .uri("/csp-report")
                .header("content-type", "application/csp-report")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn csp_report_rejects_oversize_body_with_413() {
        let (app, telemetry) = test_state();
        let router = unsecured(app, telemetry);

        // 8 KiB + 1 byte
        let huge = vec![b'x'; 8 * 1024 + 1];

        let resp = router
            .oneshot(Request::builder()
                .method("POST")
                .uri("/csp-report")
                .header("content-type", "application/csp-report")
                .body(Body::from(huge))
                .unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn csp_report_bypasses_bearer_auth() {
        let security = Arc::new(ResolvedSecurity {
            token: Some(SecretTokenForTest::build()),
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let (app, telemetry) = test_state();
        let router = super::build_router(app, telemetry, security, None);

        let resp = router
            .oneshot(Request::builder()
                .method("POST")
                .uri("/csp-report")
                .header("content-type", "application/csp-report")
                .body(Body::from("{}"))
                .unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
```

### Step 4.5 — Run the failing test

```bash
cargo test -p wundler-abs --lib server::tests::csp_report
```

Expected: route 404s (not registered yet).

### Step 4.6 — Add the handler + route

- [ ] In `crates/wundler-abs/src/server.rs`, add the handler. Use a JSONL line via `tracing` (we don't need a separate sink — telemetry log is for `TelemetryEvent`s, not browser reports):

```rust
/// `POST /csp-report` — sink for browser CSP violation reports.
///
/// Body limit: **8 KiB**. Bodies larger than this are rejected with `413
/// Payload Too Large` to bound memory use even when the request is unauth.
/// Smaller bodies are logged via `tracing::warn!` (which the operator can
/// pipe to a JSONL log) and we always return `200 OK`. The endpoint is
/// **exempt** from bearer auth because browsers POST CSP reports without
/// credentials.
async fn post_csp_report(body: axum::body::Body) -> Response {
    const MAX_CSP_REPORT_BYTES: usize = 8 * 1024;

    let bytes = match axum::body::to_bytes(body, MAX_CSP_REPORT_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "csp-report body too large")
                .into_response();
        }
    };

    // Lossy UTF-8 conversion is fine — this is a log line, not a parser.
    let body_str = String::from_utf8_lossy(&bytes);
    tracing::warn!(target: "csp_report", body = %body_str, "CSP violation report");

    StatusCode::OK.into_response()
}
```

- [ ] Register the route in `build_router` — replace the existing route chain with:

```rust
    Router::new()
        .merge(manifest_route)
        .route("/manifest/full.json", get(get_manifest_full_json))
        .route("/health", get(get_health))
        .route("/sw.js", get(get_service_worker))
        .route("/reload", post(post_reload))
        .route("/select", post(post_select))
        .route("/versions", get(get_versions))
        .route("/csp-report", post(post_csp_report))
        .layer(middleware::from_fn_with_state(security, require_bearer))
        .layer(cors)
        .with_state(state)
```

### Step 4.7 — Run the CSP integration tests

```bash
cargo test -p wundler-abs --lib server::tests::csp_report
```

Expected: 3 tests pass.

### Step 4.8 — Full suite + clippy

```bash
cargo test -p wundler-abs
cargo clippy -p wundler-abs --all-targets -- -D warnings
```

Expected: clean.

### Step 4.9 — Commit

```bash
git add crates/wundler-abs
git commit -m "feat(abs): CSP report-only header builder + POST /csp-report sink

- security/csp.rs: build_csp_report_only(manifest) returns header tuple
- hex_to_sri_b64() helper (local to ABS; mirrors wundler-html)
- POST /csp-report accepts <=8 KiB bodies, logs via tracing, returns 200
- Bodies >8 KiB rejected with 413
- Route is exempt from bearer auth (browsers send unauthenticated)"
```

---

## Task 5: Wire signing into `POST /reload` + cross-language contract test

**Files:**
- Modify: `crates/wundler-abs/src/server.rs` — load signer in `run()`, re-sign in `post_reload`
- Create: `crates/wundler-abs/tests/signing_contract_test.rs`

### Step 5.1 — Write the failing integration test for re-signing on reload

- [ ] In `crates/wundler-abs/src/server.rs`, append to `mod tests`:

```rust
    use base64::Engine as _;
    use crate::signing::{generate_keypair, ManifestSigner, ManifestVerifier};

    #[tokio::test]
    async fn reload_resigns_manifest_when_signer_configured() {
        let (app, telemetry) = test_state();
        let kp = generate_keypair();
        let signer = Arc::new(ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer"));
        let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem).expect("verifier");

        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let router = super::build_router(app.clone(), telemetry, security, Some(signer));

        // Write a fresh manifest with a NEW build id to a temp file.
        let tmp = tempfile::NamedTempFile::new().expect("tmp manifest file");
        let new_manifest = ChunkManifest {
            build_id: "build-resigned".to_string(),
            chunks: vec![],
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        };
        std::fs::write(tmp.path(), serde_json::to_vec(&new_manifest).unwrap()).unwrap();

        let body = serde_json::json!({"manifest_path": tmp.path().to_str().unwrap()});

        let resp = router
            .clone()
            .oneshot(Request::builder()
                .method("POST")
                .uri("/reload")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "/reload must succeed");

        // After /reload, the signature slot should hold a verifiable signature.
        let sig = app
            .snapshot_signature()
            .await
            .expect("signature must be stored after /reload with signer");
        let active = app.snapshot().await;
        assert_eq!(active.build_id, "build-resigned");
        verifier
            .verify(&active, &sig)
            .expect("re-signed manifest must verify against the matching public key");

        // And the new sig must be visible on /manifest/full.json.
        let resp = router
            .oneshot(Request::builder()
                .uri("/manifest/full.json")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let header = resp.headers().get("x-wundler-signature").unwrap().to_str().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(header).unwrap();
        assert_eq!(decoded.as_slice(), sig.to_bytes().as_slice());
    }

    #[tokio::test]
    async fn reload_does_not_set_signature_when_no_signer() {
        let (app, telemetry) = test_state();
        let security = Arc::new(ResolvedSecurity {
            token: None,
            allowed_origins: vec![],
            rate_limiter: None,
        });
        let router = super::build_router(app.clone(), telemetry, security, None);

        let tmp = tempfile::NamedTempFile::new().expect("tmp manifest file");
        let new_manifest = ChunkManifest {
            build_id: "build-no-sig".to_string(),
            chunks: vec![],
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        };
        std::fs::write(tmp.path(), serde_json::to_vec(&new_manifest).unwrap()).unwrap();
        let body = serde_json::json!({"manifest_path": tmp.path().to_str().unwrap()});

        let resp = router
            .oneshot(Request::builder()
                .method("POST")
                .uri("/reload")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(
            app.snapshot_signature().await.is_none(),
            "no signer ⇒ no signature after /reload"
        );
    }
```

### Step 5.2 — Run the failing test

```bash
cargo test -p wundler-abs --lib server::tests::reload_resigns
```

Expected: `app.snapshot_signature()` returns `None` — the handler doesn't sign yet.

### Step 5.3 — Implement re-signing in `post_reload`

- [ ] In `crates/wundler-abs/src/server.rs`, replace the existing `post_reload` function with this version (adds the re-sign block at the end):

```rust
async fn post_reload(
    MaybeConnectAddr(addr): MaybeConnectAddr,
    State(state): State<RouterState>,
    Json(body): Json<ReloadRequest>,
) -> impl IntoResponse {
    // Enforce loopback when ConnectInfo is present (production TCP mode).
    if let Some(addr) = addr {
        if !addr.ip().is_loopback() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "reload is only permitted from loopback addresses"
                })),
            )
                .into_response();
        }
    }

    let json_bytes = match tokio::fs::read(&body.manifest_path).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("failed to read manifest file: {e}")
                })),
            )
                .into_response();
        }
    };

    let new_manifest: ChunkManifest = match serde_json::from_slice(&json_bytes) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("invalid manifest JSON: {e}")
                })),
            )
                .into_response();
        }
    };

    let build_id = new_manifest.build_id.clone();
    if let Err(e) = state.app.archive.install(&new_manifest) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("failed to install manifest into archive: {e}")
            })),
        )
            .into_response();
    }

    let report = match state.app.swap_to(&build_id).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("failed to swap to new manifest: {e}")
                })),
            )
                .into_response();
        }
    };

    // Re-sign the newly-active manifest if a signing key is configured.
    // `swap_to` already cleared the previous signature; we replace it with a
    // fresh one signed over the canonical bytes of the new manifest.
    if let Some(signer) = state.signer.as_ref() {
        let active = state.app.snapshot().await;
        let sig = signer.sign_manifest(&active);
        state.app.set_signature(Some(sig)).await;
    }

    (StatusCode::OK, Json(SwapResponseBody {
        previous: report.previous,
        current: report.current,
    }))
        .into_response()
}
```

- [ ] Update `run()` to actually load the signer from the configured PEM file when present:

```rust
pub async fn run(config: AbsConfig) -> Result<()> {
    let archive = crate::archive::ManifestArchive::open(
        &config.archive_dir,
        config.archive_retention,
    )
    .context("failed to open manifest archive")?;

    let app = AppState::load_from_disk(
        &config.manifest_path,
        config.cdn_base_url.clone(),
        config.ttl_seconds,
        archive,
    )
    .await?;

    let telemetry = TelemetryLogger::new(&config.telemetry_log)?;
    let security = ResolvedSecurity::from_config(&config.security)
        .context("failed to initialise security config")?;

    let signer: Option<Arc<crate::signing::ManifestSigner>> = match &config.signing_key_pem {
        None => None,
        Some(path) => {
            let pem = std::fs::read_to_string(path).with_context(|| {
                format!("failed to read signing key PEM at {}", path.display())
            })?;
            let s = crate::signing::ManifestSigner::from_pem(&pem)
                .context("failed to parse signing key PEM")?;
            Some(Arc::new(s))
        }
    };

    let router = build_router(app, telemetry, Arc::new(security), signer);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!("wundler-abs listening on {}", local_addr);

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}
```

### Step 5.4 — Run the reload tests

```bash
cargo test -p wundler-abs --lib server::tests::reload_
```

Expected: both new reload tests pass.

### Step 5.5 — Write the cross-language contract test (Rust half)

- [ ] Create `crates/wundler-abs/tests/signing_contract_test.rs`:

```rust
//! Cross-language contract test for `manifest_signature_bytes`.
//!
//! This test pins the **canonical byte format** that both Rust and the
//! Service Worker JavaScript verifier must agree on. If you ever change the
//! format in `signing::manifest_signature_bytes`, this test will fail —
//! and so will every existing Service Worker in the field.
//!
//! ## Out-of-scope: the JavaScript half
//!
//! A companion script `tests/sw-verify.mjs` (separate PR — Phase 2 C3) must:
//!
//! 1. Read `target/contract-fixtures/manifest.json`.
//! 2. Re-derive the canonical bytes using the exact algorithm documented in
//!    `signing::manifest_signature_bytes`:
//!       * Push `build_id` then `\n`.
//!       * Sort chunks by id (lexicographic, ascending).
//!       * For each chunk: push `id`, ` `, `hash_hex`, `\n`.
//! 3. Compare those bytes (hex-encoded) against the contents of
//!    `target/contract-fixtures/signing-bytes.hex` — they must match exactly.
//! 4. ed25519-verify `target/contract-fixtures/signature.b64` against those
//!    bytes using the public key in `target/contract-fixtures/public.pem`.
//!
//! As long as steps 1–4 succeed, the SW will be able to verify any manifest
//! signed by ABS at runtime.

use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine as _;
use wundler_abs::signing::{generate_keypair, manifest_signature_bytes, ManifestSigner, ManifestVerifier};
use wundler_core::types::ContentHash;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

/// A small but representative manifest:
/// * two chunks
/// * non-trivial entry-chunks map (which `manifest_signature_bytes` ignores)
/// * advisory PGO fields populated on one chunk (also ignored by signing)
fn fixture_manifest() -> ChunkManifest {
    let chunks = vec![
        Chunk {
            id: "z-last".to_string(),
            modules: vec![],
            hash: ContentHash(
                "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447".to_string(),
            ),
            load_condition: LoadCondition::Lazy,
            co_request_score: Some(0.42),
            median_load_order: Some(3.0),
            suggested_merge: Some("a-first".to_string()),
        },
        Chunk {
            id: "a-first".to_string(),
            modules: vec![],
            hash: ContentHash(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
            ),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        },
    ];
    let mut entry_chunks = HashMap::new();
    entry_chunks.insert("main".to_string(), vec!["a-first".to_string(), "z-last".to_string()]);
    ChunkManifest {
        build_id: "contract-fixture-1".to_string(),
        chunks,
        entry_chunks,
        module_index: HashMap::new(),
    }
}

/// The exact bytes the SW must reproduce:
///   contract-fixture-1\n
///   a-first e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n
///   z-last a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447\n
const EXPECTED_BYTES: &[u8] = b"contract-fixture-1\na-first e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nz-last a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447\n";

#[test]
fn signing_bytes_are_byte_for_byte_stable() {
    let m = fixture_manifest();
    let bytes = manifest_signature_bytes(&m);
    assert_eq!(
        bytes.as_slice(),
        EXPECTED_BYTES,
        "manifest_signature_bytes() output drifted; \
         this breaks every deployed SW — fix the implementation, do NOT update this constant \
         unless you are coordinating a fleet-wide SW redeploy."
    );
}

#[test]
fn signing_bytes_ignore_advisory_pgo_fields() {
    // Mutate the PGO fields on the first chunk and confirm the bytes are unchanged.
    let baseline = manifest_signature_bytes(&fixture_manifest());

    let mut m = fixture_manifest();
    m.chunks[0].co_request_score = Some(0.99);
    m.chunks[0].median_load_order = Some(99.0);
    m.chunks[0].suggested_merge = Some("totally-different".to_string());
    let mutated = manifest_signature_bytes(&m);

    assert_eq!(
        baseline, mutated,
        "co_request_score / median_load_order / suggested_merge must NOT affect signing bytes"
    );
}

#[test]
fn signing_bytes_ignore_entry_chunks_map_order() {
    let baseline = manifest_signature_bytes(&fixture_manifest());

    let mut m = fixture_manifest();
    m.entry_chunks
        .insert("admin".to_string(), vec!["z-last".to_string()]);
    let with_extra_entry = manifest_signature_bytes(&m);

    assert_eq!(
        baseline, with_extra_entry,
        "entry_chunks must NOT affect signing bytes — only build_id + chunks (id, hash)"
    );
}

#[test]
fn round_trip_sign_then_verify_succeeds() {
    let m = fixture_manifest();
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer");
    let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem).expect("verifier");
    let sig = signer.sign_manifest(&m);
    verifier.verify(&m, &sig).expect("freshly signed manifest must verify");
}

/// Emit fixture artifacts to `target/contract-fixtures/` for the future
/// Node-side verifier. Marked `#[ignore]` by default so CI doesn't pollute
/// `target/` on every run; the JS PR will run it explicitly with
/// `cargo test -p wundler-abs --test signing_contract_test -- --ignored`.
#[test]
#[ignore]
fn emit_fixture_artifacts() {
    let m = fixture_manifest();
    let bytes = manifest_signature_bytes(&m);
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer");
    let sig = signer.sign_manifest(&m);

    let dir: PathBuf = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("contract-fixtures");
    std::fs::create_dir_all(&dir).expect("create fixture dir");

    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&m).expect("serialise manifest"),
    )
    .expect("write manifest.json");

    std::fs::write(dir.join("signing-bytes.hex"), hex::encode(&bytes))
        .expect("write signing-bytes.hex");

    std::fs::write(
        dir.join("signature.b64"),
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
    )
    .expect("write signature.b64");

    std::fs::write(dir.join("public.pem"), &kp.verifying_key_pem)
        .expect("write public.pem");

    eprintln!("contract fixtures emitted to {}", dir.display());
}
```

> Note: the `#[ignore]` test uses `env!("CARGO_TARGET_TMPDIR")` which is set automatically by cargo for integration tests; no extra config needed. The non-ignored tests do *not* touch the filesystem and run in normal CI.

- [ ] Add `hex` and `wundler-core` to wundler-abs `[dev-dependencies]` so the test compiles. Edit `crates/wundler-abs/Cargo.toml`'s `[dev-dependencies]` section:

```toml
[dev-dependencies]
axum-test = "20"
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tower = { version = "0.5", features = ["util"] }
hex = "0.4"
wundler-core = { path = "../wundler-core" }
```

### Step 5.6 — Run the contract test

```bash
cargo test -p wundler-abs --test signing_contract_test
```

Expected: 4 tests pass (the `#[ignore]` one is skipped).

Then verify the emit path also works:

```bash
cargo test -p wundler-abs --test signing_contract_test -- --ignored
```

Expected: 1 ignored test passes; prints the fixture directory.

### Step 5.7 — Full workspace suite + clippy + fmt

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Expected: clean.

### Step 5.8 — Commit

```bash
git add crates/wundler-abs crates/wundler-html
git commit -m "feat(abs): re-sign manifest on /reload + cross-language contract test

- post_reload now re-signs the swapped-in manifest when a signing key is
  configured; signature is stored in AppState and surfaced via /manifest/full.json
- run() loads the signer from AbsConfig.signing_key_pem when present
- tests/signing_contract_test.rs pins the canonical byte format for the
  Service Worker JavaScript verifier (Phase 2 C3) — the EXPECTED_BYTES
  constant is the contract; do not mutate without a fleet-wide SW redeploy"
```

---

## Acceptance Checklist

Run all of these from the repo root and confirm green output.

- [ ] `cargo test -p wundler-abs` — all unit + integration tests pass
- [ ] `cargo test -p wundler-html` — 6 tests pass
- [ ] `cargo test --workspace` — full workspace green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — no warnings
- [ ] `cargo fmt --all -- --check` — clean
- [ ] `GET /manifest/full.json` (via integration test `manifest_full_json_returns_manifest_body_and_build_id_header`) returns 200, JSON body, `X-Wundler-Build-Id` header, `Cache-Control: public, max-age=<ttl>, immutable`
- [ ] When a signing key is configured and `/reload` succeeded, `X-Wundler-Signature` is present and base64-decodes to 64 bytes (`reload_resigns_manifest_when_signer_configured`)
- [ ] When no signing key is configured, `X-Wundler-Signature` is absent (`manifest_full_json_returns_manifest_body_and_build_id_header`)
- [ ] `POST /csp-report` with an 8 KiB+1 byte body returns `413 Payload Too Large` (`csp_report_rejects_oversize_body_with_413`)
- [ ] `POST /csp-report` with a small JSON body returns `200 OK` and logs via `tracing` (`csp_report_accepts_small_body_and_returns_200`)
- [ ] `POST /csp-report` and `GET /manifest/full.json` bypass bearer auth even when the rest of the router requires it (`csp_report_bypasses_bearer_auth`, `manifest_full_json_bypasses_bearer_auth`)
- [ ] `wundler_html::render_script_tags` emits `<script src integrity="sha256-<b64>" crossorigin defer>` per chunk in entry order
- [ ] `wundler_html::hex_to_sri_b64` converts the known sha256("") hash to `47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=`
- [ ] `manifest_signature_bytes` produces the byte-for-byte fixture in `signing_contract_test::signing_bytes_are_byte_for_byte_stable` — this is the cross-language contract anchor

---

## Self-Review Notes

**Spec coverage check (✓ = covered, → task):**

- `AppState.signature` field, `snapshot_signature`, swap reset → Task 1
- `GET /manifest/full.json` with all four headers → Task 3
- `POST /csp-report` (8 KiB cap, exempt, 200/413) → Task 4
- `security/csp.rs` (`build_csp_report_only`, `hex_to_sri_b64`) → Task 4
- `wundler-html` crate (`render_script_tags`, `sri::hex_to_sri_b64`, startup-invariant panic) → Task 2
- Re-sign on `POST /reload` → Task 5
- Rust half of contract test (`manifest_signature_bytes` byte stability) → Task 5
- All exemptions in `EXEMPT_PATHS` → Task 3 (`/manifest/full.json`, `/csp-report`)
- Workspace `members` updated → Task 2
- `base64`/`hex` deps added → Task 3 (abs), Task 2 (html), Task 5 (abs dev-deps)

**Out-of-scope confirmations:** SW JS verifier, key pinning, CSP enforce mode, JWT/OAuth/mTLS, key rotation, `session_id`/`build_id` bug — none of these appear in any task. ✓

**Type/signature consistency:**
- `build_router(app, telemetry, security, signer)` — 4-arg form used consistently in Task 3, Task 4, and Task 5 (`router_with_ip` helper updated in Task 3.5, `unsecured` helper introduced in Task 3.3).
- `RouterState { app, telemetry, signer }` — `signer` field added in Task 3.5, consumed in Task 5.3.
- `AppState.set_signature(Option<Signature>)` / `snapshot_signature() -> Option<Signature>` — defined in Task 1.3, used in Task 3, Task 5.
- `hex_to_sri_b64` exists in both `wundler_html::sri` (Task 2.3) and `wundler_abs::security::csp` (Task 4.2). This is intentional duplication — ABS doesn't depend on wundler-html and we don't want to add a path dep just for one helper. Both implementations have the same known-answer test against `sha256("")`.

**Placeholder scan:** every code block is complete; every command has expected output stated.
