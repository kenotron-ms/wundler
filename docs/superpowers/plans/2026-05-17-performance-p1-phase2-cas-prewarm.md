# Performance P1 Phase 2 — CAS Pre-warm for `node_modules` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the `DepPrebundler::bundle_into` stub with a real CAS pre-warm step that walks `node_modules/` directories under the project root and populates the global summary cache (`~/.wundler/cache/summaries/`) so the next `wundler build` / `wundler dev` analysis pass hits warm cache entries for every `.ts`/`.tsx`/`.js`/`.jsx`/`.mjs`/`.cjs` file.

**Architecture:** `DepPrebundler` already gates pre-bundling on a `package.json` + lockfile fingerprint and writes `<cache_root>/<fp>/index.json` as the "we did the work" marker. We thread `project_root` into `bundle_into`, add a `discover_node_modules` helper that returns the top-level + one-level-deep (monorepo) `node_modules/` directories, and call `wundler_core::summarizer::summarize_directory_with_stats` against each one using the globally-shared `LocalCache`. The per-run stats (total files, new entries written, cache hits) flow back through `PrebundleResult` to the CLI, which logs them on the `info!` path. Pre-warm failures are non-fatal — the CLI already logs `dep pre-bundle failed (continuing)` and that behaviour stays unchanged.

**Tech Stack:** Rust 2021, `anyhow`, `serde`, `serde_json`, `tempfile`, `tracing`, `wundler-core` (in-workspace).

**Scope Boundary:**
- **In scope:** wire `wundler-core` into `wundler-dev`; add `new_entries` / `cached_entries` to `PrebundleResult`; thread `project_root` into `bundle_into`; implement `discover_node_modules(project_root)` (depth 0 + 1, deduped); call `summarize_directory_with_stats` on each result; write extended `index.json`; surface stats in the CLI `info!` log.
- **Out of scope:** `PackageLevelCache` integration (stays as opt-in layer); deeper monorepo traversal (e.g. `packages/foo/packages/bar/node_modules`); `.d.ts` filtering (`JS_EXTENSIONS` in `summarizer/mod.rs:27` already excludes it); progress reporting; per-package parallelism (`summarize_directory_with_stats` already uses Rayon internally).

---

## File Structure

**Files modified**
- `crates/wundler-dev/Cargo.toml` — add `wundler-core` path dependency.
- `crates/wundler-dev/src/prebundle/mod.rs` — extend `PrebundleResult`, thread `project_root` into `bundle_into`, add `discover_node_modules`, replace stub body with real pre-warm logic, extend `index.json` payload, add a `cas_root` field to `DepPrebundler` so tests can inject a tempdir CAS.
- `crates/wundler-dev/tests/prebundle.rs` — extend `cache_miss_creates_dir_with_index_json` to assert new `index.json` fields and update constructor calls; add CAS pre-warm integration test.
- `crates/wundler-cli/src/main.rs` — pass a CAS root into `DepPrebundler::new`; update logging in `run_dev` to surface `new_entries` / `cached_entries`.

**Files created**
- _None._ Everything lives in the existing `prebundle/mod.rs` module so changes that move together stay together.

---

## Task 1: Wire `wundler-core` in, extend `PrebundleResult`, thread `project_root`

**Files:**
- Modify: `crates/wundler-dev/Cargo.toml`
- Modify: `crates/wundler-dev/src/prebundle/mod.rs` (lines 15-20 struct, 22-26 `DepPrebundler` fields, 53-77 `ensure_fresh`/constructor, 79-106 `bundle_into`)
- Modify: `crates/wundler-cli/src/main.rs` (lines 403-406 — `DepPrebundler::new` call site, will need a CAS root argument)
- Test: `crates/wundler-dev/tests/prebundle.rs` (lines 19-37 existing `cache_miss_creates_dir_with_index_json` — needs updated assertions for new fields)

This task is plumbing only. It introduces the new public surface area so Task 2 and Task 3 can land in small, focused commits. After this task the project still compiles, all existing tests still pass, but `bundle_into` is still the stub — it just has the new signature.

- [ ] **Step 1: Add `wundler-core` to `wundler-dev` deps**

Edit `crates/wundler-dev/Cargo.toml`. The full file after editing:

