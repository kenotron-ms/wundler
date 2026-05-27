# Scale Benchmark Foundation — Synthetic Corpus Profiler (MVP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a profile-driven synthetic-corpus generator and structural conformance verifier to `cloudpack-bench` so benchmarks can target reproducible, declarative repo shapes (Steps 1–7 of the scale benchmark foundation design).

**Architecture:** A new `BenchProfile` JSON contract wraps an existing `RepoScaleReport` plus generator hints (seed + tolerances). `corpus_gen` walks the profile’s `WorkspaceStats` and emits files using a small library of file archetypes (TypeScript module, JSON config, Markdown) padded to per-extension average byte sizes; everything is driven by a single seeded `StdRng` so two runs with identical inputs produce byte-identical trees. `corpus_verify` re-measures the corpus, compares each `ExtensionStat` field against the profile within configured tolerances, and additionally runs an SWC parse sample over `.ts` files. CLI subcommands and a CV-stability gate on `analysis-bench` round out the workflow.

**Tech Stack:**
- Rust 2021, `cloudpack-bench` crate
- `serde` / `serde_json` for the profile contract
- `rand 0.8` (`StdRng`) for deterministic generation
- `swc_core` (v65, ecma_parser/ecma_ast/common features — already used by `cloudpack-transform`) for V3 TS parse validity
- `clap 4` derive for CLI
- `tempfile`, `anyhow` (already in `cloudpack-bench`)

**Scope Boundary (MVP, Steps 1–7 only):**
- ✅ `profile.rs` types + schema version gate
- ✅ Archetypes: `ts_module` (Leaf/Intermediate/Barrel), `json_config`, `markdown`
- ✅ `corpus_gen.rs` deterministic generator
- ✅ `corpus_verify.rs` V1 structural + V3 SWC parse sample
- ✅ CLI: `generate-corpus`, `verify-corpus`
- ✅ Committed profiles: `tiny.v1.json` (20 files), `large-web-app-small.v1.json` (100 files)
- ✅ V5 CV stability gate: `--repeat N --check-cv <T>` on `analysis-bench`
- ❌ Graph-shape profiling (Steps 8–13)
- ❌ 36k-file `large-web-app.v1.json` (needs a real repo measurement)
- ❌ Archetypes for `.tsx`, `.yaml`, `.cmd`
- ❌ C3 anonymized skeleton replay, B3 stochastic block model

---

## File Structure

**Files to create:**
- `crates/cloudpack-bench/src/profile.rs` — Profile contract types + schema version constant + JSON load/save
- `crates/cloudpack-bench/src/archetypes/mod.rs` — Shared `pad_to_target` helper + module re-exports
- `crates/cloudpack-bench/src/archetypes/ts_module.rs` — Three TS variants (Leaf, Intermediate, Barrel)
- `crates/cloudpack-bench/src/archetypes/json_config.rs` — Small JSON config object generator
- `crates/cloudpack-bench/src/archetypes/markdown.rs` — H1 + paragraphs generator
- `crates/cloudpack-bench/src/corpus_gen.rs` — `generate_corpus(profile, out_dir)` + fingerprint writer
- `crates/cloudpack-bench/src/corpus_verify.rs` — `verify_corpus(profile, dir) -> ConformanceReport`
- `crates/cloudpack-bench/profiles/test/tiny.v1.json` — 20-file profile for `cargo test`
- `crates/cloudpack-bench/profiles/large-web-app-small.v1.json` — 100-file profile (< 1 s)
- `crates/cloudpack-bench/tests/corpus_e2e.rs` — End-to-end integration test (generate → verify)

**Files to modify:**
- `crates/cloudpack-bench/Cargo.toml` — Add `rand`, `swc_core` deps
- `crates/cloudpack-bench/src/lib.rs` — Add new `pub mod` declarations
- `crates/cloudpack-bench/src/main.rs` — Add `generate-corpus`, `verify-corpus` subcommands; add `--repeat` + `--check-cv` flags to `analysis-bench`

---

## Task 1: Profile Contract + Schema Version Gate

**Files:**
- Modify: `crates/cloudpack-bench/Cargo.toml`
- Create: `crates/cloudpack-bench/src/profile.rs`
- Modify: `crates/cloudpack-bench/src/lib.rs`

- [ ] **Step 1.1: Add `rand` dependency**

Open `crates/cloudpack-bench/Cargo.toml`. Under `[dependencies]`, append:

```toml
rand            = "0.8"
```

The other deps we need later (`swc_core`) will be added in Task 4 when first used; doing it now would force unused-dep warnings until then.

- [ ] **Step 1.2: Write the failing test for `BenchProfile` JSON round-trip**

Create `crates/cloudpack-bench/src/profile.rs` (initially with **only** the test module so the test fails to compile):

```rust
//! BenchProfile contract — declarative, versioned shape spec for synthetic corpora.

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> BenchProfile {
        use crate::repo_scale::{
            DirectoryStat, ExtensionStat, GitStats, ManifestStats, RepoScaleReport,
            WorkspaceStats,
        };
        use std::collections::HashMap;

        let mut ext = HashMap::new();
        ext.insert(
            "ts".to_string(),
            ExtensionStat {
                file_count: 10,
                total_bytes: 10_000,
                total_lines: 500,
                avg_file_bytes: 1_000.0,
            },
        );

        BenchProfile {
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            target: RepoScaleReport {
                git_stats: GitStats::default(),
                workspace_stats: WorkspaceStats {
                    total_files: 10,
                    total_bytes: 10_000,
                    extension_stats: ext,
                    directory_stats: vec![DirectoryStat {
                        path: "src".to_string(),
                        file_count: 10,
                    }],
                    packages: vec!["src".to_string()],
                },
                manifest_stats: ManifestStats::default(),
            },
            gen: GenHints {
                seed: 42,
                tolerances: Tolerances::default(),
            },
        }
    }

    #[test]
    fn round_trip_json() {
        let p = sample_profile();
        let json = serde_json::to_string(&p).unwrap();
        let back: BenchProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.profile_schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(back.gen.seed, 42);
        assert_eq!(back.target.workspace_stats.total_files, 10);
    }

    #[test]
    fn default_tolerances() {
        let t = Tolerances::default();
        assert!((t.file_count_pct - 0.02).abs() < f64::EPSILON);
        assert!((t.byte_count_pct - 0.05).abs() < f64::EPSILON);
        assert!((t.line_count_pct - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn schema_version_gate_accepts_current() {
        let p = sample_profile();
        assert!(check_schema_version(&p).is_ok());
    }

    #[test]
    fn schema_version_gate_rejects_future() {
        let mut p = sample_profile();
        p.profile_schema_version = PROFILE_SCHEMA_VERSION + 1;
        let err = check_schema_version(&p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("schema"), "msg was: {msg}");
    }

    #[test]
    fn check_status_serializes_snake_case() {
        let s = serde_json::to_string(&CheckStatus::Pass).unwrap();
        assert_eq!(s, "\"pass\"");
        let s = serde_json::to_string(&CheckStatus::Skipped).unwrap();
        assert_eq!(s, "\"skipped\"");
    }
}
```

- [ ] **Step 1.3: Run the test, watch it fail to compile**

```bash
cargo test -p cloudpack-bench --lib profile::tests 2>&1 | tail -20
```

Expected: many `cannot find type`/`cannot find function` errors. That’s the failing state.

- [ ] **Step 1.4: Add `pub mod profile;` to lib.rs**

Edit `crates/cloudpack-bench/src/lib.rs` — append:

```rust
pub mod archetypes;
pub mod corpus_gen;
pub mod corpus_verify;
pub mod profile;
```

(We add all four module declarations now even though only `profile.rs` exists; we’ll create empty placeholders before compiling.)

Create empty placeholders so the crate still compiles:

