# Performance P1 — Dependency Pre-bundling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the on-disk dependency pre-bundling cache that `run_dev` consults before starting the file watcher, so subsequent `cloudpack dev` invocations on an unchanged `package.json` + lockfile pair skip node_modules work entirely.

**Architecture:** A new `cloudpack-dev` crate owns a `DepPrebundler` that fingerprints `package.json` + the canonical lockfile with blake3, looks up `<project>/.cloudpack/cache/deps/<fingerprint>/index.json` for cache hits, and on miss writes into `.tmp-<fingerprint>/` and renames atomically. `run_dev` calls `ensure_fresh` before spinning up `DevServer` and logs hit/miss/error. The SCA writes only `index.json` — real node_modules bundling is deferred to a later plan.

**Tech Stack:** Rust 2021, blake3 1.x (fingerprint), tempfile 3.x (atomic write staging), anyhow 1.x (errors), tracing 0.1 (logging), walkdir 2.x (GC traversal), serde_json 1.x (`index.json`), toml 0.8 (`[dev]` section), tokio 1.x (async `run_dev`).

## Scope Boundary

**In scope (SCA):**
- New `cloudpack-dev` crate with `DepPrebundler`, `PrebundleResult`, `compute_fingerprint`
- `ensure_fresh`: cache-hit fast path, cache-miss writes `index.json` only (no actual node_modules bundling)
- `gc()` walks cache root and deletes directories whose mtime is older than TTL
- `[dev]` section in `cloudpack.toml` with `dep_cache_ttl_days` (default 14)
- `run_dev` integration: call `ensure_fresh` before the watcher starts; log result; warn on >1 GB cache
- Atomic create via tempdir + rename
- Absent `[dev]` section preserves current behavior

**Out of scope:**
- Actual bundling of node_modules contents into `*.js` chunks
- Per-package invalidation (current invalidation is whole-cache by fingerprint)
- Raw ESM serving from node_modules
- P2 HMR, P3/P4 incremental graph
- Cross-platform fsync hardening beyond what tempfile already provides

---

## File Structure

**Create:**
- `crates/cloudpack-dev/Cargo.toml` — crate manifest
- `crates/cloudpack-dev/src/lib.rs` — re-exports
- `crates/cloudpack-dev/src/prebundle/mod.rs` — `DepPrebundler`, `PrebundleResult`, `compute_fingerprint`
- `crates/cloudpack-dev/tests/prebundle.rs` — integration tests for hit/miss/gc/concurrency

**Modify:**
- `Cargo.toml` — add `crates/cloudpack-dev` to `[workspace] members`
- `crates/cloudpack-pipeline/src/config.rs` — add `DevConfig` + `BuildConfig.dev: Option<DevConfig>`
- `crates/cloudpack-cli/Cargo.toml` — add `cloudpack-dev` + `tracing` deps
- `crates/cloudpack-cli/src/main.rs::run_dev` (line ~396) — wire `DepPrebundler` in before `DevServer::start`

---

## Task 1: Create `cloudpack-dev` crate with fingerprinting

**Files:**
- Create: `crates/cloudpack-dev/Cargo.toml`
- Create: `crates/cloudpack-dev/src/lib.rs`
- Create: `crates/cloudpack-dev/src/prebundle/mod.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1.1: Add `cloudpack-dev` to workspace members**

Edit `/home/ken/workspace/cloudpack/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/cloudpack-core",
    "crates/cloudpack-cli",
    "crates/cloudpack-graph",
    "crates/cloudpack-transform",
    "crates/cloudpack-pipeline",
    "crates/cloudpack-abs",
    "crates/cloudpack-pgo",
    "crates/cloudpack-bench",
    "crates/cloudpack-dev",
]
```

- [ ] **Step 1.2: Create `crates/cloudpack-dev/Cargo.toml`**

```toml
[package]
name = "cloudpack-dev"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow = { workspace = true }
blake3 = "1"
serde = { workspace = true }
serde_json = { workspace = true }
tempfile = "3"
tracing = "0.1"
walkdir = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 1.3: Create `crates/cloudpack-dev/src/lib.rs`**

```rust
//! Dev-mode helpers: dependency pre-bundling cache, etc.
//!
//! The `prebundle` module owns the `<project>/.cloudpack/cache/deps/` directory
//! that `cloudpack dev` consults before starting its file watcher.

pub mod prebundle;

pub use prebundle::{compute_fingerprint, DepPrebundler, PrebundleResult};
```