```toml
[package]
name = "wundler-dev"
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
wundler-core = { path = "../wundler-core" }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Run `cargo check -p wundler-dev` to confirm the dep resolves**

Run:
```bash
cargo check -p wundler-dev
```
Expected: builds cleanly, no errors. `wundler-core` shows up in the dep tree.

- [ ] **Step 3: Update the failing `cache_miss_creates_dir_with_index_json` test for the new `index.json` shape**

Edit `crates/wundler-dev/tests/prebundle.rs`. Replace the entire `cache_miss_creates_dir_with_index_json` test (currently lines 19-37) with:

```rust
#[test]
fn cache_miss_creates_dir_with_index_json() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert!(r.cache_dir.is_dir());
    assert_eq!(r.cache_dir.file_name().unwrap().to_str().unwrap(), r.fingerprint);

    // PrebundleResult carries pre-warm stats.
    assert_eq!(r.new_entries, 0, "no node_modules in this fixture");
    assert_eq!(r.cached_entries, 0, "no node_modules in this fixture");

    let index_path = r.cache_dir.join("index.json");
    assert!(index_path.is_file());
    let body = std::fs::read_to_string(&index_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["fingerprint"], serde_json::Value::String(r.fingerprint.clone()));
    assert_eq!(parsed["new_entries"], serde_json::Value::from(0u64));
    assert_eq!(parsed["cached_entries"], serde_json::Value::from(0u64));
    assert_eq!(parsed["total_modules"], serde_json::Value::from(0u64));
}
```

Also update **every** other call to `DepPrebundler::new(...)` in this file so the new 3-arg signature compiles. The existing 2-arg calls live at lines 25, 45, 64, 82, 104, 109, 125, 141, 151, 165. Each call follows the pattern:

```rust
let pre = DepPrebundler::new(cache.path().to_path_buf(), 14);
```

Replace each with:

```rust
let cas = TempDir::new().unwrap();
let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
```

For tests that already use the variable name `cache_root` in a thread closure (`concurrent_ensure_fresh_does_not_error`, currently lines 92-117), also clone a `cas_root`:

```rust
#[test]
fn concurrent_ensure_fresh_does_not_error() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let proj = project.path().to_path_buf();
    let cache_root = cache.path().to_path_buf();
    let cas_root = cas.path().to_path_buf();

    let t1 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        let cas_root = cas_root.clone();
        thread::spawn(move || {
            DepPrebundler::new(cache_root, cas_root, 14)
                .ensure_fresh(&proj)
                .unwrap()
        })
    };
    let t2 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        let cas_root = cas_root.clone();
        thread::spawn(move || {
            DepPrebundler::new(cache_root, cas_root, 14)
                .ensure_fresh(&proj)
                .unwrap()
        })
    };

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    assert_eq!(r1.fingerprint, r2.fingerprint);
    assert_eq!(r1.cache_dir, r2.cache_dir);
    assert!(r1.cache_dir.join("index.json").is_file());
}
```

- [ ] **Step 4: Run the tests to confirm they fail with compile errors (RED)**

Run:
```bash
cargo test -p wundler-dev --test prebundle 2>&1 | head -40
```
Expected: compilation fails with errors like `no field 'new_entries' on type 'PrebundleResult'` and `this function takes 2 arguments but 3 arguments were supplied`. This is the RED state.

- [ ] **Step 5: Extend `PrebundleResult` and thread `project_root`/`cas_root`**

Edit `crates/wundler-dev/src/prebundle/mod.rs`.

Replace the struct at lines 15-20:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrebundleResult {
    pub cache_dir: PathBuf,
    pub from_cache: bool,
    pub fingerprint: String,
    /// Number of summary entries newly written to the CAS during this run.
    /// Always 0 on `from_cache == true`.
    pub new_entries: u64,
    /// Number of summary entries that were already warm in the CAS.
    /// Always 0 on `from_cache == true`.
    pub cached_entries: u64,
}
```

Replace the struct at lines 22-26 and the constructor at lines 53-56:

```rust
#[derive(Debug, Clone)]
pub struct DepPrebundler {
    cache_root: PathBuf,
    cas_root: PathBuf,
    ttl_days: u32,
}

impl DepPrebundler {
    pub fn new(cache_root: PathBuf, cas_root: PathBuf, ttl_days: u32) -> Self {
        Self { cache_root, cas_root, ttl_days }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn cas_root(&self) -> &Path {
        &self.cas_root
    }
```

Replace `ensure_fresh` (currently lines 62-77) so it passes `project_root` into `bundle_into` and constructs the new `PrebundleResult`:

```rust
    pub fn ensure_fresh(&self, project_root: &Path) -> Result<PrebundleResult> {
        let fingerprint = compute_fingerprint(project_root)?;
        let cache_dir = self.cache_root.join(&fingerprint);

        if cache_dir.join("index.json").is_file() {
            return Ok(PrebundleResult {
                cache_dir,
                from_cache: true,
                fingerprint,
                new_entries: 0,
                cached_entries: 0,
            });
        }

        std::fs::create_dir_all(&self.cache_root).with_context(|| {
            format!("could not create cache root {}", self.cache_root.display())
        })?;

        let (new_entries, cached_entries) =
            self.bundle_into(&fingerprint, &cache_dir, project_root)?;

        Ok(PrebundleResult {
            cache_dir,
            from_cache: false,
            fingerprint,
            new_entries,
            cached_entries,
        })
    }
```

Replace `bundle_into` (currently lines 79-106). For now it remains a stub but with the new signature and return type; the pre-warm body lands in Task 3. The `total_modules` field is included in the JSON now so the test contract is stable.

```rust
    fn bundle_into(
        &self,
        fingerprint: &str,
        final_dir: &Path,
        _project_root: &Path,
    ) -> Result<(u64, u64)> {
        let tmp = tempfile::Builder::new()
            .prefix(&format!(".tmp-{fingerprint}-"))
            .tempdir_in(&self.cache_root)
            .with_context(|| {
                format!("could not create temp dir under {}", self.cache_root.display())
            })?;

        // Task 1 stub: zero stats. Task 3 replaces this with real CAS pre-warm.
        let new_entries: u64 = 0;
        let cached_entries: u64 = 0;
        let total_modules: u64 = 0;

        let index = serde_json::json!({
            "fingerprint": fingerprint,
            "new_entries": new_entries,
            "cached_entries": cached_entries,
            "total_modules": total_modules,
        });
        let index_bytes = serde_json::to_vec_pretty(&index)?;
        std::fs::write(tmp.path().join("index.json"), &index_bytes)
            .with_context(|| {
                format!("could not write index.json in {}", tmp.path().display())
            })?;

        let staged = tmp.keep();
        match std::fs::rename(&staged, final_dir) {
            Ok(()) => Ok((new_entries, cached_entries)),
            Err(_) if final_dir.join("index.json").is_file() => {
                let _ = std::fs::remove_dir_all(&staged);
                Ok((new_entries, cached_entries))
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
```

- [ ] **Step 6: Update the CLI call site so the workspace builds**

Edit `crates/wundler-cli/src/main.rs`. Replace the `DepPrebundler::new(...)` call currently at lines 403-406:

```rust
    let prebundler = wundler_dev::DepPrebundler::new(
        cfg.root.join(".wundler").join("cache").join("deps"),
        cfg.root.join(".wundler").join("cache").join("deps"),
        ttl_days,
    );
```

We pass the deps cache dir twice here as a temporary placeholder so the workspace still builds. Task 4 swaps the CAS argument for the proper global path via `LocalCache::with_default_root`. Leave the existing `match prebundler.ensure_fresh(&cfg.root) { ... }` block at lines 408-412 unchanged for now.

- [ ] **Step 7: Run the full test suite to confirm GREEN**

Run:
```bash
cargo test -p wundler-dev
cargo build --workspace
```
Expected: all `wundler-dev` tests pass (including the new assertions on `new_entries`/`cached_entries`/`total_modules`), `cargo build --workspace` succeeds.

- [ ] **Step 8: Commit**

```bash
git add crates/wundler-dev/Cargo.toml \
        crates/wundler-dev/src/prebundle/mod.rs \
        crates/wundler-dev/tests/prebundle.rs \
        crates/wundler-cli/src/main.rs
git commit -m "feat(dev): plumb project_root + CAS root through DepPrebundler

PrebundleResult now carries new_entries/cached_entries stats and
index.json carries fingerprint + new_entries + cached_entries +
total_modules. bundle_into still writes zeros — real pre-warm lands
in the next commit."
```

---

## Task 2: `discover_node_modules` helper

**Files:**
- Modify: `crates/wundler-dev/src/prebundle/mod.rs` (add a private function above `impl DepPrebundler`, expand the existing `#[cfg(test)] mod tests` block at the bottom)

This task introduces the discovery function that Task 3 will call. It does NOT yet wire it up — the function lives free-standing with its own unit tests.

The function rule:
- Always include `<project_root>/node_modules` if it is a directory.
- For each direct child `<project_root>/<child>` that is a directory (and not itself named `node_modules`), include `<child>/node_modules` if that is a directory.
- Dedupe by canonicalized path (cheap defensive measure — symlinked workspaces).
- Skip silently on I/O errors during enumeration (we should never make `wundler dev` fail because a sibling directory was unreadable).

- [ ] **Step 1: Write the failing unit tests**

Append to the `#[cfg(test)] mod tests { ... }` block at the bottom of `crates/wundler-dev/src/prebundle/mod.rs` (the block starts at line 160). Add the imports and tests below — keep the existing tests in the module untouched.