```bash
mkdir -p crates/cloudpack-bench/src/archetypes
cat > crates/cloudpack-bench/src/archetypes/mod.rs <<'EOF'
//! Placeholder — populated in Task 2.
EOF
cat > crates/cloudpack-bench/src/corpus_gen.rs <<'EOF'
//! Placeholder — populated in Task 3.
EOF
cat > crates/cloudpack-bench/src/corpus_verify.rs <<'EOF'
//! Placeholder — populated in Task 4.
EOF
```

- [ ] **Step 1.5: Implement the profile types**

Replace the `profile.rs` test-only stub with the full implementation, placing the test module at the bottom unchanged. Prepend above `#[cfg(test)] mod tests { ... }`:

```rust
//! BenchProfile contract — declarative, versioned shape spec for synthetic corpora.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::repo_scale::RepoScaleReport;

/// Bumped only on breaking changes to the on-disk JSON shape.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchProfile {
    pub profile_schema_version: u32,
    pub target: RepoScaleReport,
    pub gen: GenHints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenHints {
    pub seed: u64,
    #[serde(default)]
    pub tolerances: Tolerances,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tolerances {
    pub file_count_pct: f64,
    pub byte_count_pct: f64,
    pub line_count_pct: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            file_count_pct: 0.02,
            byte_count_pct: 0.05,
            line_count_pct: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub profile_schema_version: u32,
    pub checks: Vec<Check>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub target: f64,
    pub actual: f64,
    pub tolerance_pct: f64,
    pub status: CheckStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skipped,
}

/// Returns `Ok(())` iff `profile.profile_schema_version <= PROFILE_SCHEMA_VERSION`.
pub fn check_schema_version(profile: &BenchProfile) -> Result<()> {
    if profile.profile_schema_version > PROFILE_SCHEMA_VERSION {
        return Err(anyhow!(
            "profile schema version {} is newer than supported maximum {} — upgrade cloudpack-bench",
            profile.profile_schema_version,
            PROFILE_SCHEMA_VERSION
        ));
    }
    Ok(())
}

/// Load a profile from disk and validate its schema version.
pub fn load_profile(path: &Path) -> Result<BenchProfile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading profile from {}", path.display()))?;
    let profile: BenchProfile = serde_json::from_str(&text)
        .with_context(|| format!("parsing profile JSON at {}", path.display()))?;
    check_schema_version(&profile)?;
    Ok(profile)
}

/// Write a profile to disk as pretty-printed JSON.
pub fn save_profile(profile: &BenchProfile, path: &Path) -> Result<()> {
    let text = serde_json::to_string_pretty(profile)?;
    std::fs::write(path, text)
        .with_context(|| format!("writing profile to {}", path.display()))?;
    Ok(())
}
```

> **Note:** `RepoScaleReport`, `GitStats`, `ManifestStats`, `WorkspaceStats`, `ExtensionStat`, `DirectoryStat` must derive `Serialize + Deserialize + Default` for the tests to pass. If any are missing `Default`, derive it; if any are missing `Serialize/Deserialize`, derive those.

- [ ] **Step 1.6: Verify derives on `repo_scale.rs` types**

```bash
grep -nE 'derive\(' crates/cloudpack-bench/src/repo_scale.rs
```

For each of `GitStats`, `ManifestStats`, `RepoScaleReport`, `WorkspaceStats`, `ExtensionStat`, `DirectoryStat`, ensure the derive includes `Serialize, Deserialize, Default`. If `Default` is missing on any, add it:

```bash
# Example fix (apply per-struct as needed):
# Before: #[derive(Debug, Clone, Serialize, Deserialize)]
# After:  #[derive(Debug, Clone, Default, Serialize, Deserialize)]
```

- [ ] **Step 1.7: Run the profile tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib profile::tests -- --nocapture
```

Expected: 5 tests pass (`round_trip_json`, `default_tolerances`, `schema_version_gate_accepts_current`, `schema_version_gate_rejects_future`, `check_status_serializes_snake_case`).

- [ ] **Step 1.8: Compile-check the whole crate**

```bash
cargo build -p cloudpack-bench
```

Expected: clean build (the three placeholder files are empty modules, which is legal).

- [ ] **Step 1.9: Commit**

```bash
git add crates/cloudpack-bench/Cargo.toml \
        crates/cloudpack-bench/src/lib.rs \
        crates/cloudpack-bench/src/profile.rs \
        crates/cloudpack-bench/src/archetypes/mod.rs \
        crates/cloudpack-bench/src/corpus_gen.rs \
        crates/cloudpack-bench/src/corpus_verify.rs \
        crates/cloudpack-bench/src/repo_scale.rs
git commit -m "feat(bench): add BenchProfile contract + schema version gate"
```

---

## Task 2: File Archetypes

**Files:**
- Modify: `crates/cloudpack-bench/src/archetypes/mod.rs`
- Create: `crates/cloudpack-bench/src/archetypes/ts_module.rs`
- Create: `crates/cloudpack-bench/src/archetypes/json_config.rs`
- Create: `crates/cloudpack-bench/src/archetypes/markdown.rs`
- Modify: `crates/cloudpack-bench/Cargo.toml`

- [ ] **Step 2.1: Add `swc_core` dependency (used both here for tests and in Task 4)**

Edit `crates/cloudpack-bench/Cargo.toml` under `[dependencies]`:

```toml
swc_core        = { version = "65", features = ["ecma_parser", "ecma_ast", "common"] }
```

- [ ] **Step 2.2: Write failing tests for the shared `pad_to_target` helper**

Open `crates/cloudpack-bench/src/archetypes/mod.rs` and replace its contents with:

```rust
//! File content archetypes used by `corpus_gen`. Each archetype emits a UTF-8
//! string padded to *approximately* a target byte size so that the generated
//! corpus matches the profile's per-extension `avg_file_bytes`.

pub mod json_config;
pub mod markdown;
pub mod ts_module;

/// Pads `body` with a trailing block comment of filler text so the final
/// string length (in bytes) is as close as possible to `target_bytes`, while
/// never returning shorter than the original body.
///
/// The pad character is chosen so the output is syntactically valid for the
/// caller-specified comment style.
pub fn pad_to_target(body: String, target_bytes: u64, style: PadStyle) -> String {
    let current = body.len() as u64;
    if current >= target_bytes {
        return body;
    }
    let needed = (target_bytes - current) as usize;

    // Reserve room for the comment delimiters.
    let (open, close, fill) = match style {
        PadStyle::CSlash => ("/* ", " */", 'x'),
        PadStyle::Hash => ("# ", "", 'x'),
        PadStyle::HtmlComment => ("<!-- ", " -->", 'x'),
        PadStyle::JsonStringField => {
            // JSON has no comments; we extend a string field on the caller side
            // before calling pad_to_target. As a safety net, return as-is.
            return body;
        }
    };

    let delim_len = open.len() + close.len();
    if needed <= delim_len {
        return body;
    }
    let fill_len = needed - delim_len;

    let mut out = String::with_capacity(body.len() + needed);
    out.push_str(&body);
    out.push_str(open);
    for _ in 0..fill_len {
        out.push(fill);
    }
    out.push_str(close);
    out
}

#[derive(Debug, Clone, Copy)]
pub enum PadStyle {
    /// `/* ... */`
    CSlash,
    /// `# ...` (line comment, no close)
    Hash,
    /// `<!-- ... -->`
    HtmlComment,
    /// JSON has no comment syntax — caller must pad inline; this style is a no-op.
    JsonStringField,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_to_target_extends_short_body() {
        let body = "let x = 1;\n".to_string();
        let out = pad_to_target(body.clone(), 200, PadStyle::CSlash);
        // Should be within a few bytes of 200 (delimiters add a tiny constant).
        let diff = (out.len() as i64 - 200).abs();
        assert!(diff <= 1, "len={}, diff={}", out.len(), diff);
        assert!(out.starts_with(&body));
    }

    #[test]
    fn pad_to_target_does_not_shrink() {
        let body = "x".repeat(500);
        let out = pad_to_target(body.clone(), 100, PadStyle::CSlash);
        assert_eq!(out, body);
    }