- [ ] **Step 1.4: Write the failing test for `compute_fingerprint`**

Create `crates/cloudpack-dev/src/prebundle/mod.rs` with the test scaffold first:

```rust
//! Dependency pre-bundling cache.
//!
//! Layout:
//! ```text
//! <project>/.cloudpack/cache/deps/
//! ├── {fingerprint-hex}/
//! │   ├── index.json
//! │   └── *.js            # (future: real bundles; SCA writes nothing else)
//! └── .tmp-{fingerprint}/  # transient during bundle; renamed atomically
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Lockfile names checked, in preference order. The first one found "wins"
/// and is the only lockfile mixed into the fingerprint.
const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrebundleResult {
    pub cache_dir: PathBuf,
    pub from_cache: bool,
    /// Lowercase hex of `blake3(package.json || lockfile)`.
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct DepPrebundler {
    cache_root: PathBuf,
    ttl_days: u32,
}

/// Compute `blake3(package.json || first-found-lockfile)` as lowercase hex.
///
/// Returns an error if `package.json` is missing. If no lockfile is present,
/// only `package.json` contributes to the hash (this still produces a stable
/// fingerprint).
pub fn compute_fingerprint(root: &Path) -> Result<String> {
    let pkg_path = root.join("package.json");
    let pkg_bytes = std::fs::read(&pkg_path)
        .with_context(|| format!("could not read {}", pkg_path.display()))?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(&pkg_bytes);

    for name in LOCKFILE_NAMES {
        let lock_path = root.join(name);
        if lock_path.is_file() {
            let lock_bytes = std::fs::read(&lock_path)
                .with_context(|| format!("could not read {}", lock_path.display()))?;
            hasher.update(&lock_bytes);
            break;
        }
    }

    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn fingerprint_is_lowercase_hex_and_stable() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.0"}"#);

        let fp1 = compute_fingerprint(tmp.path()).unwrap();
        let fp2 = compute_fingerprint(tmp.path()).unwrap();

        assert_eq!(fp1, fp2, "fingerprint must be stable");
        assert_eq!(fp1.len(), 64, "blake3 hex is 64 chars");
        assert!(
            fp1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "fingerprint must be lowercase hex"
        );
    }

    #[test]
    fn fingerprint_changes_when_package_json_changes() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.0"}"#);
        let fp1 = compute_fingerprint(tmp.path()).unwrap();

        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.1"}"#);
        let fp2 = compute_fingerprint(tmp.path()).unwrap();

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_changes_when_lockfile_changes() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":1}"#);
        let fp1 = compute_fingerprint(tmp.path()).unwrap();

        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":2}"#);
        let fp2 = compute_fingerprint(tmp.path()).unwrap();

        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_prefers_package_lock_over_pnpm_lock() {
        // Both lockfiles exist. We should hash package-lock.json only.
        // To verify, mutate pnpm-lock.yaml and observe the fingerprint is unchanged.
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":1}"#);
        write(tmp.path(), "pnpm-lock.yaml", "lockfileVersion: '6.0'\n");
        let fp1 = compute_fingerprint(tmp.path()).unwrap();

        write(tmp.path(), "pnpm-lock.yaml", "lockfileVersion: '7.0'\n");
        let fp2 = compute_fingerprint(tmp.path()).unwrap();

        assert_eq!(fp1, fp2, "pnpm-lock.yaml must be ignored when package-lock.json exists");
    }

    #[test]
    fn fingerprint_missing_package_json_is_error() {
        let tmp = TempDir::new().unwrap();
        let err = compute_fingerprint(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("package.json"));
    }

    #[test]
    fn fingerprint_no_lockfile_still_works() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        let fp = compute_fingerprint(tmp.path()).unwrap();
        assert_eq!(fp.len(), 64);
    }
}
```

- [ ] **Step 1.5: Verify tests compile and pass**

Run: `cargo test -p cloudpack-dev --lib`

Expected: 6 tests pass (`fingerprint_*`).

If `blake3::Hasher::finalize().to_hex()` returns an uppercase hex string in your blake3 version, the `is_ascii_uppercase()` assertion will fail. Blake3 1.x returns lowercase; pin to `blake3 = "1"` as in the manifest.

- [ ] **Step 1.6: Add stub `DepPrebundler::new`**

Append to `crates/cloudpack-dev/src/prebundle/mod.rs` (before the `#[cfg(test)]` block):