```rust
    use std::collections::HashSet;

    fn mkdir(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
    }

    #[test]
    fn discover_node_modules_finds_top_level_only() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("node_modules"));

        let found = discover_node_modules(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], tmp.path().join("node_modules"));
    }

    #[test]
    fn discover_node_modules_finds_monorepo_depth_one() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("node_modules"));
        mkdir(&tmp.path().join("packages").join("a").join("node_modules"));
        mkdir(&tmp.path().join("packages").join("b").join("node_modules"));
        // depth-2 must NOT be picked up
        mkdir(
            &tmp.path()
                .join("packages")
                .join("a")
                .join("nested")
                .join("node_modules"),
        );

        let found: HashSet<PathBuf> = discover_node_modules(tmp.path()).into_iter().collect();

        // Note: "packages/a/node_modules" sits at depth 2 from project_root,
        // because "packages" is the depth-1 child. The plan's "one level deep"
        // rule is "child of project_root has a node_modules subdir".
        // So we DO want packages/node_modules (none here) and we do NOT want
        // packages/a/node_modules from this fixture. Adjust expectations:
        assert!(found.contains(&tmp.path().join("node_modules")));
        assert!(!found.contains(
            &tmp.path()
                .join("packages")
                .join("a")
                .join("nested")
                .join("node_modules"),
        ));
        // packages/a is at depth 2, not depth 1, so its node_modules is NOT included.
        assert!(!found.contains(&tmp.path().join("packages").join("a").join("node_modules")));
    }

    #[test]
    fn discover_node_modules_finds_workspace_layout_at_depth_one() {
        // pnpm/yarn workspaces commonly do: <root>/<pkg>/node_modules
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("node_modules"));
        mkdir(&tmp.path().join("pkg-a").join("node_modules"));
        mkdir(&tmp.path().join("pkg-b").join("node_modules"));
        // a child without node_modules must not break us
        mkdir(&tmp.path().join("docs"));

        let found: HashSet<PathBuf> = discover_node_modules(tmp.path()).into_iter().collect();
        assert!(found.contains(&tmp.path().join("node_modules")));
        assert!(found.contains(&tmp.path().join("pkg-a").join("node_modules")));
        assert!(found.contains(&tmp.path().join("pkg-b").join("node_modules")));
        assert_eq!(found.len(), 3, "found = {found:?}");
    }

    #[test]
    fn discover_node_modules_ignores_files_named_node_modules() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("node_modules"), b"not a dir").unwrap();
        let found = discover_node_modules(tmp.path());
        assert!(found.is_empty(), "file (not dir) named node_modules: {found:?}");
    }

    #[test]
    fn discover_node_modules_does_not_recurse_into_node_modules() {
        // Real node_modules/ contains thousands of subdirs each with their own
        // node_modules/. We MUST NOT walk into the top-level node_modules looking
        // for nested ones.
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("node_modules"));
        mkdir(&tmp.path().join("node_modules").join("foo").join("node_modules"));

        let found = discover_node_modules(tmp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0], tmp.path().join("node_modules"));
    }

    #[test]
    fn discover_node_modules_returns_empty_when_none_present() {
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("src"));
        let found = discover_node_modules(tmp.path());
        assert!(found.is_empty());
    }

    #[test]
    fn discover_node_modules_dedupes_by_canonical_path() {
        // Defensive: if a child is a symlink to "." we should still see node_modules once.
        // We can't easily portably create a symlink in a test on every platform, so we
        // verify the simpler invariant: no duplicates in the returned vec.
        let tmp = TempDir::new().unwrap();
        mkdir(&tmp.path().join("node_modules"));
        mkdir(&tmp.path().join("pkg").join("node_modules"));

        let found = discover_node_modules(tmp.path());
        let unique: HashSet<_> = found.iter().collect();
        assert_eq!(found.len(), unique.len(), "duplicates in {found:?}");
    }
```

- [ ] **Step 2: Run the new tests to confirm they fail (RED)**

Run:
```bash
cargo test -p wundler-dev --lib prebundle::tests::discover_node_modules 2>&1 | head -30
```
Expected: compile errors `cannot find function 'discover_node_modules' in this scope`.

- [ ] **Step 3: Implement `discover_node_modules`**

Add this function to `crates/wundler-dev/src/prebundle/mod.rs`, just above `impl DepPrebundler` (i.e. after `compute_fingerprint` ends around line 51, before line 53):