    #[test]
    fn pad_to_target_small_diff_is_noop() {
        // If needed <= delim_len we don't pad (would over/under-shoot heavily).
        let body = "ab".to_string();
        let out = pad_to_target(body.clone(), 5, PadStyle::CSlash); // delim_len=6
        assert_eq!(out, body);
    }
}
```

- [ ] **Step 2.3: Run the helper tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib archetypes::tests
```

Expected: 3 tests pass.

- [ ] **Step 2.4: Write failing tests for `ts_module::generate`**

Create `crates/cloudpack-bench/src/archetypes/ts_module.rs` with **tests only first**:

```rust
//! TypeScript module archetype with three variants:
//!   - Leaf:        no imports, pure value/function exports
//!   - Intermediate: 1–3 imports, re-exports + functions
//!   - Barrel:      only `export * from "./X";` lines

#[cfg(test)]
mod tests {
    use super::*;
    use swc_core::common::{sync::Lrc, FileName, SourceMap};
    use swc_core::ecma::parser::{lexer::Lexer, Parser, StringInput, Syntax, TsConfig};

    fn parses_as_ts(source: &str) -> bool {
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(FileName::Anon, source.to_string());
        let lexer = Lexer::new(
            Syntax::Typescript(TsConfig::default()),
            Default::default(),
            StringInput::from(&*fm),
            None,
        );
        let mut p = Parser::new_from(lexer);
        p.parse_module().is_ok()
    }

    #[test]
    fn leaf_variant_parses() {
        let src = generate(Variant::Leaf, 0, 200);
        assert!(parses_as_ts(&src), "Leaf did not parse:\n{src}");
    }

    #[test]
    fn intermediate_variant_parses() {
        let src = generate(Variant::Intermediate, 1, 400);
        assert!(parses_as_ts(&src), "Intermediate did not parse:\n{src}");
    }

    #[test]
    fn barrel_variant_parses() {
        let src = generate(Variant::Barrel, 2, 250);
        assert!(parses_as_ts(&src), "Barrel did not parse:\n{src}");
    }

    #[test]
    fn output_is_near_target_size() {
        for target in [120u64, 300, 800, 2_000] {
            let src = generate(Variant::Leaf, 7, target);
            let diff = (src.len() as i64 - target as i64).abs();
            // Allow up to 16 bytes of slack (delimiter rounding).
            assert!(
                diff <= 16,
                "target={target}, actual={}, diff={diff}",
                src.len()
            );
        }
    }

    #[test]
    fn deterministic_for_same_seed() {
        let a = generate(Variant::Intermediate, 99, 500);
        let b = generate(Variant::Intermediate, 99, 500);
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 2.5: Run, expect compile failure (no `generate`, no `Variant`)**

```bash
cargo test -p cloudpack-bench --lib archetypes::ts_module 2>&1 | tail -10
```

- [ ] **Step 2.6: Implement `ts_module`**

Prepend above the `#[cfg(test)]` block:

```rust
use crate::archetypes::{pad_to_target, PadStyle};

#[derive(Debug, Clone, Copy)]
pub enum Variant {
    Leaf,
    Intermediate,
    Barrel,
}

/// Generate TypeScript source roughly `target_bytes` long. Deterministic in
/// `(variant, variant_seed, target_bytes)`.
pub fn generate(variant: Variant, variant_seed: u64, target_bytes: u64) -> String {
    let body = match variant {
        Variant::Leaf => leaf(variant_seed),
        Variant::Intermediate => intermediate(variant_seed),
        Variant::Barrel => barrel(variant_seed),
    };
    pad_to_target(body, target_bytes, PadStyle::CSlash)
}

fn leaf(seed: u64) -> String {
    format!(
        "export const VALUE_{seed}: number = {seed};\n\
         export function compute_{seed}(input: number): number {{\n\
         \treturn input + {seed};\n\
         }}\n"
    )
}

fn intermediate(seed: u64) -> String {
    // Deterministic 1–3 "imports" of synthetic siblings. Path is relative and
    // intentionally points to other generated files; the file may not exist
    // at parse time, but TS parsing is purely syntactic so this is fine.
    let n_imports = 1 + (seed % 3);
    let mut s = String::new();
    for i in 0..n_imports {
        s.push_str(&format!(
            "import {{ compute_{i} }} from \"./sibling_{i}\";\n"
        ));
    }
    s.push_str(&format!(
        "export function pipeline_{seed}(x: number): number {{\n\
         \tlet v = x;\n"
    ));
    for i in 0..n_imports {
        s.push_str(&format!("\tv = compute_{i}(v);\n"));
    }
    s.push_str(&format!("\treturn v + {seed};\n}}\n"));
    s
}

fn barrel(seed: u64) -> String {
    let n = 2 + (seed % 4);
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("export * from \"./mod_{i}\";\n"));
    }
    s
}
```

- [ ] **Step 2.7: Run ts_module tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib archetypes::ts_module
```

Expected: 5 tests pass.

- [ ] **Step 2.8: Write failing tests for `json_config::generate`**

Create `crates/cloudpack-bench/src/archetypes/json_config.rs`:

```rust
//! JSON config archetype — emits a small object whose `filler` string field is
//! grown to absorb the target padding (since JSON has no comments).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_as_json() {
        let s = generate(0, 300);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v.is_object());
    }

    #[test]
    fn near_target_size() {
        for target in [150u64, 500, 1_500] {
            let s = generate(3, target);
            let diff = (s.len() as i64 - target as i64).abs();
            assert!(diff <= 8, "target={target}, actual={}", s.len());
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(generate(42, 400), generate(42, 400));
    }
}
```

- [ ] **Step 2.9: Implement `json_config`**

Prepend:

```rust
/// Generates a JSON object string sized to roughly `target_bytes`. The pad is
/// applied by extending a `"filler": "xxxx..."` field.
pub fn generate(variant_seed: u64, target_bytes: u64) -> String {
    // Compose the fixed scaffold first, then size the filler string to land
    // close to the target.
    let head = format!(
        "{{\n  \"name\": \"pkg_{seed}\",\n  \"version\": \"0.0.{seed}\",\n  \"filler\": \"",
        seed = variant_seed
    );
    let tail = "\"\n}\n";
    let scaffold_len = (head.len() + tail.len()) as u64;
    let fill_len = target_bytes.saturating_sub(scaffold_len) as usize;

    let mut out = String::with_capacity(target_bytes as usize + 16);
    out.push_str(&head);
    for _ in 0..fill_len {
        out.push('x');
    }
    out.push_str(tail);
    out
}
```

- [ ] **Step 2.10: Run json_config tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib archetypes::json_config
```

- [ ] **Step 2.11: Write failing tests for `markdown::generate`**

Create `crates/cloudpack-bench/src/archetypes/markdown.rs`:

```rust
//! Markdown archetype — H1 followed by paragraphs of filler text.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_h1() {
        let s = generate(0, 200);
        assert!(s.starts_with("# "));
    }

    #[test]
    fn near_target_size() {
        for target in [100u64, 400, 1_200] {
            let s = generate(1, target);
            let diff = (s.len() as i64 - target as i64).abs();
            assert!(diff <= 8, "target={target}, actual={}", s.len());
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(generate(9, 500), generate(9, 500));
    }
}
```

- [ ] **Step 2.12: Implement `markdown`**

Prepend:

```rust
/// Generates a Markdown document sized to roughly `target_bytes`.
pub fn generate(variant_seed: u64, target_bytes: u64) -> String {
    let head = format!("# Document {variant_seed}\n\n");
    let tail = "\n";
    let scaffold_len = (head.len() + tail.len()) as u64;
    let fill_len = target_bytes.saturating_sub(scaffold_len) as usize;

    let mut out = String::with_capacity(target_bytes as usize + 16);
    out.push_str(&head);
    for _ in 0..fill_len {
        out.push('x');
    }
    out.push_str(tail);
    out
}
```