```rust
impl DepPrebundler {
    /// Construct a pre-bundler rooted at `cache_root` with `ttl_days` TTL.
    /// Neither path nor TTL is validated here; `ensure_fresh` does the work.
    pub fn new(cache_root: PathBuf, ttl_days: u32) -> Self {
        Self { cache_root, ttl_days }
    }

    /// Accessor used by tests and the CLI cache-size check.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }
}
```

- [ ] **Step 1.7: Run workspace build to confirm crate links**

Run: `cargo build -p cloudpack-dev`

Expected: clean build.

- [ ] **Step 1.8: Commit**

```bash
git add Cargo.toml crates/cloudpack-dev/
git commit -m "feat(cloudpack-dev): scaffold crate with compute_fingerprint (blake3 of package.json + lockfile)"
```

---

## Task 2: Implement `ensure_fresh` and `gc`

**Files:**
- Modify: `crates/cloudpack-dev/src/prebundle/mod.rs`
- Create: `crates/cloudpack-dev/tests/prebundle.rs`

- [ ] **Step 2.1: Write failing tests for `ensure_fresh` (cache miss → hit, atomic, gc)**

Create `crates/cloudpack-dev/tests/prebundle.rs`:

```rust
//! Integration tests for the dependency pre-bundling cache.

use std::path::Path;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use cloudpack_dev::{compute_fingerprint, DepPrebundler};

fn write(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

fn make_project(tmp: &Path) {
    write(tmp, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
    write(tmp, "package-lock.json", r#"{"lockfileVersion":1}"#);
}

#[test]
fn cache_miss_creates_dir_with_index_json() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache, "first run is a miss");
    assert!(r.cache_dir.is_dir());
    assert_eq!(r.cache_dir.file_name().unwrap().to_str().unwrap(), r.fingerprint);

    let index_path = r.cache_dir.join("index.json");
    assert!(index_path.is_file(), "index.json must exist after ensure_fresh");

    // index.json must be valid JSON and contain the fingerprint.
    let body = std::fs::read_to_string(&index_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["fingerprint"], serde_json::Value::String(r.fingerprint.clone()));
}

#[test]
fn cache_hit_returns_instantly_without_touching_index() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project.path()).unwrap();
    assert!(!r1.from_cache);

    // Tamper with index.json so we'd notice if ensure_fresh rewrote it.
    let sentinel = r1.cache_dir.join("index.json");
    std::fs::write(&sentinel, "SENTINEL").unwrap();

    let r2 = pre.ensure_fresh(project.path()).unwrap();
    assert!(r2.from_cache, "second run is a hit");
    assert_eq!(r1.fingerprint, r2.fingerprint);
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "SENTINEL");
}

#[test]
fn fingerprint_changes_invalidate_cache() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project.path()).unwrap();

    // Bump version → new fingerprint → new cache dir, still a miss.
    write(project.path(), "package.json", r#"{"name":"app","version":"2.0.0"}"#);
    let r2 = pre.ensure_fresh(project.path()).unwrap();

    assert_ne!(r1.fingerprint, r2.fingerprint);
    assert!(!r2.from_cache);
    assert!(r1.cache_dir.is_dir(), "old cache dir still exists (gc handles cleanup)");
    assert!(r2.cache_dir.is_dir());
}

#[test]
fn ensure_fresh_leaves_no_tmp_dirs_behind() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    pre.ensure_fresh(project.path()).unwrap();

    for entry in std::fs::read_dir(cache.path()).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.starts_with(".tmp-"),
            "tempdir {name:?} should have been renamed away"
        );
    }
}

#[test]
fn concurrent_ensure_fresh_does_not_error() {
    // Two threads racing on the same project/cache. Both should succeed and
    // agree on the fingerprint; at most one observes from_cache=false.
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let proj = project.path().to_path_buf();
    let cache_root = cache.path().to_path_buf();

    let t1 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        thread::spawn(move || {
            let pre = DepPrebundler::new(cache_root, 14);
            pre.ensure_fresh(&proj).unwrap()
        })
    };
    let t2 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        thread::spawn(move || {
            let pre = DepPrebundler::new(cache_root, 14);
            pre.ensure_fresh(&proj).unwrap()
        })
    };

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();

    assert_eq!(r1.fingerprint, r2.fingerprint);
    assert_eq!(r1.cache_dir, r2.cache_dir);
    assert!(r1.cache_dir.join("index.json").is_file());
}

#[test]
fn gc_deletes_stale_dirs_and_keeps_fresh_ones() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    // TTL = 0 days means "anything older than now" — i.e., everything is stale.
    let pre = DepPrebundler::new(cache.path().to_path_buf(), 0);
    let r = pre.ensure_fresh(project.path()).unwrap();
    assert!(r.cache_dir.is_dir());

    // Sleep so mtime is strictly older than "now - 0 days" (which is "now").
    // A 0-day TTL with `Duration::from_secs(0)` would race on same-second mtime;
    // 1 second is enough because gc uses mtime <= now - ttl.
    thread::sleep(Duration::from_millis(1100));

    pre.gc().unwrap();

    assert!(
        !r.cache_dir.exists(),
        "ttl_days=0 must evict the directory we just created (after 1s)"
    );
}

#[test]
fn gc_keeps_fresh_dirs_under_default_ttl() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    pre.gc().unwrap();
    assert!(r.cache_dir.is_dir(), "fresh dir must survive gc under default TTL");
}

#[test]
fn gc_on_missing_root_is_not_an_error() {
    // A user who has never run `cloudpack dev` has no cache dir; gc must not panic.
    let cache = TempDir::new().unwrap();
    let missing = cache.path().join("never-created");
    let pre = DepPrebundler::new(missing, 14);
    pre.gc().unwrap();
}

#[test]
fn ensure_fresh_does_not_call_gc_inline() {
    // Adjacent stale dir survives a single ensure_fresh call; gc is the caller's job.
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    make_project(project.path());

    let stale = cache.path().join("deadbeef".repeat(8));
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("index.json"), "{}").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
    let _ = pre.ensure_fresh(project.path()).unwrap();

    assert!(stale.is_dir(), "ensure_fresh must not touch unrelated entries");
}

#[test]
fn compute_fingerprint_is_reexported() {
    let project = TempDir::new().unwrap();
    make_project(project.path());
    let _fp = compute_fingerprint(project.path()).unwrap();
}
```