```rust
/// Return the `node_modules` directories worth pre-warming for `project_root`.
///
/// Includes:
/// - `<project_root>/node_modules` if it is a directory.
/// - `<project_root>/<child>/node_modules` for each direct child of
///   `project_root` that is itself a directory (monorepo / workspace layout).
///
/// Does NOT recurse further — `packages/foo/packages/bar/node_modules` is out
/// of scope (see scope boundary in the plan).
///
/// Returned paths are deduplicated by canonical path. I/O errors during
/// enumeration are swallowed (they should never fail a `wundler dev` run);
/// at worst we pre-warm fewer directories than ideal.
pub(crate) fn discover_node_modules(project_root: &Path) -> Vec<PathBuf> {
    use std::collections::HashSet;

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    let mut push = |p: PathBuf, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>| {
        if !p.is_dir() {
            return;
        }
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if seen.insert(key) {
            out.push(p);
        }
    };

    // Depth 0: <project_root>/node_modules
    push(project_root.join("node_modules"), &mut out, &mut seen);

    // Depth 1: each direct child's node_modules (skip "node_modules" itself).
    let children = match std::fs::read_dir(project_root) {
        Ok(it) => it,
        Err(_) => return out,
    };
    for entry in children.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|s| s.to_str()) == Some("node_modules") {
            continue;
        }
        push(path.join("node_modules"), &mut out, &mut seen);
    }

    out
}
```

- [ ] **Step 4: Run the tests to confirm GREEN**

Run:
```bash
cargo test -p wundler-dev --lib prebundle::tests::discover_node_modules
```
Expected: all 7 `discover_node_modules_*` tests pass.

- [ ] **Step 5: Run the whole `wundler-dev` test surface to confirm no regression**

Run:
```bash
cargo test -p wundler-dev
```
Expected: every test passes (existing prebundle tests + new discovery tests).

- [ ] **Step 6: Commit**

```bash
git add crates/wundler-dev/src/prebundle/mod.rs
git commit -m "feat(dev): add discover_node_modules — depth 0 + 1 for monorepos

Discovers <project_root>/node_modules and <project_root>/<child>/node_modules.
Does not recurse into node_modules itself (avoids the n² real-world cost).
Canonical-path dedupe for symlinked workspaces.

Not yet wired into bundle_into."
```

---

## Task 3: Real `bundle_into` — call `summarize_directory_with_stats` on each `node_modules`

**Files:**
- Modify: `crates/wundler-dev/src/prebundle/mod.rs` — replace the stub body of `bundle_into` with the real pre-warm logic; tighten imports.
- Test: `crates/wundler-dev/tests/prebundle.rs` — add an end-to-end test that creates a fake `node_modules` with a `.ts` file and verifies (a) the returned `PrebundleResult.new_entries == 1` and (b) the global CAS now contains exactly one summary entry.

- [ ] **Step 1: Write the failing integration test**

Append to `crates/wundler-dev/tests/prebundle.rs`:

```rust
#[test]
fn ensure_fresh_prewarms_cas_for_node_modules() {
    use walkdir::WalkDir;

    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    // Fake a single dependency module under node_modules.
    let dep = project.path().join("node_modules").join("acme");
    std::fs::create_dir_all(&dep).unwrap();
    std::fs::write(dep.join("index.ts"), "export const x = 1;\n").unwrap();
    // A non-JS file must be ignored.
    std::fs::write(dep.join("README.md"), "# acme\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert_eq!(r.new_entries, 1, "should have summarized one .ts file");
    assert_eq!(r.cached_entries, 0, "first run, nothing was cached yet");

    // The CAS should now contain exactly one summary JSON file under a 2-char shard dir.
    let cas_files: Vec<_> = WalkDir::new(cas.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    assert_eq!(cas_files.len(), 1, "expected 1 CAS entry, got: {cas_files:?}");
}

#[test]
fn ensure_fresh_prewarms_workspace_packages() {
    use walkdir::WalkDir;

    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    // Top-level node_modules with one module
    let top_dep = project.path().join("node_modules").join("alpha");
    std::fs::create_dir_all(&top_dep).unwrap();
    std::fs::write(top_dep.join("a.ts"), "export const a = 1;\n").unwrap();

    // Workspace package node_modules with one module
    let pkg_dep = project
        .path()
        .join("pkg-a")
        .join("node_modules")
        .join("beta");
    std::fs::create_dir_all(&pkg_dep).unwrap();
    std::fs::write(pkg_dep.join("b.ts"), "export const b = 2;\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert_eq!(r.new_entries, 2, "both node_modules trees should have been walked");
    assert_eq!(r.cached_entries, 0);

    let cas_files: Vec<_> = WalkDir::new(cas.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    assert_eq!(cas_files.len(), 2, "expected 2 CAS entries");
}

#[test]
fn ensure_fresh_records_hits_on_warm_cas() {
    // If the CAS already has the entry (e.g. from a previous project), a *new*
    // project with the same source content should report cached_entries=1.
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();

    // Project 1 — cold CAS.
    let project1 = TempDir::new().unwrap();
    make_project(project1.path());
    let dep1 = project1.path().join("node_modules").join("shared");
    std::fs::create_dir_all(&dep1).unwrap();
    std::fs::write(dep1.join("index.ts"), "export const shared = 42;\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project1.path()).unwrap();
    assert_eq!(r1.new_entries, 1);
    assert_eq!(r1.cached_entries, 0);

    // Project 2 — different fingerprint (different package.json), same dep content.
    let project2 = TempDir::new().unwrap();
    write(
        project2.path(),
        "package.json",
        r#"{"name":"other","version":"9.9.9"}"#,
    );
    let dep2 = project2.path().join("node_modules").join("shared");
    std::fs::create_dir_all(&dep2).unwrap();
    std::fs::write(dep2.join("index.ts"), "export const shared = 42;\n").unwrap();

    let r2 = pre.ensure_fresh(project2.path()).unwrap();
    assert_ne!(r1.fingerprint, r2.fingerprint, "different project, different fp");
    assert!(!r2.from_cache, "different fingerprint, so deps cache misses");
    assert_eq!(r2.new_entries, 0, "content was already in the CAS");
    assert_eq!(r2.cached_entries, 1, "and we should have recorded the hit");
}
```