- [ ] **Step 2.13: Run all archetype tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib archetypes
```

Expected: 11 tests pass total (3 mod + 5 ts_module + 3 json_config + 3 markdown — note `near_target_size`/`deterministic` exist in both json_config and markdown, plus `has_h1` and `parses_as_json`).

- [ ] **Step 2.14: Commit**

```bash
git add crates/cloudpack-bench/Cargo.toml crates/cloudpack-bench/src/archetypes
git commit -m "feat(bench): add TS/JSON/Markdown file archetypes with deterministic padding"
```

---

## Task 3: Corpus Generator

**Files:**
- Modify: `crates/cloudpack-bench/src/corpus_gen.rs`

- [ ] **Step 3.1: Sketch the API and write a failing integration-style test**

Replace `crates/cloudpack-bench/src/corpus_gen.rs` with **tests-only first**:

```rust
//! Deterministic synthetic corpus generator. Walks the `BenchProfile`'s
//! `WorkspaceStats` and emits a tree of files matching the per-extension
//! counts and average byte sizes.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{BenchProfile, GenHints, Tolerances, PROFILE_SCHEMA_VERSION};
    use crate::repo_scale::{
        DirectoryStat, ExtensionStat, GitStats, ManifestStats, RepoScaleReport, WorkspaceStats,
    };
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    fn tiny_profile() -> BenchProfile {
        // 20 files total: 10 .ts, 5 .json, 5 .md across 2 dirs ("src", "docs").
        let mut ext = HashMap::new();
        ext.insert(
            "ts".to_string(),
            ExtensionStat {
                file_count: 10,
                total_bytes: 4_000,
                total_lines: 200,
                avg_file_bytes: 400.0,
            },
        );
        ext.insert(
            "json".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_000,
                total_lines: 25,
                avg_file_bytes: 200.0,
            },
        );
        ext.insert(
            "md".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_500,
                total_lines: 25,
                avg_file_bytes: 300.0,
            },
        );

        BenchProfile {
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            target: RepoScaleReport {
                git_stats: GitStats::default(),
                workspace_stats: WorkspaceStats {
                    total_files: 20,
                    total_bytes: 6_500,
                    extension_stats: ext,
                    directory_stats: vec![
                        DirectoryStat { path: "src".to_string(), file_count: 15 },
                        DirectoryStat { path: "docs".to_string(), file_count: 5 },
                    ],
                    packages: vec!["src".to_string()],
                },
                manifest_stats: ManifestStats::default(),
            },
            gen: GenHints {
                seed: 12345,
                tolerances: Tolerances::default(),
            },
        }
    }

    fn count_files(dir: &PathBuf, ext: &str) -> u64 {
        let mut n = 0u64;
        for entry in walkdir_min(dir) {
            if entry.extension().and_then(|s| s.to_str()) == Some(ext) {
                n += 1;
            }
        }
        n
    }

    fn walkdir_min(root: &PathBuf) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    #[test]
    fn generates_expected_file_counts() {
        let dir = tempfile::tempdir().unwrap();
        let profile = tiny_profile();
        generate_corpus(&profile, dir.path()).unwrap();

        let root: PathBuf = dir.path().to_path_buf();
        assert_eq!(count_files(&root, "ts"), 10);
        assert_eq!(count_files(&root, "json"), 5);
        assert_eq!(count_files(&root, "md"), 5);
    }

    #[test]
    fn writes_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        generate_corpus(&tiny_profile(), dir.path()).unwrap();
        let fp = dir.path().join(".cloudpack-bench-fingerprint.json");
        assert!(fp.exists());
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&fp).unwrap()).unwrap();
        assert_eq!(v["profile_schema_version"], PROFILE_SCHEMA_VERSION);
        assert_eq!(v["seed"], 12345);
    }

    #[test]
    fn deterministic_same_seed_identical_bytes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, a.path()).unwrap();
        generate_corpus(&p, b.path()).unwrap();

        let hash_a = hash_dir(a.path());
        let hash_b = hash_dir(b.path());
        assert_eq!(hash_a, hash_b, "byte-identical output expected");
    }

    #[test]
    fn refuses_future_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = tiny_profile();
        p.profile_schema_version = PROFILE_SCHEMA_VERSION + 1;
        let err = generate_corpus(&p, dir.path()).unwrap_err();
        assert!(format!("{err:?}").contains("schema"));
    }

    fn hash_dir(root: &std::path::Path) -> String {
        // Stable hash: sort path strings, then sha256 (path || content).
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                    if rel == ".cloudpack-bench-fingerprint.json" {
                        continue; // exclude fingerprint from determinism check
                    }
                    map.insert(rel, fs::read(&p).unwrap());
                }
            }
        }
        // Build a flat digest without pulling sha2 just for tests — use DefaultHasher.
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (k, v) in &map {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        format!("{:x}", h.finish())
    }
}
```

- [ ] **Step 3.2: Run, expect compile failure**

```bash
cargo test -p cloudpack-bench --lib corpus_gen::tests 2>&1 | tail -10
```

- [ ] **Step 3.3: Implement `generate_corpus`**

Prepend above the `#[cfg(test)]` block:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

use crate::archetypes::{json_config, markdown, ts_module};
use crate::profile::{check_schema_version, BenchProfile, PROFILE_SCHEMA_VERSION};
use crate::repo_scale::{DirectoryStat, ExtensionStat};

const FINGERPRINT_FILENAME: &str = ".cloudpack-bench-fingerprint.json";

/// Generate a synthetic corpus at `out_dir` matching `profile`'s shape.
/// Fully deterministic in `(profile, out_dir-independence)`: two invocations
/// with the same profile produce byte-identical file contents.
pub fn generate_corpus(profile: &BenchProfile, out_dir: &Path) -> Result<()> {
    check_schema_version(profile)?;
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating out_dir {}", out_dir.display()))?;

    // Build a stable-ordered list of extensions for determinism.
    let mut exts: Vec<(&String, &ExtensionStat)> =
        profile.target.workspace_stats.extension_stats.iter().collect();
    exts.sort_by(|a, b| a.0.cmp(b.0));

    // Ensure at least one directory exists; default to "src" if profile has none.
    let dirs: Vec<DirectoryStat> = if profile.target.workspace_stats.directory_stats.is_empty() {
        vec![DirectoryStat {
            path: "src".to_string(),
            file_count: profile.target.workspace_stats.total_files,
        }]
    } else {
        profile.target.workspace_stats.directory_stats.clone()
    };

    let mut rng = StdRng::seed_from_u64(profile.gen.seed);
    let mut global_index: u64 = 0;

    for (ext, stat) in exts {
        let n = stat.file_count;
        if n == 0 {
            continue;
        }
        let avg_bytes = stat.avg_file_bytes.max(1.0) as u64;

        // Allocate files to directories proportionally to each dir's file_count.
        let allocations = allocate_files_to_dirs(&dirs, n);

        for (dir_path, count_in_dir) in allocations {
            let dir_full = out_dir.join(&dir_path);
            std::fs::create_dir_all(&dir_full)
                .with_context(|| format!("creating dir {}", dir_full.display()))?;

            for i in 0..count_in_dir {
                let seed_for_file: u64 = rng.gen();
                let filename = format!("file_{global_index:06}.{ext}");
                let path = dir_full.join(&filename);

                let content = render_file_for_ext(ext, seed_for_file, avg_bytes);
                std::fs::write(&path, content)
                    .with_context(|| format!("writing {}", path.display()))?;

                global_index += 1;
                let _ = i;
            }
        }
    }

    write_fingerprint(profile, out_dir)?;
    Ok(())
}