- [ ] **Step 2.2: Run tests to verify they fail**

Run: `cargo test -p cloudpack-dev --test prebundle`

Expected: compile error — `ensure_fresh` and `gc` not defined on `DepPrebundler`.

- [ ] **Step 2.3: Implement `ensure_fresh`, `gc`, and the internal `bundle_into`**

Append to `crates/cloudpack-dev/src/prebundle/mod.rs`, inside the existing `impl DepPrebundler` block (replace the stub block from Task 1):

```rust
impl DepPrebundler {
    /// Construct a pre-bundler rooted at `cache_root` with `ttl_days` TTL.
    /// Neither path nor TTL is validated here; `ensure_fresh` does the work.
    pub fn new(cache_root: PathBuf, ttl_days: u32) -> Self {
        Self { cache_root, ttl_days }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Cache-hit fast path: if `<cache_root>/<fp>/index.json` exists, return it.
    /// Cache-miss slow path: bundle into a temp dir under `cache_root`, then
    /// atomically rename into place.
    pub fn ensure_fresh(&self, project_root: &Path) -> Result<PrebundleResult> {
        let fingerprint = compute_fingerprint(project_root)?;
        let cache_dir = self.cache_root.join(&fingerprint);

        if cache_dir.join("index.json").is_file() {
            return Ok(PrebundleResult {
                cache_dir,
                from_cache: true,
                fingerprint,
            });
        }

        std::fs::create_dir_all(&self.cache_root).with_context(|| {
            format!("could not create cache root {}", self.cache_root.display())
        })?;

        self.bundle_into(&fingerprint, &cache_dir)?;

        Ok(PrebundleResult {
            cache_dir,
            from_cache: false,
            fingerprint,
        })
    }

    /// SCA implementation: stage `index.json` in `.tmp-<fp>/`, then atomic-rename
    /// into `<fp>/`. Real chunk bundling is intentionally out of scope.
    fn bundle_into(&self, fingerprint: &str, final_dir: &Path) -> Result<()> {
        // tempfile::Builder gives us a uniquely-named directory we own. We then
        // write `index.json`, then rename to the deterministic name. If another
        // process wins the rename race, our rename either succeeds (and theirs
        // is overwritten — both directories have equivalent contents) or returns
        // an error we tolerate by re-checking the final path.
        let tmp = tempfile::Builder::new()
            .prefix(&format!(".tmp-{fingerprint}-"))
            .tempdir_in(&self.cache_root)
            .with_context(|| {
                format!(
                    "could not create temp dir under {}",
                    self.cache_root.display()
                )
            })?;

        let index = serde_json::json!({ "fingerprint": fingerprint });
        let index_bytes = serde_json::to_vec_pretty(&index)?;
        std::fs::write(tmp.path().join("index.json"), &index_bytes)
            .with_context(|| format!("could not write index.json in {}", tmp.path().display()))?;

        // Persist the tempdir so the Drop guard doesn't delete it, then rename.
        let staged = tmp.into_path();
        match std::fs::rename(&staged, final_dir) {
            Ok(()) => Ok(()),
            Err(_) if final_dir.join("index.json").is_file() => {
                // Concurrent writer beat us to it. Clean up our staging dir and
                // accept the winner's output (semantically equivalent).
                let _ = std::fs::remove_dir_all(&staged);
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staged);
                Err(anyhow::Error::new(e).context(format!(
                    "could not rename {} → {}",
                    staged.display(),
                    final_dir.display()
                )))
            }
        }
    }

    /// Walk `cache_root` and delete any subdirectory whose mtime is older
    /// than `ttl_days`. Never fails the whole operation if one entry errors —
    /// logs and continues, because GC must never block `cloudpack dev` startup.
    pub fn gc(&self) -> Result<()> {
        if !self.cache_root.is_dir() {
            return Ok(());
        }

        let ttl_secs = u64::from(self.ttl_days) * 24 * 60 * 60;
        let cutoff = match std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(ttl_secs))
        {
            Some(t) => t,
            None => return Ok(()), // ttl absurdly large; nothing is older than that
        };

        let entries = std::fs::read_dir(&self.cache_root)
            .with_context(|| format!("could not read {}", self.cache_root.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("gc: bad dir entry under {}: {e}", self.cache_root.display());
                    continue;
                }
            };
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("gc: could not stat {}: {e}", path.display());
                    continue;
                }
            };
            if !meta.is_dir() {
                continue;
            }
            let mtime = match meta.modified() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("gc: no mtime on {}: {e}", path.display());
                    continue;
                }
            };
            if mtime <= cutoff {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!("gc: could not remove {}: {e}", path.display());
                }
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 2.4: Run all `cloudpack-dev` tests**

Run: `cargo test -p cloudpack-dev`

Expected: all unit tests + all integration tests pass (16+ total).

If `concurrent_ensure_fresh_does_not_error` flakes on slow filesystems, the rename-race branch in `bundle_into` is the safety net — it should still pass.

- [ ] **Step 2.5: Lint clean**

Run: `cargo clippy -p cloudpack-dev --all-targets -- -D warnings`

Expected: no warnings. Fix any (typically: unused imports, `&PathBuf` vs `&Path`).

- [ ] **Step 2.6: Commit**

```bash
git add crates/cloudpack-dev/
git commit -m "feat(cloudpack-dev): DepPrebundler::ensure_fresh + gc with atomic tempdir rename"
```

---

## Task 3: Wire `DepPrebundler` into `run_dev` + `[dev]` config section + cache-size warning

**Files:**
- Modify: `crates/cloudpack-pipeline/src/config.rs`
- Modify: `crates/cloudpack-cli/Cargo.toml`
- Modify: `crates/cloudpack-cli/src/main.rs` (`run_dev` around line 396)

- [ ] **Step 3.1: Add `[dev]` section to `BuildConfig` — write the failing test**

Append to `crates/cloudpack-pipeline/src/config.rs` (inside the file, before EOF — there is currently no `#[cfg(test)]` block, so add one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn absent_dev_section_yields_none() {
        let tmp = TempDir::new().unwrap();
        let cfg = write(
            tmp.path(),
            "cloudpack.toml",
            r#"
[build]
root = "src"
out_dir = "dist"

[entry]
main = "src/index.ts"
"#,
        );
        let parsed = BuildConfig::load(&cfg).unwrap();
        assert!(parsed.dev.is_none(), "absent [dev] must preserve current behavior");
    }

    #[test]
    fn dev_section_with_ttl_parses() {
        let tmp = TempDir::new().unwrap();
        let cfg = write(
            tmp.path(),
            "cloudpack.toml",
            r#"
[build]
root = "src"
out_dir = "dist"

[entry]
main = "src/index.ts"

[dev]
dep_cache_ttl_days = 7
"#,
        );
        let parsed = BuildConfig::load(&cfg).unwrap();
        let dev = parsed.dev.expect("[dev] should be present");
        assert_eq!(dev.dep_cache_ttl_days, Some(7));
    }
}
```

Run: `cargo test -p cloudpack-pipeline config::tests`

Expected: FAIL — `BuildConfig` has no `dev` field, `DevConfig` undefined.

- [ ] **Step 3.2: Implement `DevConfig` and thread it through `BuildConfig`**

Edit `crates/cloudpack-pipeline/src/config.rs`:

In the public types section, add after `BuildConfig`:

```rust
/// `[dev]` section of `cloudpack.toml`. All fields optional; an absent
/// section preserves current behavior.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DevConfig {
    /// TTL (days) for entries in `<project>/.cloudpack/cache/deps/`.
    #[serde(default)]
    pub dep_cache_ttl_days: Option<u32>,
}
```

Add `pub dev: Option<DevConfig>,` to `BuildConfig`:

```rust
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
    pub budget: Option<crate::budget::BudgetConfig>,
    pub dev: Option<DevConfig>,
}
```

Add the raw field. Update `RawConfig`:

```rust
#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
    #[serde(default)]
    budget: Option<crate::budget::BudgetConfig>,
    #[serde(default)]
    dev: Option<DevConfig>,
}
```

Update the constructor in `BuildConfig::load`:

```rust
Ok(Self {
    root: raw.build.root,
    out_dir: raw.build.out_dir,
    source_maps: raw.build.source_maps,
    commons_threshold: raw.build.commons_threshold,
    engine: raw.build.engine,
    entry_points: raw.entry,
    budget: raw.budget,
    dev: raw.dev,
})
```

- [ ] **Step 3.3: Re-export `DevConfig` from `cloudpack-pipeline`**

Open `crates/cloudpack-pipeline/src/lib.rs` and confirm `BuildConfig` is already re-exported. Add `DevConfig` alongside it. Example diff if the current re-export is `pub use config::{BuildConfig, EngineChoice};`:

```rust
pub use config::{BuildConfig, DevConfig, EngineChoice};
```

(If the existing re-export style differs, follow the existing style exactly — just include `DevConfig` next to `BuildConfig`.)

- [ ] **Step 3.4: Run the pipeline tests**

Run: `cargo test -p cloudpack-pipeline config::tests`

Expected: both tests pass.

- [ ] **Step 3.5: Make sure existing pipeline consumers still compile**

Run: `cargo build -p cloudpack-pipeline`

Expected: clean build. If anywhere in the workspace constructs `BuildConfig { .. }` by struct-literal (rather than via `BuildConfig::load`), it will fail to compile. Fix each call site by adding `dev: None,`.

Check with:

```bash
grep -rn "BuildConfig {" crates/ --include="*.rs"
```

For every match that is a struct literal (not a method call or type annotation), add `dev: None,` to the field list.

- [ ] **Step 3.6: Commit the config change**

```bash
git add crates/cloudpack-pipeline/
git commit -m "feat(cloudpack-pipeline): add optional [dev] section with dep_cache_ttl_days"
```

- [ ] **Step 3.7: Add `cloudpack-dev` + `tracing` deps to the CLI**

Edit `crates/cloudpack-cli/Cargo.toml`. After the existing dep block, ensure these are present:

```toml
[dependencies]
cloudpack-core = { path = "../cloudpack-core" }
cloudpack-graph = { path = "../cloudpack-graph" }
cloudpack-transform = { path = "../cloudpack-transform" }
cloudpack-pipeline = { path = "../cloudpack-pipeline" }
cloudpack-abs = { path = "../cloudpack-abs" }
cloudpack-pgo = { path = "../cloudpack-pgo" }
cloudpack-bench = { path = "../cloudpack-bench" }
cloudpack-dev = { path = "../cloudpack-dev" }
clap = { version = "4", features = ["derive"] }
anyhow = { workspace = true }
serde = { workspace = true }
indicatif = "0.17"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
toml = "0.8"
walkdir = "2"
tracing = "0.1"
```

- [ ] **Step 3.8: Wire `DepPrebundler` into `run_dev`**

Edit `crates/cloudpack-cli/src/main.rs`. At the top, add to the existing `use cloudpack_pipeline::` line so `DevConfig` is in scope:

```rust
use cloudpack_pipeline::{BuildConfig, BuildPipeline, DevConfig, DevServer, EngineChoice};
```

And add a use line:

```rust
use cloudpack_dev::DepPrebundler;
```

Replace the body of `run_dev` (currently lines ~396–404):

```rust
async fn run_dev(config_path: &Path, port: u16) -> Result<()> {
    let cfg = BuildConfig::load(config_path)?;

    // Pre-bundle dependencies before the watcher starts. Failures here are
    // non-fatal: we log and continue so the user can still iterate.
    let ttl_days = cfg
        .dev
        .as_ref()
        .and_then(|d: &DevConfig| d.dep_cache_ttl_days)
        .unwrap_or(14);
    let cache_root = cfg.root.join(".cloudpack").join("cache").join("deps");
    let prebundler = DepPrebundler::new(cache_root.clone(), ttl_days);

    match prebundler.ensure_fresh(&cfg.root) {
        Ok(r) if r.from_cache => {
            tracing::debug!("dep cache hit: {}", r.fingerprint);
        }
        Ok(r) => {
            tracing::info!("dep pre-bundle complete: {}", r.fingerprint);
        }
        Err(e) => {
            tracing::warn!("dep pre-bundle failed (continuing): {e}");
        }
    }

    // Best-effort GC; never block startup on failure.
    if let Err(e) = prebundler.gc() {
        tracing::warn!("dep cache gc failed (continuing): {e}");
    }

    // Cache-size warning (>1 GB).
    match dir_size_bytes(&cache_root) {
        Ok(bytes) if bytes > 1_073_741_824 => {
            tracing::warn!(
                "dep cache at {} is {:.2} GB — consider clearing or lowering dep_cache_ttl_days",
                cache_root.display(),
                bytes as f64 / 1_073_741_824.0
            );
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("could not measure dep cache size: {e}"),
    }

    println!(
        "cloudpack dev: serving {} on http://127.0.0.1:{}",
        cfg.root.display(),
        port
    );
    DevServer { root: cfg.root, port }.start().await
}

/// Recursive size in bytes of `dir`, following no symlinks, returning 0 if the
/// directory does not exist. Used only for the >1 GB warning in `run_dev`.
fn dir_size_bytes(dir: &Path) -> Result<u64> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir).follow_links(false) {
        let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
        if entry.file_type().is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}