- [ ] **Step 2: Run the new tests to confirm they fail (RED)**

Run:
```bash
cargo test -p wundler-dev --test prebundle ensure_fresh_prewarms 2>&1 | head -30
cargo test -p wundler-dev --test prebundle ensure_fresh_records_hits 2>&1 | head -30
```
Expected: tests compile but fail with `assertion `left == right` failed: should have summarized one .ts file` (because `bundle_into` still writes zeros).

- [ ] **Step 3: Replace the `bundle_into` stub body with the real pre-warm**

Edit `crates/wundler-dev/src/prebundle/mod.rs`. Replace the entire `bundle_into` method (the one introduced in Task 1) with:

```rust
    fn bundle_into(
        &self,
        fingerprint: &str,
        final_dir: &Path,
        project_root: &Path,
    ) -> Result<(u64, u64)> {
        use wundler_core::cache::local::LocalCache;
        use wundler_core::summarizer::summarize_directory_with_stats;

        let tmp = tempfile::Builder::new()
            .prefix(&format!(".tmp-{fingerprint}-"))
            .tempdir_in(&self.cache_root)
            .with_context(|| {
                format!("could not create temp dir under {}", self.cache_root.display())
            })?;

        // Open the shared CAS. This is the same directory the build/dev analysis
        // step uses, so warming it here is what makes the next analysis pass fast.
        let cas = LocalCache::new(self.cas_root.clone()).with_context(|| {
            format!("could not open CAS at {}", self.cas_root.display())
        })?;

        let roots = discover_node_modules(project_root);

        let mut total_modules: u64 = 0;
        let mut new_entries: u64 = 0;
        let mut cached_entries: u64 = 0;

        for nm in &roots {
            tracing::debug!("pre-warming CAS from {}", nm.display());
            match summarize_directory_with_stats(nm, &cas) {
                Ok(result) => {
                    total_modules += result.stats.total as u64;
                    new_entries += result.stats.cache_misses as u64;
                    cached_entries += result.stats.cache_hits as u64;
                }
                Err(e) => {
                    // Non-fatal: pre-warm is best-effort.
                    tracing::warn!(
                        "pre-warm failed for {} (continuing): {e}",
                        nm.display()
                    );
                }
            }
        }

        let index = serde_json::json!({
            "fingerprint": fingerprint,
            "new_entries": new_entries,
            "cached_entries": cached_entries,
            "total_modules": total_modules,
        });
        let index_bytes = serde_json::to_vec_pretty(&index)?;
        std::fs::write(tmp.path().join("index.json"), &index_bytes)
            .with_context(|| {
                format!("could not write index.json in {}", tmp.path().display())
            })?;

        let staged = tmp.keep();
        match std::fs::rename(&staged, final_dir) {
            Ok(()) => Ok((new_entries, cached_entries)),
            Err(_) if final_dir.join("index.json").is_file() => {
                let _ = std::fs::remove_dir_all(&staged);
                Ok((new_entries, cached_entries))
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
```

- [ ] **Step 4: Run the new tests to confirm GREEN**