fn render_file_for_ext(ext: &str, seed: u64, target_bytes: u64) -> String {
    match ext {
        "ts" => {
            // Pick a variant based on seed for some diversity.
            let variant = match seed % 5 {
                0 => ts_module::Variant::Barrel,
                1 | 2 => ts_module::Variant::Intermediate,
                _ => ts_module::Variant::Leaf,
            };
            ts_module::generate(variant, seed, target_bytes)
        }
        "json" => json_config::generate(seed, target_bytes),
        "md" => markdown::generate(seed, target_bytes),
        other => {
            // Unknown extension: emit a plain-text placeholder so the file
            // still contributes to byte/line counts. Logged in fingerprint.
            let head = format!("// archetype-fallback for .{other} seed={seed}\n");
            crate::archetypes::pad_to_target(head, target_bytes, crate::archetypes::PadStyle::CSlash)
        }
    }
}

/// Apportions `total` files across `dirs` weighted by each dir's `file_count`,
/// using a deterministic largest-remainder method so totals match exactly.
fn allocate_files_to_dirs(dirs: &[DirectoryStat], total: u64) -> Vec<(String, u64)> {
    let weight_sum: u64 = dirs.iter().map(|d| d.file_count.max(1)).sum();
    let mut alloc: Vec<(String, u64, f64)> = dirs
        .iter()
        .map(|d| {
            let w = d.file_count.max(1) as f64 / weight_sum as f64;
            let exact = w * total as f64;
            (d.path.clone(), exact.floor() as u64, exact - exact.floor())
        })
        .collect();

    let assigned: u64 = alloc.iter().map(|(_, n, _)| *n).sum();
    let mut remaining = total.saturating_sub(assigned);
    // Distribute remaining to the largest fractional remainders (ties broken by name).
    let mut order: Vec<usize> = (0..alloc.len()).collect();
    order.sort_by(|&a, &b| {
        alloc[b]
            .2
            .partial_cmp(&alloc[a].2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| alloc[a].0.cmp(&alloc[b].0))
    });
    for &idx in &order {
        if remaining == 0 {
            break;
        }
        alloc[idx].1 += 1;
        remaining -= 1;
    }

    alloc.into_iter().map(|(p, n, _)| (p, n)).collect()
}

#[derive(Serialize)]
struct Fingerprint<'a> {
    profile_schema_version: u32,
    seed: u64,
    generator_version: &'static str,
    profile_target_total_files: u64,
    notes: &'a str,
}

fn write_fingerprint(profile: &BenchProfile, out_dir: &Path) -> Result<()> {
    let fp = Fingerprint {
        profile_schema_version: PROFILE_SCHEMA_VERSION,
        seed: profile.gen.seed,
        generator_version: env!("CARGO_PKG_VERSION"),
        profile_target_total_files: profile.target.workspace_stats.total_files,
        notes: "generated by cloudpack-bench corpus_gen",
    };
    let text = serde_json::to_string_pretty(&fp)?;
    std::fs::write(out_dir.join(FINGERPRINT_FILENAME), text)?;
    Ok(())
}
```

- [ ] **Step 3.4: Run the corpus_gen tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib corpus_gen::tests -- --nocapture
```

Expected: 4 tests pass.

- [ ] **Step 3.5: Compile-check whole crate**

```bash
cargo build -p cloudpack-bench
```

- [ ] **Step 3.6: Commit**

```bash
git add crates/cloudpack-bench/src/corpus_gen.rs
git commit -m "feat(bench): deterministic corpus generator (generate_corpus)"
```

---

## Task 4: Corpus Verifier (V1 Structural + V3 SWC Parse Sample)

**Files:**
- Modify: `crates/cloudpack-bench/src/corpus_verify.rs`

- [ ] **Step 4.1: Write failing tests**

Replace `crates/cloudpack-bench/src/corpus_verify.rs`:

```rust
//! Conformance verifier: re-measures a corpus directory and checks that each
//! per-extension stat is within tolerance of the originating `BenchProfile`.
//! Also runs an SWC parse sample over `.ts` files (V3 syntactic validity).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus_gen::generate_corpus;
    use crate::profile::{BenchProfile, CheckStatus, GenHints, Tolerances, PROFILE_SCHEMA_VERSION};
    use crate::repo_scale::{
        DirectoryStat, ExtensionStat, GitStats, ManifestStats, RepoScaleReport, WorkspaceStats,
    };
    use std::collections::HashMap;
    use std::fs;

    fn tiny_profile() -> BenchProfile {
        // Line counts chosen to match newlines actually emitted by archetypes
        // (Markdown=3/file, JSON=5/file, TS=avg ~5/file with high variant variance).
        let mut ext = HashMap::new();
        ext.insert(
            "ts".to_string(),
            ExtensionStat {
                file_count: 10,
                total_bytes: 4_000,
                total_lines: 50,
                avg_file_bytes: 400.0,
            },
        );
        ext.insert(
            "json".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_000,
                total_lines: 25,
                avg_file_bytes: 200.0,
            },
        );
        ext.insert(
            "md".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_500,
                total_lines: 15,
                avg_file_bytes: 300.0,
            },
        );

        BenchProfile {
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            target: RepoScaleReport {
                git_stats: GitStats::default(),
                workspace_stats: WorkspaceStats {
                    total_files: 20,
                    total_bytes: 6_500,
                    extension_stats: ext,
                    directory_stats: vec![
                        DirectoryStat { path: "src".into(), file_count: 15 },
                        DirectoryStat { path: "docs".into(), file_count: 5 },
                    ],
                    packages: vec!["src".into()],
                },
                manifest_stats: ManifestStats::default(),
            },
            gen: GenHints {
                seed: 7,
                // line_count_pct loosened: TS variant selection (Leaf/Intermediate/Barrel)
                // produces 2..=10 newlines per file, so total line variance is high.
                tolerances: Tolerances {
                    file_count_pct: 0.02,
                    byte_count_pct: 0.05,
                    line_count_pct: 0.30,
                },
            },
        }
    }

    #[test]
    fn freshly_generated_corpus_passes() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();
        let report = verify_corpus(&p, dir.path()).unwrap();
        assert!(report.passed, "checks: {:#?}", report.checks);
        // Every check should be Pass or Skipped, none Fail.
        for c in &report.checks {
            assert_ne!(c.status, CheckStatus::Fail, "{c:?}");
        }
    }

    #[test]
    fn corrupted_corpus_fails_file_count_check() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();

        // Delete most .ts files so file_count is way under tolerance.
        let mut deleted = 0;
        for entry in fs::read_dir(dir.path().join("src")).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ts") && deleted < 8 {
                fs::remove_file(&path).unwrap();
                deleted += 1;
            }
        }

        let report = verify_corpus(&p, dir.path()).unwrap();
        assert!(!report.passed);
        assert!(report.checks.iter().any(|c| {
            c.name.contains("ts") && c.name.contains("file_count") && c.status == CheckStatus::Fail
        }));
    }

    #[test]
    fn ts_parse_sample_passes_on_clean_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();
        let report = verify_corpus(&p, dir.path()).unwrap();
        let parse_check = report
            .checks
            .iter()
            .find(|c| c.name == "ts_parse_sample")
            .expect("expected ts_parse_sample check");
        assert_eq!(parse_check.status, CheckStatus::Pass);
    }

    #[test]
    fn ts_parse_sample_fails_when_file_is_broken_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();

        // Overwrite one .ts file with invalid syntax.
        for entry in fs::read_dir(dir.path().join("src")).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ts") {
                fs::write(&path, "this is (((( not valid TS @@@@").unwrap();
                break;
            }
        }

        let report = verify_corpus(&p, dir.path()).unwrap();
        let parse_check = report
            .checks
            .iter()
            .find(|c| c.name == "ts_parse_sample")
            .expect("expected ts_parse_sample check");
        assert_eq!(parse_check.status, CheckStatus::Fail);
        assert!(!report.passed);
    }
}
```

- [ ] **Step 4.2: Run, expect compile failure**

```bash
cargo test -p cloudpack-bench --lib corpus_verify::tests 2>&1 | tail -10
```

- [ ] **Step 4.3: Implement `verify_corpus`**

Prepend above the `#[cfg(test)]` block:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use swc_core::common::{sync::Lrc, FileName, SourceMap};
use swc_core::ecma::parser::{lexer::Lexer, Parser, StringInput, Syntax, TsConfig};