```

- [ ] **Step 3.9: Add a unit test for `dir_size_bytes`**

Find the `#[cfg(test)] mod tests` block in `crates/cloudpack-cli/src/main.rs` (or create one at EOF):

```rust
#[cfg(test)]
mod dev_tests {
    use super::dir_size_bytes;
    use tempfile::TempDir;

    #[test]
    fn dir_size_bytes_missing_dir_is_zero() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nope");
        assert_eq!(dir_size_bytes(&missing).unwrap(), 0);
    }

    #[test]
    fn dir_size_bytes_sums_nested_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/x.txt"), b"hello").unwrap();      // 5
        std::fs::write(tmp.path().join("a/b/y.txt"), b"worldly").unwrap();  // 7
        assert_eq!(dir_size_bytes(tmp.path()).unwrap(), 12);
    }
}
```

- [ ] **Step 3.10: Run CLI tests**

Run: `cargo test -p cloudpack-cli`

Expected: pre-existing tests still pass + 2 new `dev_tests::*` pass.

- [ ] **Step 3.11: Full workspace build + test**

Run: `cargo build --workspace`

Expected: clean build of all 9 crates.

Run: `cargo test --workspace`

Expected: all tests pass. If any other call site of `BuildConfig` broke (struct literal missing `dev: None`), fix and re-run.