Run:
```bash
cargo test -p wundler-dev --test prebundle
```
Expected: all prebundle integration tests pass, including the three new pre-warm tests.

- [ ] **Step 5: Run the whole `wundler-dev` test surface**

Run:
```bash
cargo test -p wundler-dev
```
Expected: every test passes (unit tests + integration tests).

- [ ] **Step 6: Commit**

```bash
git add crates/wundler-dev/src/prebundle/mod.rs crates/wundler-dev/tests/prebundle.rs
git commit -m "feat(dev): real CAS pre-warm for node_modules in bundle_into

Walks every node_modules discovered by discover_node_modules and calls
summarize_directory_with_stats against the shared LocalCache, populating
~/.wundler/cache/summaries so the next analysis pass hits warm.

Pre-warm failures are non-fatal — logged at warn, never propagated.
Per-root stats are aggregated and returned via PrebundleResult."
```

---

## Task 4: Wire the global CAS into the CLI, surface stats in `run_dev` logging, final workspace check

**Files:**
- Modify: `crates/wundler-cli/src/main.rs` (lines ~403-412 — `DepPrebundler::new` construction and `match prebundler.ensure_fresh(...)` log block in `run_dev`)

- [ ] **Step 1: Update `DepPrebundler::new` call site to use the global CAS root**

Edit `crates/wundler-cli/src/main.rs`. Replace the temporary 3-arg construction added in Task 1 (lines 403-406) with one that resolves the shared CAS root via `LocalCache::with_default_root`. The CAS root must be the *exact* directory `LocalCache::with_default_root()` uses (`~/.wundler/cache/summaries/`), otherwise downstream analysis runs would open a different cache and see zero hits.

```rust
    // Resolve the shared summary CAS root the same way LocalCache::with_default_root does.
    // We can't easily borrow that path back out of a LocalCache, so we recompute it here.
    // Keep this in sync with wundler_core::cache::local::LocalCache::with_default_root.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let cas_root = std::path::PathBuf::from(home)
        .join(".wundler")
        .join("cache")
        .join("summaries");

    let prebundler = wundler_dev::DepPrebundler::new(
        cfg.root.join(".wundler").join("cache").join("deps"),
        cas_root,
        ttl_days,
    );
```

- [ ] **Step 2: Update the `match prebundler.ensure_fresh(...)` log block to surface stats**

Edit `crates/wundler-cli/src/main.rs`. Replace the existing match block (lines 408-412):

```rust
    match prebundler.ensure_fresh(&cfg.root) {
        Ok(r) if r.from_cache => {
            tracing::debug!("dep cache hit: {}", r.fingerprint);
        }
        Ok(r) => {
            tracing::info!(
                "dep pre-bundle complete: {} — CAS pre-warm: {} new, {} cached",
                r.fingerprint,
                r.new_entries,
                r.cached_entries,
            );
        }
        Err(e) => tracing::warn!("dep pre-bundle failed (continuing): {e}"),
    }
```

- [ ] **Step 3: Build the full workspace**

Run:
```bash
cargo build --workspace
```
Expected: clean build, no warnings beyond any pre-existing ones.

- [ ] **Step 4: Run the full workspace test suite**

Run:
```bash
cargo test --workspace
```
Expected: all tests pass. `wundler-dev`'s prebundle integration tests in particular should show:
- `cache_miss_creates_dir_with_index_json` ... ok
- `cache_hit_returns_instantly_without_touching_index` ... ok
- `fingerprint_changes_invalidate_cache` ... ok
- `ensure_fresh_leaves_no_tmp_dirs_behind` ... ok
- `concurrent_ensure_fresh_does_not_error` ... ok
- `gc_deletes_stale_dirs_and_keeps_fresh_ones` ... ok
- `gc_keeps_fresh_dirs_under_default_ttl` ... ok
- `gc_on_missing_root_is_not_an_error` ... ok
- `ensure_fresh_does_not_call_gc_inline` ... ok
- `compute_fingerprint_is_reexported` ... ok
- `ensure_fresh_prewarms_cas_for_node_modules` ... ok
- `ensure_fresh_prewarms_workspace_packages` ... ok
- `ensure_fresh_records_hits_on_warm_cas` ... ok

- [ ] **Step 5: Run clippy on the workspace**

Run:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: no warnings. If `clippy` complains about the duplicated home-directory logic in `main.rs` (e.g. `clippy::or_fun_call`), that mirrors the exact pattern already in `LocalCache::with_default_root` (crates/wundler-core/src/cache/local.rs:32-41) — keep it as-is for parity. If clippy raises an unrelated, real warning, fix it; do not blanket-allow.

- [ ] **Step 6: Manual smoke test (optional but recommended)**