use crate::profile::{
    check_schema_version, BenchProfile, Check, CheckStatus, ConformanceReport,
    PROFILE_SCHEMA_VERSION,
};
use crate::repo_scale::ExtensionStat;

/// Verify that `corpus_dir` conforms to `profile` within configured tolerances.
pub fn verify_corpus(profile: &BenchProfile, corpus_dir: &Path) -> Result<ConformanceReport> {
    check_schema_version(profile)?;
    let measured = measure_corpus(corpus_dir)
        .with_context(|| format!("measuring corpus at {}", corpus_dir.display()))?;

    let mut checks: Vec<Check> = Vec::new();
    let tol = &profile.gen.tolerances;

    // V1: per-extension structural checks (file_count, total_bytes, total_lines).
    for (ext, target) in &profile.target.workspace_stats.extension_stats {
        let actual = measured
            .get(ext)
            .cloned()
            .unwrap_or_else(ExtensionStat::default);

        checks.push(within_tol(
            format!("ext[{ext}].file_count"),
            target.file_count as f64,
            actual.file_count as f64,
            tol.file_count_pct,
        ));
        checks.push(within_tol(
            format!("ext[{ext}].total_bytes"),
            target.total_bytes as f64,
            actual.total_bytes as f64,
            tol.byte_count_pct,
        ));
        checks.push(within_tol(
            format!("ext[{ext}].total_lines"),
            target.total_lines as f64,
            actual.total_lines as f64,
            tol.line_count_pct,
        ));
    }

    // V3: SWC parse sample on .ts files.
    checks.push(ts_parse_sample(profile, corpus_dir)?);

    let passed = checks.iter().all(|c| c.status != CheckStatus::Fail);
    Ok(ConformanceReport {
        profile_schema_version: PROFILE_SCHEMA_VERSION,
        checks,
        passed,
    })
}

fn within_tol(name: String, target: f64, actual: f64, pct: f64) -> Check {
    if target == 0.0 {
        return Check {
            name,
            target,
            actual,
            tolerance_pct: pct,
            status: if actual == 0.0 {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
        };
    }
    let diff = (actual - target).abs() / target;
    let status = if diff <= pct {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Check {
        name,
        target,
        actual,
        tolerance_pct: pct,
        status,
    }
}

/// Lightweight directory walker that aggregates per-extension stats.
fn measure_corpus(root: &Path) -> Result<HashMap<String, ExtensionStat>> {
    let mut out: HashMap<String, ExtensionStat> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if file_name == ".cloudpack-bench-fingerprint.json" {
                continue;
            }
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };

            let bytes = std::fs::read(&path)?;
            let lines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
            let len = bytes.len() as u64;

            let entry = out.entry(ext.to_string()).or_insert_with(ExtensionStat::default);
            entry.file_count += 1;
            entry.total_bytes += len;
            entry.total_lines += lines;
            entry.avg_file_bytes = entry.total_bytes as f64 / entry.file_count as f64;
        }
    }
    Ok(out)
}

/// Collect all .ts paths, sample 1% (with floor 1, and 100% if total <= 20),
/// then parse each with SWC. Returns a Check with status Pass/Fail/Skipped.
fn ts_parse_sample(profile: &BenchProfile, root: &Path) -> Result<Check> {
    let mut ts_files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("ts") {
                ts_files.push(p);
            }
        }
    }
    // Stable order so RNG sampling is deterministic.
    ts_files.sort();

    if ts_files.is_empty() {
        return Ok(Check {
            name: "ts_parse_sample".into(),
            target: 0.0,
            actual: 0.0,
            tolerance_pct: 0.0,
            status: CheckStatus::Skipped,
        });
    }

    let sample_size = if ts_files.len() <= 20 {
        ts_files.len()
    } else {
        ((ts_files.len() as f64) * 0.01).ceil() as usize
    };

    let mut rng = StdRng::seed_from_u64(profile.gen.seed ^ 0xA5A5_A5A5_A5A5_A5A5);
    let mut indices: Vec<usize> = (0..ts_files.len()).collect();
    // Fisher-Yates partial shuffle of length sample_size.
    for i in 0..sample_size {
        let j = i + rng.gen_range(0..(ts_files.len() - i));
        indices.swap(i, j);
    }

    let mut ok = 0u64;
    let mut total = 0u64;
    for &idx in indices.iter().take(sample_size) {
        total += 1;
        let src = std::fs::read_to_string(&ts_files[idx])?;
        if parse_ts(&src) {
            ok += 1;
        }
    }

    let status = if ok == total {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Ok(Check {
        name: "ts_parse_sample".into(),
        target: total as f64,
        actual: ok as f64,
        tolerance_pct: 0.0,
        status,
    })
}