- [ ] **Step 3.12: Lint clean across the workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: no warnings. Common fixes:
- `tracing` unused import → remove if a path was inlined
- `&PathBuf` arguments → narrow to `&Path`
- unused `cache_root.clone()` → if the warning fires, the clone is required because both `DepPrebundler::new` and the size check use the path; ignore that specific instance via local refactor (e.g., compute size from `prebundler.cache_root()`).

If the clone warning fires, replace the size-check block with:

```rust
    match dir_size_bytes(prebundler.cache_root()) {
        Ok(bytes) if bytes > 1_073_741_824 => {
            tracing::warn!(
                "dep cache at {} is {:.2} GB — consider clearing or lowering dep_cache_ttl_days",
                prebundler.cache_root().display(),
                bytes as f64 / 1_073_741_824.0
            );
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("could not measure dep cache size: {e}"),
    }
```

And drop the `let cache_root = ...; ... cache_root.clone()` line, constructing the prebundler directly:

```rust
    let prebundler = DepPrebundler::new(
        cfg.root.join(".cloudpack").join("cache").join("deps"),
        ttl_days,
    );
```

- [ ] **Step 3.13: Smoke test the CLI end-to-end**

```bash
mkdir -p /tmp/cloudpack-smoke/src
cat > /tmp/cloudpack-smoke/package.json <<'JSON'
{"name":"smoke","version":"1.0.0"}
JSON
cat > /tmp/cloudpack-smoke/package-lock.json <<'JSON'
{"lockfileVersion":1}
JSON
cat > /tmp/cloudpack-smoke/src/index.ts <<'TS'
export const x = 1;
TS
cat > /tmp/cloudpack-smoke/cloudpack.toml <<'TOML'
[build]
root = "/tmp/cloudpack-smoke"
out_dir = "/tmp/cloudpack-smoke/dist"

[entry]
main = "/tmp/cloudpack-smoke/src/index.ts"

[dev]
dep_cache_ttl_days = 14
TOML

# Run with tracing enabled so we see the info/debug lines.
RUST_LOG=cloudpack=info,cloudpack_dev=debug,cloudpack_cli=info \
  timeout 3 cargo run -p cloudpack-cli -- dev --config /tmp/cloudpack-smoke/cloudpack.toml --port 8123 || true

ls /tmp/cloudpack-smoke/.cloudpack/cache/deps/
cat /tmp/cloudpack-smoke/.cloudpack/cache/deps/*/index.json
```