Run:
```bash
mkdir -p /tmp/wundler-smoke/node_modules/acme
cat > /tmp/wundler-smoke/package.json <<'JSON'
{"name":"smoke","version":"0.0.1"}
JSON
cat > /tmp/wundler-smoke/node_modules/acme/index.ts <<'TS'
export const x: number = 1;
TS

# Run dev briefly so the pre-warm path executes. SIGINT after the log appears.
RUST_LOG=info cargo run -p wundler-cli -- dev /tmp/wundler-smoke 2>&1 | head -20
```
Expected: the log line `dep pre-bundle complete: <hex> — CAS pre-warm: 1 new, 0 cached` (or `0 new, 1 cached` if you ran it twice and `~/.wundler/cache/summaries/` was already warm). Kill with Ctrl-C.

- [ ] **Step 7: Commit**

```bash
git add crates/wundler-cli/src/main.rs
git commit -m "feat(cli): wire shared CAS into DepPrebundler, log pre-warm stats

run_dev now constructs DepPrebundler with the same ~/.wundler/cache/summaries
root that LocalCache::with_default_root() opens, so the analysis step that
follows actually benefits from the pre-warmed entries. info! log now reports
new vs cached entry counts."
```

---

## Self-Review

**1. Spec coverage**
- ✅ Add `wundler-core` dep to `wundler-dev/Cargo.toml` — Task 1, Step 1.
- ✅ Thread `project_root: &Path` through `bundle_into(fingerprint, final_dir, project_root)` — Task 1, Step 5.
- ✅ `discover_node_modules(project_root) -> Vec<PathBuf>` at depth 0 + 1, deduped, no `node_modules` recursion — Task 2, Step 3, plus tests in Step 1.
- ✅ Call `summarize_directory_with_stats` on each discovered `node_modules` — Task 3, Step 3.
- ✅ Update `index.json` format with pre-warm stats (`fingerprint`, `new_entries`, `cached_entries`, `total_modules`) — Task 1 introduces the field set (zeros), Task 3 fills it for real; the existing test at `tests/prebundle.rs:36` continues to assert `parsed["fingerprint"]` is present.
- ✅ Update `PrebundleResult` to carry pre-warm stats — Task 1, Step 5 (`new_entries`, `cached_entries`).
- ✅ Update `run_dev` logging to surface stats — Task 4, Step 2.
- ✅ Out of scope explicitly avoided: `PackageLevelCache` (untouched), depth > 1 traversal (test `discover_node_modules_finds_monorepo_depth_one` asserts the boundary), `.d.ts` filtering (delegated to `JS_EXTENSIONS` in `summarizer/mod.rs:27`), parallel pre-warm (relies on Rayon inside `summarize_directory_with_stats`).

**2. Placeholder scan**
- No "TBD" / "implement later" / "add appropriate error handling" / "write tests for the above" anywhere. Every code block contains complete code.
- No "similar to Task N" hand-waves — Task 3 repeats the full `bundle_into` body, not a diff.
- Every `expected:` line names a concrete outcome.

**3. Type consistency**
- `PrebundleResult` fields used across all four tasks: `cache_dir`, `from_cache`, `fingerprint`, `new_entries`, `cached_entries`. All references match.
- `DepPrebundler::new(cache_root: PathBuf, cas_root: PathBuf, ttl_days: u32)` — same three-arg signature in Task 1 (definition), Task 1 step 3 (test updates), and Task 4 step 1 (CLI call site).
- `bundle_into(&self, fingerprint: &str, final_dir: &Path, project_root: &Path) -> Result<(u64, u64)>` — consistent across Task 1 (stub) and Task 3 (real impl).
- `discover_node_modules(project_root: &Path) -> Vec<PathBuf>` is `pub(crate)`, called inside the same module by `bundle_into` in Task 3 — visibility matches.
- `summarize_directory_with_stats` returns `SummarizeResult { nodes, stats: SummarizeStats { total, cache_hits, cache_misses } }` (verified at `crates/wundler-core/src/summarizer/mod.rs:35-50`); Task 3 uses `result.stats.total`, `result.stats.cache_hits`, `result.stats.cache_misses` — matches.
- `LocalCache::new(root: PathBuf) -> Result<Self>` (verified at `crates/wundler-core/src/cache/local.rs:22`); Task 3 calls it with `self.cas_root.clone()` — matches.
- `index.json` JSON field names (`fingerprint`, `new_entries`, `cached_entries`, `total_modules`) are identical between Task 1 stub and Task 3 real implementation; existing test at `tests/prebundle.rs:36` is updated in Task 1 step 3 to match.