fn parse_ts(source: &str) -> bool {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(FileName::Anon, source.to_string());
    let lexer = Lexer::new(
        Syntax::Typescript(TsConfig::default()),
        Default::default(),
        StringInput::from(&*fm),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    parser.parse_module().is_ok()
}
```

- [ ] **Step 4.4: Run verifier tests — expect PASS**

```bash
cargo test -p cloudpack-bench --lib corpus_verify::tests -- --nocapture
```

Expected: 4 tests pass.

- [ ] **Step 4.5: Run the full library suite to ensure nothing regressed**

```bash
cargo test -p cloudpack-bench --lib
```

- [ ] **Step 4.6: Commit**

```bash
git add crates/cloudpack-bench/src/corpus_verify.rs
git commit -m "feat(bench): corpus verifier (V1 structural + V3 SWC parse sample)"
```

---

## Task 5: CLI Wiring, Committed Profiles, CV Stability Gate, E2E

**Files:**
- Modify: `crates/cloudpack-bench/src/main.rs`
- Create: `crates/cloudpack-bench/profiles/test/tiny.v1.json`
- Create: `crates/cloudpack-bench/profiles/large-web-app-small.v1.json`
- Create: `crates/cloudpack-bench/tests/corpus_e2e.rs`

- [ ] **Step 5.1: Inspect the existing CLI to identify the subcommand enum**

```bash
sed -n '1,80p' crates/cloudpack-bench/src/main.rs
```

Note the location of the `#[derive(Subcommand)]` enum (call it `Cmd`) and the `match` block in `main()`. You will add two new variants and one set of fields, plus extend the existing `AnalysisBench` variant with `--repeat` and `--check-cv`.

- [ ] **Step 5.2: Add the `GenerateCorpus` and `VerifyCorpus` subcommands**

In `crates/cloudpack-bench/src/main.rs`, inside the subcommand enum, add:

```rust
    /// Generate a synthetic corpus that matches a BenchProfile.
    GenerateCorpus {
        /// Path to a BenchProfile JSON file.
        #[arg(long)]
        profile: std::path::PathBuf,
        /// Output directory (will be created; existing contents not erased).
        #[arg(long)]
        out: std::path::PathBuf,
    },

    /// Verify that a generated corpus conforms to its BenchProfile.
    VerifyCorpus {
        /// Path to the corpus directory.
        #[arg(long)]
        corpus: std::path::PathBuf,
        /// Path to the BenchProfile JSON file used to generate it.
        #[arg(long)]
        profile: std::path::PathBuf,
        /// Emit the full ConformanceReport JSON to stdout (otherwise: human summary).
        #[arg(long)]
        json: bool,
    },
```

Then, in the `match` block in `main()`, add handlers:

```rust
        Cmd::GenerateCorpus { profile, out } => {
            let p = cloudpack_bench::profile::load_profile(&profile)?;
            cloudpack_bench::corpus_gen::generate_corpus(&p, &out)?;
            eprintln!(
                "generated corpus at {} (seed={}, target_files={})",
                out.display(),
                p.gen.seed,
                p.target.workspace_stats.total_files
            );
            Ok(())
        }
        Cmd::VerifyCorpus { corpus, profile, json } => {
            let p = cloudpack_bench::profile::load_profile(&profile)?;
            let report = cloudpack_bench::corpus_verify::verify_corpus(&p, &corpus)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let fails: Vec<_> = report
                    .checks
                    .iter()
                    .filter(|c| c.status == cloudpack_bench::profile::CheckStatus::Fail)
                    .collect();
                eprintln!(
                    "verify: {} checks, passed={} (fails: {})",
                    report.checks.len(),
                    report.passed,
                    fails.len()
                );
                for c in &fails {
                    eprintln!(
                        "  FAIL {}: target={}, actual={}, tol={}%",
                        c.name,
                        c.target,
                        c.actual,
                        c.tolerance_pct * 100.0
                    );
                }
            }
            if !report.passed {
                std::process::exit(1);
            }
            Ok(())
        }
```

- [ ] **Step 5.3: Add `--repeat N --check-cv <T>` to the existing `AnalysisBench` subcommand**

Locate the existing `AnalysisBench { ... }` variant in the `Cmd` enum. Add two new fields:

```rust
        /// Repeat the cold-build measurement N times and compute the coefficient
        /// of variation (stddev/mean). Defaults to 1 (no repetition).
        #[arg(long, default_value_t = 1)]
        repeat: u32,

        /// If set, exit non-zero when CV >= this threshold (e.g. 0.02 = 2%).
        #[arg(long)]
        check_cv: Option<f64>,
```

In the handler for `AnalysisBench`, after running `analysis_bench::run` (or whatever the existing call is), add this CV gate block. The exact integration depends on the existing handler — the pattern is:

```rust
        Cmd::AnalysisBench { repo, scales, repeat, check_cv /* + existing fields */ } => {
            // Run the existing benchmark `repeat` times, collecting cold wall-time
            // per scale. We use the largest scale as the stability target.
            let mut runs: Vec<Vec<cloudpack_bench::analysis_bench::AnalysisBenchResult>> = Vec::new();
            for _ in 0..repeat.max(1) {
                runs.push(cloudpack_bench::analysis_bench::run(&repo, &scales)?);
            }

            // Aggregate the cold wall-time of the LAST (largest) scale.
            let cold_times_ms: Vec<f64> = runs
                .iter()
                .filter_map(|r| r.last().map(|res| res.cold_wall_ms as f64))
                .collect();

            // Print existing per-run results as before:
            for (i, r) in runs.iter().enumerate() {
                println!("=== run {} ===", i + 1);
                for res in r {
                    println!("{res:?}");
                }
            }

            if let Some(threshold) = check_cv {
                let cv = coefficient_of_variation(&cold_times_ms);
                eprintln!(
                    "CV(cold_wall_ms) over {} runs at largest scale = {:.4} (threshold {:.4})",
                    cold_times_ms.len(),
                    cv,
                    threshold
                );
                if cv >= threshold {
                    eprintln!("CV gate FAILED: variance too high");
                    std::process::exit(2);
                }
            }
            Ok(())
        }
```

> **NOTE:** Adapt to whichever field name `analysis_bench::AnalysisBenchResult` actually uses for cold wall time. Check with:
>
> ```bash
> grep -n 'struct AnalysisBenchResult' -A 12 crates/cloudpack-bench/src/analysis_bench.rs
> ```
>
> If the field is named differently (e.g. `cold_wall_time_ms`, `cold_ms`, `cold_total_ms`), substitute that name in `res.cold_wall_ms` above.

Add the helper `coefficient_of_variation` near the bottom of `main.rs`:

```rust
fn coefficient_of_variation(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() / mean
}

#[cfg(test)]
mod cv_tests {
    use super::coefficient_of_variation;

    #[test]
    fn zero_for_single_sample() {
        assert_eq!(coefficient_of_variation(&[42.0]), 0.0);
    }

    #[test]
    fn zero_for_constant_samples() {
        assert_eq!(coefficient_of_variation(&[10.0, 10.0, 10.0]), 0.0);
    }

    #[test]
    fn positive_for_varying_samples() {
        let cv = coefficient_of_variation(&[10.0, 12.0, 8.0]);
        assert!(cv > 0.0 && cv < 1.0);
    }
}
```

- [ ] **Step 5.4: Compile check**

```bash
cargo build -p cloudpack-bench
```

- [ ] **Step 5.5: Verify CLI help works**

```bash
cargo run -p cloudpack-bench --quiet -- generate-corpus --help
cargo run -p cloudpack-bench --quiet -- verify-corpus --help
cargo run -p cloudpack-bench --quiet -- analysis-bench --help
```

Expected: each prints a help block listing the new flags.

- [ ] **Step 5.6: Create `tiny.v1.json` profile (20 files)**

```bash
mkdir -p crates/cloudpack-bench/profiles/test
```

Write `crates/cloudpack-bench/profiles/test/tiny.v1.json`. Note the line totals are calibrated to what the archetypes actually emit (md=3/file, json=5/file, ts~5/file with variant-dependent spread), and `line_count_pct` is set to 0.30 so the TS variant variance doesn't trip the check:

```json
{
  "profile_schema_version": 1,
  "target": {
    "git_stats": {},
    "workspace_stats": {
      "total_files": 20,
      "total_bytes": 6500,
      "extension_stats": {
        "ts": {
          "file_count": 10,
          "total_bytes": 4000,
          "total_lines": 50,
          "avg_file_bytes": 400.0
        },
        "json": {
          "file_count": 5,
          "total_bytes": 1000,
          "total_lines": 25,
          "avg_file_bytes": 200.0
        },
        "md": {
          "file_count": 5,
          "total_bytes": 1500,
          "total_lines": 15,
          "avg_file_bytes": 300.0
        }
      },
      "directory_stats": [
        { "path": "src", "file_count": 15 },
        { "path": "docs", "file_count": 5 }
      ],
      "packages": ["src"]
    },
    "manifest_stats": {}
  },
  "gen": {
    "seed": 12345,
    "tolerances": {
      "file_count_pct": 0.02,
      "byte_count_pct": 0.05,
      "line_count_pct": 0.30
    }
  }
}
```

> **Note:** The `git_stats` and `manifest_stats` objects are `{}` — this requires those structs to derive `Default` and to have all-optional/default fields with `#[serde(default)]` where needed. If `cargo run -- generate-corpus --profile profiles/test/tiny.v1.json --out /tmp/x` fails to deserialize, open `repo_scale.rs` and ensure `GitStats` and `ManifestStats` either have only fields with `#[serde(default)]` or use `#[serde(default)]` at the struct level.

- [ ] **Step 5.7: Smoke-test the CLI with the tiny profile**

```bash
rm -rf /tmp/cloudpack-tiny-corpus
cargo run -p cloudpack-bench --quiet -- generate-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
    --out /tmp/cloudpack-tiny-corpus
echo "exit=$?"

cargo run -p cloudpack-bench --quiet -- verify-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
    --corpus /tmp/cloudpack-tiny-corpus
echo "exit=$?"
```

Expected: both exit 0; the generated corpus contains 20 files plus `.cloudpack-bench-fingerprint.json`.

- [ ] **Step 5.8: Create `large-web-app-small.v1.json` (100 files, < 1 s)**

Write `crates/cloudpack-bench/profiles/large-web-app-small.v1.json`. Line totals are calibrated to archetype output (md=3/file, json=5/file, ts~5/file) — `avg_file_bytes` controls byte size via padding (which doesn't add newlines), so line counts scale with file count, not byte count:

```json
{
  "profile_schema_version": 1,
  "target": {
    "git_stats": {},
    "workspace_stats": {
      "total_files": 100,
      "total_bytes": 120000,
      "extension_stats": {
        "ts": {
          "file_count": 70,
          "total_bytes": 84000,
          "total_lines": 350,
          "avg_file_bytes": 1200.0
        },
        "json": {
          "file_count": 15,
          "total_bytes": 12000,
          "total_lines": 75,
          "avg_file_bytes": 800.0
        },
        "md": {
          "file_count": 15,
          "total_bytes": 24000,
          "total_lines": 45,
          "avg_file_bytes": 1600.0
        }
      },
      "directory_stats": [
        { "path": "apps/web/src", "file_count": 50 },
        { "path": "packages/lib/src", "file_count": 30 },
        { "path": "docs", "file_count": 15 },
        { "path": ".", "file_count": 5 }
      ],
      "packages": ["apps/web", "packages/lib"]
    },
    "manifest_stats": {}
  },
  "gen": {
    "seed": 987654321,
    "tolerances": {
      "file_count_pct": 0.02,
      "byte_count_pct": 0.05,
      "line_count_pct": 0.30
    }
  }
}
```

- [ ] **Step 5.9: Smoke-test the small-web-app profile and time it**

```bash
rm -rf /tmp/cloudpack-small-corpus
time cargo run -p cloudpack-bench --release --quiet -- generate-corpus \
    --profile crates/cloudpack-bench/profiles/large-web-app-small.v1.json \
    --out /tmp/cloudpack-small-corpus
cargo run -p cloudpack-bench --release --quiet -- verify-corpus \
    --profile crates/cloudpack-bench/profiles/large-web-app-small.v1.json \
    --corpus /tmp/cloudpack-small-corpus
```

Expected: total wall time < 1 s (excluding compile); verify exits 0.

- [ ] **Step 5.10: Write the end-to-end integration test**

Create `crates/cloudpack-bench/tests/corpus_e2e.rs`:

```rust
//! End-to-end: load the committed tiny profile from disk, generate a corpus
//! into a tempdir, verify, and check determinism across two runs.

use std::path::PathBuf;

use cloudpack_bench::corpus_gen::generate_corpus;
use cloudpack_bench::corpus_verify::verify_corpus;
use cloudpack_bench::profile::{load_profile, CheckStatus};

fn tiny_profile_path() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("profiles/test/tiny.v1.json")
}

#[test]
fn tiny_profile_generates_and_verifies() {
    let profile = load_profile(&tiny_profile_path()).expect("load tiny profile");

    let dir = tempfile::tempdir().unwrap();
    generate_corpus(&profile, dir.path()).expect("generate");

    let report = verify_corpus(&profile, dir.path()).expect("verify");
    assert!(report.passed, "report: {report:#?}");
    assert!(report
        .checks
        .iter()
        .all(|c| c.status != CheckStatus::Fail));
}

#[test]
fn tiny_profile_generation_is_deterministic() {
    let profile = load_profile(&tiny_profile_path()).expect("load");

    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    generate_corpus(&profile, a.path()).unwrap();
    generate_corpus(&profile, b.path()).unwrap();

    fn read_all_sorted(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|s| s.to_str())
                    != Some(".cloudpack-bench-fingerprint.json")
                {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                    out.push((rel, std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort_by(|x, y| x.0.cmp(&y.0));
        out
    }

    assert_eq!(read_all_sorted(a.path()), read_all_sorted(b.path()));
}
```

- [ ] **Step 5.11: Run the full crate test suite**

```bash
cargo test -p cloudpack-bench
```

Expected: all tests (unit + e2e) pass.

- [ ] **Step 5.12: Run clippy with the project's standard flags**

```bash
cargo clippy -p cloudpack-bench --all-targets -- -D warnings
```

Fix any warnings. Common ones to expect: unused imports if a helper went away; `needless_borrow` on `&PathBuf::clone()`. Treat all warnings as errors.

- [ ] **Step 5.13: Verify acceptance criteria one-by-one**

Run each as a shell check; all should print `OK`:

```bash
# AC1: cargo test passes
cargo test -p cloudpack-bench >/dev/null 2>&1 && echo "AC1 OK"

# AC2: generate-corpus exits 0
rm -rf /tmp/ac-corpus
cargo run -q -p cloudpack-bench -- generate-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
    --out /tmp/ac-corpus >/dev/null 2>&1 && echo "AC2 OK"

# AC3: verify-corpus exits 0
cargo run -q -p cloudpack-bench -- verify-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
    --corpus /tmp/ac-corpus >/dev/null 2>&1 && echo "AC3 OK"

# AC4: byte-identical output for same seed (excluding fingerprint)
rm -rf /tmp/ac-a /tmp/ac-b
cargo run -q -p cloudpack-bench -- generate-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json --out /tmp/ac-a >/dev/null
cargo run -q -p cloudpack-bench -- generate-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json --out /tmp/ac-b >/dev/null
HA=$(find /tmp/ac-a -type f ! -name '.cloudpack-bench-fingerprint.json' | sort \
     | xargs -I{} sh -c 'printf "%s " "${1#/tmp/ac-a}"; cat "$1"' _ {} | sha256sum)
HB=$(find /tmp/ac-b -type f ! -name '.cloudpack-bench-fingerprint.json' | sort \
     | xargs -I{} sh -c 'printf "%s " "${1#/tmp/ac-b}"; cat "$1"' _ {} | sha256sum)
[ "$HA" = "$HB" ] && echo "AC4 OK"

# AC5: ts_parse_sample is Pass in tiny verify report
cargo run -q -p cloudpack-bench -- verify-corpus \
    --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
    --corpus /tmp/ac-corpus --json \
  | grep -q '"name": "ts_parse_sample"' \
  && cargo run -q -p cloudpack-bench -- verify-corpus \
       --profile crates/cloudpack-bench/profiles/test/tiny.v1.json \
       --corpus /tmp/ac-corpus --json \
     | python3 -c 'import sys,json; r=json.load(sys.stdin); \
       c=[c for c in r["checks"] if c["name"]=="ts_parse_sample"][0]; \
       sys.exit(0 if c["status"]=="pass" else 1)' \
  && echo "AC5 OK"

# AC6: schema version gate refuses future schema
python3 -c '
import json, pathlib
p = json.load(open("crates/cloudpack-bench/profiles/test/tiny.v1.json"))
p["profile_schema_version"] = 999
pathlib.Path("/tmp/future.json").write_text(json.dumps(p))
'
cargo run -q -p cloudpack-bench -- generate-corpus \
    --profile /tmp/future.json --out /tmp/future-corpus 2>/dev/null
[ "$?" -ne 0 ] && echo "AC6 OK"
```

All six should print `OK`. If any fail, debug and fix before committing.

- [ ] **Step 5.14: Commit**

```bash
git add crates/cloudpack-bench/src/main.rs \
        crates/cloudpack-bench/profiles \
        crates/cloudpack-bench/tests/corpus_e2e.rs
git commit -m "feat(bench): CLI subcommands, committed profiles, CV stability gate, e2e tests"
```

---

## Final Verification

After Task 5, run the full repo check:

```bash
cargo build --workspace
cargo test -p cloudpack-bench
cargo clippy -p cloudpack-bench --all-targets -- -D warnings
```

All three must succeed with zero warnings on the `cloudpack-bench` crate.

---

## Acceptance Criteria Map

| # | Criterion | Verified by |
|---|---|---|
| AC1 | `cargo test -p cloudpack-bench` passes | Step 5.11, Step 5.13 / AC1 |
| AC2 | `generate-corpus` exits 0 on tiny profile | Step 5.7, Step 5.13 / AC2 |
| AC3 | `verify-corpus` exits 0 on tiny profile | Step 5.7, Step 5.13 / AC3 |
| AC4 | V2: same seed → byte-identical output | Step 3.1 (`deterministic_same_seed_identical_bytes`), Step 5.10 (`tiny_profile_generation_is_deterministic`), Step 5.13 / AC4 |
| AC5 | V3: all .ts in tiny profile parse with SWC | Step 4.1 (`ts_parse_sample_passes_on_clean_corpus`), Step 5.13 / AC5 |
| AC6 | Schema version gate refuses future versions | Step 1.2 (`schema_version_gate_rejects_future`), Step 3.1 (`refuses_future_schema_version`), Step 5.13 / AC6 |