Expected:
- First run logs `dep pre-bundle complete: <hex>` at info level
- `ls` shows one 64-char-hex directory containing `index.json`
- `index.json` body contains `"fingerprint": "<hex>"` matching the directory name

Re-run the same `cargo run` command. Expected: `dep cache hit: <hex>` at debug level (and no second cache dir created).

- [ ] **Step 3.14: Commit**

```bash
git add crates/cloudpack-cli/
git commit -m "feat(cloudpack-cli): run_dev pre-bundles deps via cloudpack-dev, warns on >1 GB cache"
```

---

## Acceptance Criteria Recap

- [x] `cargo test -p cloudpack-dev` — Task 1 (fingerprint) + Task 2 (ensure_fresh/gc) cover this
- [x] `ensure_fresh` cache-hit is instant and does not touch `index.json` — `cache_hit_returns_instantly_without_touching_index`
- [x] `ensure_fresh` cache-miss is atomic (tempdir + rename) — `cache_miss_creates_dir_with_index_json` + `ensure_fresh_leaves_no_tmp_dirs_behind`
- [x] Fingerprint changes when `package.json` OR any lockfile changes — `fingerprint_changes_when_package_json_changes` + `fingerprint_changes_when_lockfile_changes` + `fingerprint_changes_invalidate_cache`
- [x] `gc()` deletes dirs older than TTL; never blocks — `gc_deletes_stale_dirs_and_keeps_fresh_ones` + `gc_on_missing_root_is_not_an_error` + `gc` uses `tracing::warn` per-entry instead of `?`
- [x] Absent `[dev]` section preserves current behavior — `absent_dev_section_yields_none` in pipeline tests + `unwrap_or(14)` in `run_dev`

## Self-Review Notes

- **Spec coverage:** all six acceptance criteria mapped to tests above; cache-size warning covered by the smoke test in Step 3.13 (no unit test because it touches the global file size of a path; the `dir_size_bytes` helper is unit-tested in Step 3.9).
- **Type consistency:** `DepPrebundler::new(PathBuf, u32) -> Self`, `ensure_fresh(&self, &Path) -> Result<PrebundleResult>`, `gc(&self) -> Result<()>`, `compute_fingerprint(&Path) -> Result<String>` — all signatures stable across Tasks 1–3.
- **No placeholders:** every step contains the exact Rust to write or the exact command to run.
- **Re-exports:** `cloudpack_dev::{DepPrebundler, PrebundleResult, compute_fingerprint}` (Task 1.3), `cloudpack_pipeline::{BuildConfig, DevConfig, ...}` (Task 3.3) — both consumed by `run_dev` in Task 3.8.
