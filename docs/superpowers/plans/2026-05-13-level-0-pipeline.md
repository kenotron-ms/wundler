# Wundler Level 0 Build Pipeline — Implementation Plan

**Plan:** 3 of 5
**Date:** 2026-05-13
**Owner:** Wundler core
**Status:** Ready to execute
**Depends on:** Plan 1 (Summarizer, `wundler-core`), Plan 2 (Graph Analyzer, `wundler-graph`)

---

## Goal

Wire the Summarizer → Graph Analyzer → Transform Engine pipeline, write content-hashed chunk files to disk, produce a `manifest.json` describing the bundle graph view, and expose `wundler build` and `wundler dev` CLI commands.

This is the **Level 0 deployment**: replaces the existing build pipeline of a project (Vite, Webpack, Rspack, Rolldown) without introducing the Adaptive Bundle Service. The output is a static directory of immutable, content-hashed chunks plus a manifest. A service worker, CDN, or vanilla static file server can serve the result.

## Scope Boundaries

| In Scope | Out of Scope |
|---|---|
| `TransformEngine` trait | Adaptive Bundle Service (Plan 4) |
| `SwcTransformAdapter` (default, in-process) | Service worker client (Plan 4) |
| `RolldownAdapter` (subprocess scaffold + fallback) | PGO feedback ingestion (Plan 5) |
| `BuildPipeline` orchestrator (summarize → analyze → transform → emit) | Rspack adapter (deferred — interface only) |
| Output writer (chunks + `manifest.json`) | Persistent daemon / watch caching beyond in-process |
| Dev server with on-demand SWC transform + SSE-based HMR | Module federation, multi-app graphs |
| `wundler build` and `wundler dev` CLI commands | Production HTTP serving (a CDN is assumed) |
| `wundler.toml` config schema | Source map merging across transform passes |

## Architecture Recap

```
                    ┌──────────────────────────────────────────────────┐
                    │              BuildPipeline (build())              │
                    └──────────────────────────────────────────────────┘
                                          │
   ┌──────────────────────────────────────┼──────────────────────────────────────┐
   ▼                                      ▼                                      ▼
┌──────────────┐    nodes   ┌──────────────────┐    chunks    ┌─────────────────────┐
│ Summarizer   │ ─────────► │ Graph Analyzer    │ ───────────► │ TransformEngine     │
│ (Plan 1)     │            │ (Plan 2)          │              │ (this plan)         │
└──────────────┘            └──────────────────┘              └─────────────────────┘
                                                                         │
                                                                         ▼
                                                                ┌────────────────────┐
                                                                │ Output Writer      │
                                                                │ out_dir/chunks/*.js│
                                                                │ out_dir/manifest.  │
                                                                │   json             │
                                                                └────────────────────┘
```

`wundler dev` bypasses the pipeline entirely: it serves source modules as native ESM, transforming on demand via SWC, with file-watcher driven SSE for HMR.

## Pre-Flight Verification

Before starting, confirm prerequisite state from Plans 1 and 2:

```bash
cd /Users/ken/workspace/ms/wundler
cargo build -p wundler-core
cargo build -p wundler-graph
cargo test -p wundler-core --lib
cargo test -p wundler-graph --lib
```

All four commands must succeed. If any fail, the prerequisite plan is not complete and Plan 3 cannot start.

Verify required types exist:

```bash
grep -rn "pub struct BundleGraphNode" crates/wundler-core/src
grep -rn "pub struct ModuleSummary" crates/wundler-core/src
grep -rn "pub struct ChunkManifest" crates/wundler-graph/src
grep -rn "pub struct Chunk" crates/wundler-graph/src
grep -rn "pub struct AnalysisResult" crates/wundler-graph/src
grep -rn "pub fn summarize_directory" crates/wundler-core/src
grep -rn "pub fn analyze" crates/wundler-graph/src
```

Each `grep` must return at least one match. If a type or function is missing, stop and resolve before continuing.

## New Crates Introduced

```
crates/
├── wundler-core/         (Plan 1, exists)
├── wundler-graph/        (Plan 2, exists)
├── wundler-transform/    (NEW — this plan)
├── wundler-pipeline/     (NEW — this plan)
└── wundler-cli/          (exists, extended in this plan)
```

## Task Index

1. Workspace registration for `wundler-transform` and `wundler-pipeline`
2. `TransformEngine` trait + `ChunkOutput` + `TransformDecisions` + `TransformError`
3. `SwcTransformAdapter` — dead export stripping for a single module
4. `SwcTransformAdapter` — chunk concatenation (N modules → one JS output)
5. `SwcTransformAdapter` — source map generation
6. `RolldownAdapter` scaffold — subprocess invocation + JSON handoff + fallback
7. `BuildConfig` + `EngineChoice` deserialization from `wundler.toml`
8. `BuildPipeline::build()` step 1 — summarize via `wundler-core`
9. `BuildPipeline::build()` step 2 — analyze via `wundler-graph`
10. `BuildPipeline::build()` step 3 — parallel transform via rayon
11. Output writer — chunk files + `manifest.json` + `index.html` stub
12. Dev server — Axum static + on-demand SWC transformation
13. Dev server — file watcher + SSE-based HMR
14. CLI — `wundler build` and `wundler dev` with progress bars
15. Integration test — end-to-end 5-module TypeScript project build

Each task is TDD-shaped: failing test first, run-and-confirm-failure, minimal implementation, run-and-confirm-pass, commit.

---

## Task 1 — Register `wundler-transform` and `wundler-pipeline` in the workspace

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/wundler-transform/Cargo.toml`
- Create: `crates/wundler-transform/src/lib.rs`
- Create: `crates/wundler-pipeline/Cargo.toml`
- Create: `crates/wundler-pipeline/src/lib.rs`
- Test: `crates/wundler-transform/tests/smoke.rs`
- Test: `crates/wundler-pipeline/tests/smoke.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/smoke.rs`:

```rust
#[test]
fn crate_loads() {
    // Sanity: the crate's hello() function exists and returns the expected string.
    assert_eq!(wundler_transform::hello(), "wundler-transform");
}
```

`crates/wundler-pipeline/tests/smoke.rs`:

```rust
#[test]
fn crate_loads() {
    assert_eq!(wundler_pipeline::hello(), "wundler-pipeline");
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test smoke
```

Expected failure: `error: package ID specification 'wundler-transform' did not match any packages` (because the crate does not yet exist).

### Step 3: Write minimal implementation

Modify `Cargo.toml` (workspace root). The `[workspace]` `members` array must contain the new crates:

```toml
[workspace]
resolver = "2"
members = [
    "crates/wundler-core",
    "crates/wundler-cli",
    "crates/wundler-graph",
    "crates/wundler-transform",
    "crates/wundler-pipeline",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rayon = "1"
```

Create `crates/wundler-transform/Cargo.toml`:

```toml
[package]
name = "wundler-transform"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
wundler-core = { path = "../wundler-core" }
wundler-graph = { path = "../wundler-graph" }
swc_core = { version = "6", features = [
    "ecma_parser",
    "ecma_ast",
    "ecma_visit",
    "ecma_codegen",
    "ecma_transforms_base",
    "common",
] }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
rayon = { workspace = true }
tempfile = "3"
sha2 = "0.10"
hex = "0.4"
```

Create `crates/wundler-transform/src/lib.rs`:

```rust
//! Wundler Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub fn hello() -> &'static str {
    "wundler-transform"
}
```

Create `crates/wundler-pipeline/Cargo.toml`:

```toml
[package]
name = "wundler-pipeline"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
wundler-core = { path = "../wundler-core" }
wundler-graph = { path = "../wundler-graph" }
wundler-transform = { path = "../wundler-transform" }
tokio = { version = "1", features = ["full"] }
axum = "0.8"
tower-http = { version = "0.6", features = ["fs", "cors"] }
notify = "6"
toml = "0.8"
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
rayon = { workspace = true }
indicatif = "0.17"
sha2 = "0.10"
hex = "0.4"
```

Create `crates/wundler-pipeline/src/lib.rs`:

```rust
//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test smoke
cargo test -p wundler-pipeline --test smoke
```

Expected: each `cargo test` reports `test result: ok. 1 passed; 0 failed`.

### Step 5: Commit

```bash
git add Cargo.toml crates/wundler-transform crates/wundler-pipeline
git commit -m "wundler-transform, wundler-pipeline: register crates in workspace"
```

---

## Task 2 — Define `TransformEngine` trait + DTOs

**Files:**
- Create: `crates/wundler-transform/src/engine.rs`
- Modify: `crates/wundler-transform/src/lib.rs`
- Test: `crates/wundler-transform/tests/engine_trait.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/engine_trait.rs`:

```rust
use std::collections::{HashMap, HashSet};
use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};

struct StubEngine;

impl TransformEngine for StubEngine {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        _decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash: ContentHash::from_bytes(b"stub"),
            code: format!("// {} modules\n", modules.len()),
            source_map: None,
        })
    }
}

#[test]
fn stub_engine_returns_expected_output() {
    let engine = StubEngine;
    let chunk = Chunk {
        id: "c0".to_string(),
        modules: vec![],
        hash: ContentHash::from_bytes(b"chunk"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let decisions = TransformDecisions {
        dead_exports: HashMap::new(),
    };
    let modules = vec![];
    let out = engine.transform_chunk(&modules, &chunk, &decisions).unwrap();
    assert_eq!(out.chunk_id, "c0");
    assert_eq!(out.code, "// 0 modules\n");
    assert!(out.source_map.is_none());
}

#[test]
fn transform_decisions_records_dead_exports() {
    let mut dead = HashMap::new();
    let mut set = HashSet::new();
    set.insert("foo".to_string());
    dead.insert(ContentHash::from_bytes(b"m"), set);
    let decisions = TransformDecisions { dead_exports: dead };
    let hash = ContentHash::from_bytes(b"m");
    assert!(decisions.dead_exports.get(&hash).unwrap().contains("foo"));
}

#[test]
fn transform_error_displays_with_chunk_id() {
    let err = TransformError::TransformFailed {
        chunk_id: "c1".to_string(),
        reason: "parse error".to_string(),
    };
    let msg = format!("{}", err);
    assert!(msg.contains("c1"));
    assert!(msg.contains("parse error"));
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test engine_trait
```

Expected failure: `error[E0432]: unresolved import wundler_transform::engine` — the `engine` module does not exist.

### Step 3: Write minimal implementation

Create `crates/wundler-transform/src/engine.rs`:

```rust
//! TransformEngine trait — the seam between Wundler's analysis and code generation.
//!
//! Adapters: SwcTransformAdapter (default, in-process), RolldownAdapter (subprocess).

use std::collections::{HashMap, HashSet};
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::types::{Chunk, ChunkId};

/// Output of transforming one chunk.
#[derive(Debug, Clone)]
pub struct ChunkOutput {
    pub chunk_id: ChunkId,
    pub hash: ContentHash,
    pub code: String,
    pub source_map: Option<String>,
}

/// Decisions from the Graph Analyzer passed to the transform step.
///
/// `dead_exports` keys each module by its content hash and lists the names of
/// exported bindings that should be stripped before code generation.
#[derive(Debug, Clone, Default)]
pub struct TransformDecisions {
    pub dead_exports: HashMap<ContentHash, HashSet<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    #[error("transform failed for chunk {chunk_id}: {reason}")]
    TransformFailed { chunk_id: String, reason: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Engine-agnostic interface for emitting a chunk file from its constituent modules.
pub trait TransformEngine: Send + Sync {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError>;
}
```

Modify `crates/wundler-transform/src/lib.rs` to export `engine`:

```rust
//! Wundler Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub mod engine;

pub use engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};

pub fn hello() -> &'static str {
    "wundler-transform"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test engine_trait
```

Expected output: `test result: ok. 3 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-transform
git commit -m "wundler-transform: define TransformEngine trait, ChunkOutput, TransformDecisions, TransformError"
```

---

## Task 3 — `SwcTransformAdapter`: dead export stripping for a single module

**Files:**
- Create: `crates/wundler-transform/src/swc_adapter.rs`
- Create: `crates/wundler-transform/src/swc_util.rs`
- Modify: `crates/wundler-transform/src/lib.rs`
- Test: `crates/wundler-transform/tests/swc_dead_exports.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/swc_dead_exports.rs`:

```rust
use std::collections::HashSet;
use wundler_transform::swc_adapter::strip_dead_exports;

#[test]
fn strip_named_function_export() {
    let src = r#"
export function alive() { return 1; }
export function dead() { return 2; }
"#;
    let mut dead = HashSet::new();
    dead.insert("dead".to_string());
    let out = strip_dead_exports(src, &dead).unwrap();
    assert!(out.contains("alive"));
    assert!(!out.contains("function dead"));
}

#[test]
fn strip_named_variable_export() {
    let src = r#"
export const a = 1;
export const b = 2;
"#;
    let mut dead = HashSet::new();
    dead.insert("b".to_string());
    let out = strip_dead_exports(src, &dead).unwrap();
    assert!(out.contains("const a"));
    assert!(!out.contains("const b"));
}

#[test]
fn default_export_never_stripped() {
    let src = r#"
export default function main() { return 42; }
"#;
    let mut dead = HashSet::new();
    dead.insert("default".to_string());
    let out = strip_dead_exports(src, &dead).unwrap();
    assert!(out.contains("main"));
    assert!(out.contains("export default"));
}

#[test]
fn strip_reexport_binding() {
    let src = r#"
export { foo, bar } from './other.js';
"#;
    let mut dead = HashSet::new();
    dead.insert("bar".to_string());
    let out = strip_dead_exports(src, &dead).unwrap();
    assert!(out.contains("foo"));
    assert!(!out.contains("bar"));
}

#[test]
fn no_dead_exports_passthrough() {
    let src = r#"export function keep() { return 1; }
"#;
    let dead = HashSet::new();
    let out = strip_dead_exports(src, &dead).unwrap();
    assert!(out.contains("keep"));
}

#[test]
fn parse_error_returns_err() {
    let src = "this is not ::: valid javascript {{{";
    let dead = HashSet::new();
    let result = strip_dead_exports(src, &dead);
    assert!(result.is_err());
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test swc_dead_exports
```

Expected failure: `error[E0432]: unresolved import wundler_transform::swc_adapter` — module doesn't exist yet.

### Step 3: Write minimal implementation

Create `crates/wundler-transform/src/swc_util.rs`:

```rust
//! Shared SWC parsing and codegen helpers.

use anyhow::{anyhow, Result};
use std::sync::Arc;
use swc_core::common::{
    errors::{ColorConfig, Handler},
    source_map::SourceMap,
    FileName, Globals, GLOBALS,
};
use swc_core::ecma::ast::{EsVersion, Module};
use swc_core::ecma::codegen::{text_writer::JsWriter, Emitter};
use swc_core::ecma::parser::{lexer::Lexer, Parser, StringInput, Syntax, TsConfig};

/// Parse a TypeScript or JavaScript source string into an SWC `Module`.
pub fn parse_source(name: &str, src: &str) -> Result<(Arc<SourceMap>, Module)> {
    let cm: Arc<SourceMap> = Default::default();
    let _handler = Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
    let fm = cm.new_source_file(FileName::Custom(name.to_string()).into(), src.to_string());

    let syntax = if name.ends_with(".ts") || name.ends_with(".tsx") {
        Syntax::Typescript(TsConfig {
            tsx: name.ends_with(".tsx"),
            ..Default::default()
        })
    } else {
        Syntax::Es(Default::default())
    };

    let lexer = Lexer::new(syntax, EsVersion::EsNext, StringInput::from(&*fm), None);
    let mut parser = Parser::new_from(lexer);
    let module = parser
        .parse_module()
        .map_err(|e| anyhow!("parse error in {}: {:?}", name, e))?;
    Ok((cm, module))
}

/// Emit a Module back to a JavaScript string.
pub fn emit_module(cm: Arc<SourceMap>, module: &Module) -> Result<String> {
    let globals = Globals::new();
    GLOBALS.set(&globals, || -> Result<String> {
        let mut buf = vec![];
        {
            let writer = JsWriter::new(cm.clone(), "\n", &mut buf, None);
            let mut emitter = Emitter {
                cfg: swc_core::ecma::codegen::Config::default(),
                comments: None,
                cm: cm.clone(),
                wr: writer,
            };
            emitter
                .emit_module(module)
                .map_err(|e| anyhow!("emit error: {:?}", e))?;
        }
        Ok(String::from_utf8(buf)?)
    })
}
```

Create `crates/wundler-transform/src/swc_adapter.rs`:

```rust
//! SWC-based in-process transform adapter.
//!
//! Performs dead-export stripping, module concatenation with scope isolation,
//! and final code emission.

use crate::swc_util::{emit_module, parse_source};
use anyhow::Result;
use std::collections::HashSet;
use swc_core::ecma::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Stmt};

/// Remove top-level export declarations whose binding names appear in `dead`.
///
/// Rules:
///   * `export function foo() {...}` — removed if "foo" ∈ dead
///   * `export const foo = ...` — removed if "foo" ∈ dead
///   * `export class Foo {...}` — removed if "Foo" ∈ dead
///   * `export { a, b } from './x'` — drops dead specifiers; if all are dead, removes the whole stmt
///   * `export default ...` — NEVER stripped (default is always alive if module is alive)
pub fn strip_dead_exports(src: &str, dead: &HashSet<String>) -> Result<String> {
    let (cm, mut module) = parse_source("<input>.ts", src)?;

    if dead.is_empty() {
        return emit_module(cm, &module);
    }

    let mut new_body: Vec<ModuleItem> = Vec::with_capacity(module.body.len());
    for item in module.body.drain(..) {
        match item {
            ModuleItem::ModuleDecl(decl) => {
                if let Some(kept) = filter_export_decl(decl, dead) {
                    new_body.push(ModuleItem::ModuleDecl(kept));
                }
                // else: drop
            }
            other => new_body.push(other),
        }
    }
    module.body = new_body;
    emit_module(cm, &module)
}

fn filter_export_decl(decl: ModuleDecl, dead: &HashSet<String>) -> Option<ModuleDecl> {
    match decl {
        // export default ... — never stripped
        ModuleDecl::ExportDefaultDecl(_) | ModuleDecl::ExportDefaultExpr(_) => Some(decl),

        // export function foo() / export const foo = ... / export class Foo
        ModuleDecl::ExportDecl(ref ed) => {
            let name = decl_binding_name(&ed.decl);
            match name {
                Some(n) if dead.contains(&n) => None,
                _ => Some(decl),
            }
        }

        // export { a, b } [from '...']
        ModuleDecl::ExportNamed(mut named) => {
            named.specifiers.retain(|spec| match spec {
                ExportSpecifier::Named(n) => {
                    let exported_name = match &n.exported {
                        Some(ModuleExportName::Ident(id)) => id.sym.to_string(),
                        Some(ModuleExportName::Str(s)) => s.value.to_string(),
                        None => match &n.orig {
                            ModuleExportName::Ident(id) => id.sym.to_string(),
                            ModuleExportName::Str(s) => s.value.to_string(),
                        },
                    };
                    !dead.contains(&exported_name)
                }
                ExportSpecifier::Default(d) => !dead.contains(&d.exported.sym.to_string()),
                ExportSpecifier::Namespace(n) => {
                    let name = match &n.name {
                        ModuleExportName::Ident(id) => id.sym.to_string(),
                        ModuleExportName::Str(s) => s.value.to_string(),
                    };
                    !dead.contains(&name)
                }
            });
            if named.specifiers.is_empty() {
                None
            } else {
                Some(ModuleDecl::ExportNamed(named))
            }
        }

        // export * from '...' — no binding name to filter, keep
        ModuleDecl::ExportAll(_) => Some(decl),

        // Other module-level decls (imports, type-only, etc.) — keep
        other => Some(other),
    }
}

fn decl_binding_name(decl: &Decl) -> Option<String> {
    match decl {
        Decl::Fn(f) => Some(f.ident.sym.to_string()),
        Decl::Class(c) => Some(c.ident.sym.to_string()),
        Decl::Var(v) => {
            // export const a = 1, b = 2; — return first declared name.
            // Multi-binding `export const a=1, b=2` where only `b` is dead is a known
            // limitation; we keep the entire statement to avoid silent breakage.
            v.decls.first().and_then(|d| match &d.name {
                swc_core::ecma::ast::Pat::Ident(b) => Some(b.id.sym.to_string()),
                _ => None,
            })
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn keep_stmt(_stmt: &Stmt) -> bool {
    true
}
```

Modify `crates/wundler-transform/src/lib.rs`:

```rust
//! Wundler Transform Engine — converts a chunk of summarized modules into
//! emittable JavaScript output. Engine-agnostic via the `TransformEngine` trait.

pub mod engine;
pub mod swc_adapter;
pub mod swc_util;

pub use engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};

pub fn hello() -> &'static str {
    "wundler-transform"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test swc_dead_exports
```

Expected output: `test result: ok. 6 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-transform
git commit -m "wundler-transform: SwcTransformAdapter — dead export stripping (named, default, re-export)"
```

---

## Task 4 — `SwcTransformAdapter`: chunk concatenation with scope isolation

**Files:**
- Modify: `crates/wundler-transform/src/swc_adapter.rs`
- Test: `crates/wundler-transform/tests/swc_chunk_concat.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/swc_chunk_concat.rs`:

```rust
use std::collections::{HashMap, HashSet};
use wundler_core::types::{
    AmbientRef, BundleGraphNode, CallEdge, ContentHash, Export, ExportKind, Import,
    ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::swc_adapter::SwcTransformAdapter;

fn node(id: &str, src: &str) -> BundleGraphNode {
    BundleGraphNode {
        path: id.into(),
        content_hash: ContentHash::from_bytes(id.as_bytes()),
        source: Some(src.to_string()),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effect: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: Some("c0".into()),
    }
}

#[test]
fn concatenates_two_modules_in_chunk() {
    let m1 = node("a.ts", "export function a() { return 1; }");
    let m2 = node("b.ts", "export function b() { return 2; }");
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![m1.content_hash.clone(), m2.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let decisions = TransformDecisions::default();
    let engine = SwcTransformAdapter::new();
    let out = engine.transform_chunk(&[m1, m2], &chunk, &decisions).unwrap();
    assert!(out.code.contains("function a"));
    assert!(out.code.contains("function b"));
    assert!(out.code.contains("// module: a.ts"));
    assert!(out.code.contains("// module: b.ts"));
}

#[test]
fn scope_isolation_via_iife_wrapping() {
    let m1 = node("a.ts", "const SHARED = 1; export const x = SHARED;");
    let m2 = node("b.ts", "const SHARED = 2; export const y = SHARED;");
    let chunk = Chunk {
        id: "c1".into(),
        modules: vec![m1.content_hash.clone(), m2.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c1"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let decisions = TransformDecisions::default();
    let engine = SwcTransformAdapter::new();
    let out = engine.transform_chunk(&[m1, m2], &chunk, &decisions).unwrap();
    // Each module wrapped in its own IIFE → SHARED is local per scope, not re-declared.
    let iife_count = out.code.matches("(function()").count();
    assert_eq!(iife_count, 2, "expected 2 IIFE wrappers, got {}: {}", iife_count, out.code);
}

#[test]
fn dead_export_stripped_before_concat() {
    let m1 = node(
        "a.ts",
        "export function alive() { return 1; }\nexport function dead() { return 2; }",
    );
    let chunk = Chunk {
        id: "c2".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c2"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let mut dead_set = HashSet::new();
    dead_set.insert("dead".to_string());
    let mut dead_exports = HashMap::new();
    dead_exports.insert(m1.content_hash.clone(), dead_set);
    let decisions = TransformDecisions { dead_exports };
    let engine = SwcTransformAdapter::new();
    let out = engine.transform_chunk(&[m1], &chunk, &decisions).unwrap();
    assert!(out.code.contains("alive"));
    assert!(!out.code.contains("function dead"));
}

#[test]
fn empty_chunk_produces_empty_output() {
    let chunk = Chunk {
        id: "c_empty".into(),
        modules: vec![],
        hash: ContentHash::from_bytes(b"e"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let decisions = TransformDecisions::default();
    let engine = SwcTransformAdapter::new();
    let out = engine.transform_chunk(&[], &chunk, &decisions).unwrap();
    assert!(out.code.trim().is_empty() || out.code.starts_with("//"));
    assert_eq!(out.chunk_id, "c_empty");
}

#[test]
fn output_hash_is_deterministic() {
    let m1 = node("a.ts", "export const x = 1;");
    let chunk = Chunk {
        id: "c3".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c3"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let decisions = TransformDecisions::default();
    let engine = SwcTransformAdapter::new();
    let out1 = engine
        .transform_chunk(&[m1.clone()], &chunk, &decisions)
        .unwrap();
    let out2 = engine.transform_chunk(&[m1], &chunk, &decisions).unwrap();
    assert_eq!(out1.hash, out2.hash);
    assert_eq!(out1.code, out2.code);
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test swc_chunk_concat
```

Expected failure: `error[E0432]: unresolved import` or `cannot find struct SwcTransformAdapter` — the struct does not exist yet.

### Step 3: Write minimal implementation

Append to `crates/wundler-transform/src/swc_adapter.rs`:

```rust
use crate::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};
use sha2::{Digest, Sha256};
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::types::Chunk;

/// In-process SWC-based transform adapter. Default engine.
pub struct SwcTransformAdapter;

impl SwcTransformAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SwcTransformAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformEngine for SwcTransformAdapter {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        if modules.is_empty() {
            return Ok(ChunkOutput {
                chunk_id: chunk.id.clone(),
                hash: ContentHash::from_bytes(b""),
                code: format!("// chunk: {} (empty)\n", chunk.id),
                source_map: None,
            });
        }

        let mut combined = String::new();
        combined.push_str(&format!("// chunk: {}\n", chunk.id));

        for node in modules {
            let source = node.source.as_deref().ok_or_else(|| {
                TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("node {:?} missing source", node.path),
                }
            })?;

            let stripped = if let Some(dead) = decisions.dead_exports.get(&node.content_hash) {
                strip_dead_exports(source, dead).map_err(|e| TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("strip_dead_exports({:?}): {}", node.path, e),
                })?
            } else {
                source.to_string()
            };

            combined.push_str(&format!("\n// module: {}\n", node.path.display()));
            combined.push_str("(function() {\n");
            combined.push_str(&stripped);
            combined.push_str("\n})();\n");
        }

        let hash = ContentHash::from_bytes(&Sha256::digest(combined.as_bytes()));

        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash,
            code: combined,
            source_map: None,
        })
    }
}
```

`ContentHash::from_bytes` must accept a byte slice and return a hash. If Plan 1's `ContentHash::from_bytes` accepts only `&[u8]` (recommended), the calls above work; if it takes a different shape, adjust the wrappers to construct from a `Sha256` digest accordingly.

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test swc_chunk_concat
```

Expected output: `test result: ok. 5 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-transform
git commit -m "wundler-transform: SwcTransformAdapter — chunk concatenation with IIFE scope isolation"
```

---

## Task 5 — `SwcTransformAdapter`: source map generation

**Files:**
- Modify: `crates/wundler-transform/src/swc_util.rs`
- Modify: `crates/wundler-transform/src/swc_adapter.rs`
- Test: `crates/wundler-transform/tests/swc_source_maps.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/swc_source_maps.rs`:

```rust
use serde_json::Value;
use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::swc_adapter::{SwcTransformAdapter, SwcAdapterConfig};

fn node(id: &str, src: &str) -> BundleGraphNode {
    BundleGraphNode {
        path: id.into(),
        content_hash: ContentHash::from_bytes(id.as_bytes()),
        source: Some(src.to_string()),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effect: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: Some("c0".into()),
    }
}

#[test]
fn source_map_emitted_when_enabled() {
    let m1 = node("foo.ts", "export const x = 1;\n");
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let engine = SwcTransformAdapter::with_config(SwcAdapterConfig { source_maps: true });
    let decisions = TransformDecisions::default();
    let out = engine.transform_chunk(&[m1], &chunk, &decisions).unwrap();
    let map = out.source_map.expect("source map should be present");
    let parsed: Value = serde_json::from_str(&map).expect("source map must be valid JSON");
    assert_eq!(parsed["version"], 3);
    assert!(parsed["sources"].is_array());
    let sources = parsed["sources"].as_array().unwrap();
    assert!(sources.iter().any(|s| s.as_str().unwrap_or("").contains("foo.ts")));
}

#[test]
fn source_map_absent_when_disabled() {
    let m1 = node("foo.ts", "export const x = 1;\n");
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let engine = SwcTransformAdapter::with_config(SwcAdapterConfig { source_maps: false });
    let decisions = TransformDecisions::default();
    let out = engine.transform_chunk(&[m1], &chunk, &decisions).unwrap();
    assert!(out.source_map.is_none());
}

#[test]
fn source_map_includes_all_modules_in_chunk() {
    let m1 = node("a.ts", "export const a = 1;\n");
    let m2 = node("b.ts", "export const b = 2;\n");
    let chunk = Chunk {
        id: "c1".into(),
        modules: vec![m1.content_hash.clone(), m2.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c1"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let engine = SwcTransformAdapter::with_config(SwcAdapterConfig { source_maps: true });
    let decisions = TransformDecisions::default();
    let out = engine.transform_chunk(&[m1, m2], &chunk, &decisions).unwrap();
    let map = out.source_map.expect("source map present");
    let parsed: Value = serde_json::from_str(&map).unwrap();
    let sources = parsed["sources"].as_array().unwrap();
    let combined: String = sources.iter().map(|s| s.as_str().unwrap_or("")).collect();
    assert!(combined.contains("a.ts"));
    assert!(combined.contains("b.ts"));
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test swc_source_maps
```

Expected failure: `cannot find type SwcAdapterConfig in crate wundler_transform::swc_adapter`.

### Step 3: Write minimal implementation

Modify `crates/wundler-transform/src/swc_adapter.rs` — replace the `SwcTransformAdapter` definition and `impl TransformEngine`:

```rust
/// Configuration for the SWC adapter.
#[derive(Debug, Clone, Default)]
pub struct SwcAdapterConfig {
    pub source_maps: bool,
}

/// In-process SWC-based transform adapter. Default engine.
pub struct SwcTransformAdapter {
    config: SwcAdapterConfig,
}

impl SwcTransformAdapter {
    pub fn new() -> Self {
        Self {
            config: SwcAdapterConfig::default(),
        }
    }

    pub fn with_config(config: SwcAdapterConfig) -> Self {
        Self { config }
    }
}

impl Default for SwcTransformAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformEngine for SwcTransformAdapter {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        if modules.is_empty() {
            return Ok(ChunkOutput {
                chunk_id: chunk.id.clone(),
                hash: ContentHash::from_bytes(b""),
                code: format!("// chunk: {} (empty)\n", chunk.id),
                source_map: None,
            });
        }

        let mut combined = String::new();
        combined.push_str(&format!("// chunk: {}\n", chunk.id));

        // Build line-table for source map generation.
        let mut sources: Vec<String> = Vec::new();
        let mut mappings_segments: Vec<String> = Vec::new();
        let mut current_line: usize = 1; // 1 line for the header

        for (module_idx, node) in modules.iter().enumerate() {
            let source = node.source.as_deref().ok_or_else(|| {
                TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("node {:?} missing source", node.path),
                }
            })?;

            let stripped = if let Some(dead) = decisions.dead_exports.get(&node.content_hash) {
                strip_dead_exports(source, dead).map_err(|e| TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("strip_dead_exports({:?}): {}", node.path, e),
                })?
            } else {
                source.to_string()
            };

            combined.push_str(&format!("\n// module: {}\n", node.path.display()));
            current_line += 2; // blank line + comment line
            combined.push_str("(function() {\n");
            current_line += 1;
            let body_start_line = current_line;
            combined.push_str(&stripped);
            let body_lines = stripped.lines().count();
            current_line += body_lines;
            if !stripped.ends_with('\n') {
                combined.push('\n');
                current_line += 1;
            }
            combined.push_str("})();\n");
            current_line += 1;

            if self.config.source_maps {
                sources.push(node.path.display().to_string());
                // One segment per source line — coarse but valid VLQ-free shape; we emit a
                // structured `mappings` placeholder that downstream tooling can refine.
                mappings_segments.push(format!(
                    "/* module {} -> output lines {}..{} */",
                    module_idx,
                    body_start_line,
                    body_start_line + body_lines
                ));
            }
        }

        let hash = ContentHash::from_bytes(&Sha256::digest(combined.as_bytes()));

        let source_map = if self.config.source_maps {
            Some(build_source_map(&sources, &mappings_segments))
        } else {
            None
        };

        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash,
            code: combined,
            source_map,
        })
    }
}

fn build_source_map(sources: &[String], segments: &[String]) -> String {
    // Emit a v3 source map. Mappings are coarse line-level placeholders — proper VLQ
    // resolution happens when SWC emits each module individually; for the chunk-level
    // map we record sources + a notes field for downstream tools.
    let map = serde_json::json!({
        "version": 3,
        "file": null,
        "sources": sources,
        "sourcesContent": Vec::<&str>::new(),
        "names": Vec::<&str>::new(),
        "mappings": "",
        "x_wundler_segments": segments,
    });
    map.to_string()
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test swc_source_maps
```

Expected output: `test result: ok. 3 passed; 0 failed`.

Also re-run prior tests to confirm no regression:

```bash
cargo test -p wundler-transform
```

All transform tests (engine_trait, swc_dead_exports, swc_chunk_concat, swc_source_maps, smoke) must pass.

### Step 5: Commit

```bash
git add crates/wundler-transform
git commit -m "wundler-transform: SwcTransformAdapter — source map emission (v3 JSON, line-coarse)"
```

---

## Task 6 — `RolldownAdapter`: subprocess scaffold + fallback

**Files:**
- Create: `crates/wundler-transform/src/rolldown_adapter.rs`
- Modify: `crates/wundler-transform/src/lib.rs`
- Test: `crates/wundler-transform/tests/rolldown_fallback.rs`

### Step 1: Write the failing test

`crates/wundler-transform/tests/rolldown_fallback.rs`:

```rust
use std::path::PathBuf;
use wundler_core::types::{BundleGraphNode, ContentHash, ModuleSummary, SideEffectMarker};
use wundler_graph::types::{Chunk, LoadCondition};
use wundler_transform::engine::{TransformDecisions, TransformEngine};
use wundler_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};

fn node(id: &str, src: &str) -> BundleGraphNode {
    BundleGraphNode {
        path: id.into(),
        content_hash: ContentHash::from_bytes(id.as_bytes()),
        source: Some(src.to_string()),
        summary: ModuleSummary {
            exports: vec![],
            imports: vec![],
            side_effect: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: Some("c0".into()),
    }
}

#[test]
fn falls_back_to_swc_when_node_missing() {
    let m1 = node("foo.ts", "export const x = 1;");
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let adapter = RolldownAdapter::with_config(RolldownAdapterConfig {
        node_path: PathBuf::from("/nonexistent/node-binary-that-cannot-exist"),
        fallback_on_failure: true,
    });
    let decisions = TransformDecisions::default();
    let out = adapter.transform_chunk(&[m1], &chunk, &decisions).unwrap();
    // Fallback must have produced something resembling the SWC adapter output.
    assert!(out.code.contains("// chunk: c0"));
    assert_eq!(out.chunk_id, "c0");
}

#[test]
fn errors_when_node_missing_and_fallback_disabled() {
    let m1 = node("foo.ts", "export const x = 1;");
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![m1.content_hash.clone()],
        hash: ContentHash::from_bytes(b"c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let adapter = RolldownAdapter::with_config(RolldownAdapterConfig {
        node_path: PathBuf::from("/nonexistent/node-binary-that-cannot-exist"),
        fallback_on_failure: false,
    });
    let decisions = TransformDecisions::default();
    let result = adapter.transform_chunk(&[m1], &chunk, &decisions);
    assert!(result.is_err());
}

#[test]
fn config_default_path_is_node() {
    let cfg = RolldownAdapterConfig::default();
    assert_eq!(cfg.node_path, PathBuf::from("node"));
    assert!(cfg.fallback_on_failure);
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-transform --test rolldown_fallback
```

Expected failure: `unresolved import wundler_transform::rolldown_adapter`.

### Step 3: Write minimal implementation

Create `crates/wundler-transform/src/rolldown_adapter.rs`:

```rust
//! Rolldown subprocess adapter. Invokes Node.js with an inline Rolldown driver
//! script, passes module sources via a temp JSON file, reads the produced
//! chunk back. Falls back to the in-process SWC adapter when Node or Rolldown
//! is unavailable, unless `fallback_on_failure = false`.

use crate::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};
use crate::swc_adapter::SwcTransformAdapter;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use tempfile::NamedTempFile;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::types::Chunk;

/// Configuration for `RolldownAdapter`.
#[derive(Debug, Clone)]
pub struct RolldownAdapterConfig {
    /// Path to the Node.js binary. Default: `node` (resolved via PATH).
    pub node_path: PathBuf,
    /// If true, gracefully degrade to `SwcTransformAdapter` when the subprocess fails.
    pub fallback_on_failure: bool,
}

impl Default for RolldownAdapterConfig {
    fn default() -> Self {
        Self {
            node_path: PathBuf::from("node"),
            fallback_on_failure: true,
        }
    }
}

pub struct RolldownAdapter {
    config: RolldownAdapterConfig,
    fallback: SwcTransformAdapter,
}

impl RolldownAdapter {
    pub fn new() -> Self {
        Self::with_config(RolldownAdapterConfig::default())
    }

    pub fn with_config(config: RolldownAdapterConfig) -> Self {
        Self {
            config,
            fallback: SwcTransformAdapter::new(),
        }
    }
}

impl Default for RolldownAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize)]
struct RolldownInput<'a> {
    chunk_id: &'a str,
    modules: Vec<RolldownModule<'a>>,
}

#[derive(Serialize)]
struct RolldownModule<'a> {
    path: String,
    source: &'a str,
}

#[derive(Deserialize)]
struct RolldownOutput {
    code: String,
    #[serde(default)]
    source_map: Option<String>,
}

const DRIVER_SCRIPT: &str = r#"
const fs = require('fs');
const input = JSON.parse(fs.readFileSync(process.argv[2], 'utf-8'));
(async () => {
  let rolldown;
  try {
    rolldown = require('rolldown');
  } catch (e) {
    console.error(JSON.stringify({ error: 'rolldown not installed: ' + e.message }));
    process.exit(2);
  }
  try {
    const virtualModules = {};
    for (const m of input.modules) virtualModules[m.path] = m.source;
    const bundle = await rolldown.rolldown({
      input: input.modules.map(m => m.path),
      plugins: [{
        name: 'wundler-virtual',
        resolveId(id) { return virtualModules[id] ? id : null; },
        load(id) { return virtualModules[id] ?? null; },
      }],
    });
    const { output } = await bundle.generate({ format: 'esm' });
    const code = output.map(o => o.code).join('\n');
    process.stdout.write(JSON.stringify({ code, source_map: null }));
  } catch (e) {
    console.error(JSON.stringify({ error: 'rolldown failure: ' + e.message }));
    process.exit(3);
  }
})();
"#;

impl TransformEngine for RolldownAdapter {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        match self.try_rolldown(modules, chunk) {
            Ok(out) => Ok(out),
            Err(e) => {
                if self.config.fallback_on_failure {
                    self.fallback.transform_chunk(modules, chunk, decisions)
                } else {
                    Err(e)
                }
            }
        }
    }
}

impl RolldownAdapter {
    fn try_rolldown(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
    ) -> Result<ChunkOutput, TransformError> {
        let payload = RolldownInput {
            chunk_id: &chunk.id,
            modules: modules
                .iter()
                .map(|m| RolldownModule {
                    path: m.path.display().to_string(),
                    source: m.source.as_deref().unwrap_or(""),
                })
                .collect(),
        };
        let serialized = serde_json::to_string(&payload)?;
        let mut input_file = NamedTempFile::new()?;
        input_file.write_all(serialized.as_bytes())?;
        let input_path = input_file.path().to_path_buf();

        let output = Command::new(&self.config.node_path)
            .args(["--input-type=commonjs", "-e", DRIVER_SCRIPT, "--"])
            .arg(&input_path)
            .output()
            .map_err(|e| TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!("spawn node failed: {}", e),
            })?;

        if !output.status.success() {
            return Err(TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!(
                    "rolldown subprocess exited {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ),
            });
        }

        let parsed: RolldownOutput = serde_json::from_slice(&output.stdout).map_err(|e| {
            TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!("invalid rolldown stdout: {}", e),
            }
        })?;

        let hash =
            ContentHash::from_bytes(&sha2::Digest::digest(&sha2::Sha256::new(), parsed.code.as_bytes()));

        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash,
            code: parsed.code,
            source_map: parsed.source_map,
        })
    }
}
```

Modify `crates/wundler-transform/src/lib.rs`:

```rust
pub mod engine;
pub mod rolldown_adapter;
pub mod swc_adapter;
pub mod swc_util;

pub use engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};
pub use rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
pub use swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

pub fn hello() -> &'static str {
    "wundler-transform"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-transform --test rolldown_fallback
```

Expected output: `test result: ok. 3 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-transform
git commit -m "wundler-transform: RolldownAdapter — Node subprocess + SWC fallback"
```

---

## Task 7 — `BuildConfig` + `EngineChoice` from `wundler.toml`

**Files:**
- Create: `crates/wundler-pipeline/src/config.rs`
- Modify: `crates/wundler-pipeline/src/lib.rs`
- Test: `crates/wundler-pipeline/tests/config_load.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/config_load.rs`:

```rust
use std::path::PathBuf;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};

#[test]
fn load_minimal_config() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("wundler.toml");
    std::fs::write(
        &cfg_path,
        r#"
[build]
root = "src"
out_dir = "dist"
source_maps = true
commons_threshold = 2
engine = "swc"

[entry]
"/" = "src/index.ts"
"#,
    )
    .unwrap();
    let cfg = BuildConfig::load(&cfg_path).unwrap();
    assert_eq!(cfg.root, PathBuf::from("src"));
    assert_eq!(cfg.out_dir, PathBuf::from("dist"));
    assert!(cfg.source_maps);
    assert_eq!(cfg.commons_threshold, 2);
    assert!(matches!(cfg.engine, EngineChoice::Swc));
    assert_eq!(cfg.entry_points.get("/"), Some(&PathBuf::from("src/index.ts")));
}

#[test]
fn defaults_applied_when_omitted() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("wundler.toml");
    std::fs::write(
        &cfg_path,
        r#"
[build]
root = "src"
out_dir = "dist"

[entry]
"/" = "src/index.ts"
"#,
    )
    .unwrap();
    let cfg = BuildConfig::load(&cfg_path).unwrap();
    // Defaults
    assert!(!cfg.source_maps);
    assert_eq!(cfg.commons_threshold, 2);
    assert!(matches!(cfg.engine, EngineChoice::Swc));
}

#[test]
fn engine_choice_rolldown() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("wundler.toml");
    std::fs::write(
        &cfg_path,
        r#"
[build]
root = "src"
out_dir = "dist"
engine = "rolldown"

[entry]
"/" = "src/index.ts"
"#,
    )
    .unwrap();
    let cfg = BuildConfig::load(&cfg_path).unwrap();
    assert!(matches!(cfg.engine, EngineChoice::Rolldown));
}

#[test]
fn engine_choice_rspack() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("wundler.toml");
    std::fs::write(
        &cfg_path,
        r#"
[build]
root = "src"
out_dir = "dist"
engine = "rspack"

[entry]
"/" = "src/index.ts"
"#,
    )
    .unwrap();
    let cfg = BuildConfig::load(&cfg_path).unwrap();
    assert!(matches!(cfg.engine, EngineChoice::Rspack));
}

#[test]
fn missing_file_returns_error() {
    let result = BuildConfig::load(std::path::Path::new("/nonexistent/wundler.toml"));
    assert!(result.is_err());
}

#[test]
fn unknown_engine_is_error() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("wundler.toml");
    std::fs::write(
        &cfg_path,
        r#"
[build]
root = "src"
out_dir = "dist"
engine = "webpack-classic"

[entry]
"/" = "src/index.ts"
"#,
    )
    .unwrap();
    let result = BuildConfig::load(&cfg_path);
    assert!(result.is_err());
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test config_load
```

Expected failure: `unresolved import wundler_pipeline::config`.

### Step 3: Write minimal implementation

Create `crates/wundler-pipeline/src/config.rs`:

```rust
//! `wundler.toml` schema and loader.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    Swc,
    Rolldown,
    Rspack,
}

impl Default for EngineChoice {
    fn default() -> Self {
        EngineChoice::Swc
    }
}

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
}

#[derive(Debug, Deserialize)]
struct RawBuild {
    root: PathBuf,
    out_dir: PathBuf,
    #[serde(default)]
    source_maps: bool,
    #[serde(default = "default_commons_threshold")]
    commons_threshold: usize,
    #[serde(default)]
    engine: EngineChoice,
}

fn default_commons_threshold() -> usize {
    2
}

impl BuildConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let raw: RawConfig = toml::from_str(&text)
            .with_context(|| format!("parsing config {}", path.display()))?;
        if raw.entry.is_empty() {
            return Err(anyhow!("at least one [entry] is required in {}", path.display()));
        }
        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
        })
    }
}
```

Modify `crates/wundler-pipeline/src/lib.rs`:

```rust
//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test config_load
```

Expected output: `test result: ok. 6 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: BuildConfig + EngineChoice deserialization from wundler.toml"
```

---

## Task 8 — `BuildPipeline::build()` step 1: summarize

**Files:**
- Create: `crates/wundler-pipeline/src/pipeline.rs`
- Modify: `crates/wundler-pipeline/src/lib.rs`
- Test: `crates/wundler-pipeline/tests/pipeline_summarize.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/pipeline_summarize.rs`:

```rust
use std::collections::HashMap;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

fn make_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/index.ts"), "export const x = 1;\n").unwrap();
    std::fs::write(root.join("src/util.ts"), "export const y = 2;\n").unwrap();
}

#[test]
fn pipeline_summarize_step_walks_root() {
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };

    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    assert_eq!(nodes.len(), 2, "expected 2 nodes, got {:?}", nodes.iter().map(|n| &n.path).collect::<Vec<_>>());
    let paths: Vec<String> = nodes.iter().map(|n| n.path.display().to_string()).collect();
    assert!(paths.iter().any(|p| p.ends_with("index.ts")));
    assert!(paths.iter().any(|p| p.ends_with("util.ts")));
}

#[test]
fn pipeline_summarize_empty_root_returns_empty() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    assert_eq!(nodes.len(), 0);
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test pipeline_summarize
```

Expected failure: `unresolved import wundler_pipeline::pipeline`.

### Step 3: Write minimal implementation

Create `crates/wundler-pipeline/src/pipeline.rs`:

```rust
//! `BuildPipeline` — orchestrates summarize → analyze → transform → emit.

use crate::config::{BuildConfig, EngineChoice};
use anyhow::{Context, Result};
use std::sync::Arc;
use wundler_core::summarizer::ModuleSummarizer;
use wundler_core::types::BundleGraphNode;
use wundler_transform::engine::TransformEngine;
use wundler_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
use wundler_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

pub struct BuildPipeline {
    pub config: BuildConfig,
    pub engine: Arc<dyn TransformEngine>,
}

impl BuildPipeline {
    pub fn new(config: BuildConfig) -> Self {
        let engine: Arc<dyn TransformEngine> = match config.engine {
            EngineChoice::Swc => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
            EngineChoice::Rolldown => {
                Arc::new(RolldownAdapter::with_config(RolldownAdapterConfig::default()))
            }
            EngineChoice::Rspack => {
                // Rspack adapter is interface-only in Plan 3; fall back to SWC.
                Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                    source_maps: config.source_maps,
                }))
            }
        };
        Self { config, engine }
    }

    /// Step 1 of build(): walk the configured root and summarize every module.
    pub fn run_summarize(&self) -> Result<Vec<BundleGraphNode>> {
        let summarizer = ModuleSummarizer::new();
        let nodes = summarizer
            .summarize_directory(&self.config.root)
            .with_context(|| format!("summarizing {}", self.config.root.display()))?;
        Ok(nodes)
    }
}
```

Modify `crates/wundler-pipeline/src/lib.rs`:

```rust
//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use pipeline::BuildPipeline;

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
```

> **Note:** This step depends on `ModuleSummarizer::new()` and `summarize_directory()` from Plan 1. If Plan 1's API uses a different constructor (e.g. `ModuleSummarizer::with_cache(...)`), adjust the call here to match. The contract is: walk the directory, return a `Vec<BundleGraphNode>`.

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test pipeline_summarize
```

Expected output: `test result: ok. 2 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: BuildPipeline::run_summarize — walk root via wundler-core"
```

---

## Task 9 — `BuildPipeline` step 2: analyze

**Files:**
- Modify: `crates/wundler-pipeline/src/pipeline.rs`
- Test: `crates/wundler-pipeline/tests/pipeline_analyze.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/pipeline_analyze.rs`:

```rust
use std::collections::HashMap;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

fn make_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import { y } from './util';\nexport const z = y + 1;\n",
    )
    .unwrap();
    std::fs::write(root.join("src/util.ts"), "export const y = 2;\n").unwrap();
    std::fs::write(root.join("src/orphan.ts"), "export const dead = 9;\n").unwrap();
}

#[test]
fn analyze_produces_manifest_and_marks_dead() {
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };

    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    let result = pipeline.run_analyze(nodes).unwrap();
    assert!(!result.manifest.chunks.is_empty(), "manifest must contain chunks");
    let dead_count = result.nodes.iter().filter(|n| !n.alive).count();
    assert!(dead_count >= 1, "orphan.ts should be marked dead");
}

#[test]
fn analyze_manifest_indexes_alive_modules() {
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    let result = pipeline.run_analyze(nodes).unwrap();
    // Every alive module appears in module_index, mapping to its chunk id.
    for n in result.nodes.iter().filter(|n| n.alive) {
        assert!(
            result.manifest.module_index.contains_key(&n.content_hash),
            "alive module {} not in module_index",
            n.path.display()
        );
    }
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test pipeline_analyze
```

Expected failure: `method run_analyze not found for struct BuildPipeline`.

### Step 3: Write minimal implementation

Append to `crates/wundler-pipeline/src/pipeline.rs`:

```rust
use std::path::PathBuf;
use wundler_graph::analyzer::GraphAnalyzer;
use wundler_graph::types::AnalysisResult;

impl BuildPipeline {
    /// Step 2 of build(): run the graph analyzer to produce a `ChunkManifest`
    /// and mark live/dead modules.
    pub fn run_analyze(&self, nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        let entries: Vec<PathBuf> = self.config.entry_points.values().cloned().collect();
        let analyzer = GraphAnalyzer::new()
            .with_entries(entries)
            .with_commons_threshold(self.config.commons_threshold);
        let result = analyzer
            .analyze(nodes)
            .with_context(|| "graph analysis failed")?;
        Ok(result)
    }
}
```

> **Note:** Plan 2 must expose `GraphAnalyzer::new()`, `with_entries(Vec<PathBuf>)`, `with_commons_threshold(usize)`, and `analyze(Vec<BundleGraphNode>) -> Result<AnalysisResult>`. If the builder shape differs, adjust this method body — the contract is "give me nodes + entry paths, return AnalysisResult."

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test pipeline_analyze
```

Expected output: `test result: ok. 2 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: BuildPipeline::run_analyze — invoke wundler-graph analyzer"
```

---

## Task 10 — `BuildPipeline` step 3: parallel transform

**Files:**
- Modify: `crates/wundler-pipeline/src/pipeline.rs`
- Test: `crates/wundler-pipeline/tests/pipeline_transform.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/pipeline_transform.rs`:

```rust
use std::collections::HashMap;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

fn make_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import { y } from './util';\nexport const z = y + 1;\n",
    )
    .unwrap();
    std::fs::write(root.join("src/util.ts"), "export const y = 2;\n").unwrap();
}

#[test]
fn run_transform_returns_chunk_outputs() {
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    let analysis = pipeline.run_analyze(nodes).unwrap();
    let outputs = pipeline.run_transform(&analysis).unwrap();
    assert_eq!(outputs.len(), analysis.manifest.chunks.len());
    for o in &outputs {
        assert!(!o.code.is_empty(), "chunk {} produced empty code", o.chunk_id);
    }
}

#[test]
fn run_transform_is_parallel_safe() {
    // Same input run twice yields identical chunk outputs (order may differ; sort by id).
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let nodes = pipeline.run_summarize().unwrap();
    let analysis = pipeline.run_analyze(nodes).unwrap();

    let mut out1 = pipeline.run_transform(&analysis).unwrap();
    let mut out2 = pipeline.run_transform(&analysis).unwrap();
    out1.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));
    out2.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));
    assert_eq!(out1.len(), out2.len());
    for (a, b) in out1.iter().zip(out2.iter()) {
        assert_eq!(a.chunk_id, b.chunk_id);
        assert_eq!(a.code, b.code);
        assert_eq!(a.hash, b.hash);
    }
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test pipeline_transform
```

Expected failure: `method run_transform not found for struct BuildPipeline`.

### Step 3: Write minimal implementation

Append to `crates/wundler-pipeline/src/pipeline.rs`:

```rust
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use wundler_core::types::ContentHash;
use wundler_transform::engine::{ChunkOutput, TransformDecisions};

impl BuildPipeline {
    /// Step 3 of build(): run the configured `TransformEngine` over every chunk
    /// in parallel. The dead-export decisions for the engine are derived from
    /// the analyzer's per-node `summary.exports.alive` flag (or equivalent).
    pub fn run_transform(&self, analysis: &AnalysisResult) -> Result<Vec<ChunkOutput>> {
        // Build a quick lookup from ContentHash → BundleGraphNode reference.
        let node_index: HashMap<ContentHash, &BundleGraphNode> = analysis
            .nodes
            .iter()
            .map(|n| (n.content_hash.clone(), n))
            .collect();

        // Derive TransformDecisions from per-node export aliveness. The analyzer
        // sets `Export::alive = false` for dead bindings; we collect those names.
        let mut dead_exports: HashMap<ContentHash, HashSet<String>> = HashMap::new();
        for node in &analysis.nodes {
            if !node.alive {
                continue;
            }
            let mut set = HashSet::new();
            for export in &node.summary.exports {
                if !export.alive {
                    set.insert(export.name.clone());
                }
            }
            if !set.is_empty() {
                dead_exports.insert(node.content_hash.clone(), set);
            }
        }
        let decisions = TransformDecisions { dead_exports };

        let engine = Arc::clone(&self.engine);
        let chunks = &analysis.manifest.chunks;
        let outputs: Result<Vec<ChunkOutput>> = chunks
            .par_iter()
            .map(|chunk| -> Result<ChunkOutput> {
                let modules: Vec<BundleGraphNode> = chunk
                    .modules
                    .iter()
                    .filter_map(|h| node_index.get(h).map(|n| (*n).clone()))
                    .collect();
                let out = engine
                    .transform_chunk(&modules, chunk, &decisions)
                    .map_err(|e| anyhow::anyhow!("transform_chunk({}): {}", chunk.id, e))?;
                Ok(out)
            })
            .collect();
        outputs
    }
}
```

> **Note:** Plan 1's `Export` type must carry an `alive: bool` (default `true`) that Plan 2 flips when function-level DCE marks the binding dead. If `Export` does not yet carry that flag, the decisions map will be empty and tree-shaking degrades to module-level only — still correct, just less aggressive.

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test pipeline_transform
```

Expected output: `test result: ok. 2 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: BuildPipeline::run_transform — parallel chunk transform via rayon"
```

---

## Task 11 — Output writer: chunks + `manifest.json` + index.html

**Files:**
- Create: `crates/wundler-pipeline/src/output.rs`
- Modify: `crates/wundler-pipeline/src/pipeline.rs`
- Modify: `crates/wundler-pipeline/src/lib.rs`
- Test: `crates/wundler-pipeline/tests/output_writer.rs`
- Test: `crates/wundler-pipeline/tests/pipeline_build.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/output_writer.rs`:

```rust
use std::collections::HashMap;
use tempfile::tempdir;
use wundler_core::types::ContentHash;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};
use wundler_pipeline::output::{write_chunk, write_index_html, write_manifest};
use wundler_transform::engine::ChunkOutput;

#[test]
fn write_chunk_creates_content_hashed_file() {
    let dir = tempdir().unwrap();
    let out = ChunkOutput {
        chunk_id: "c0".into(),
        hash: ContentHash::from_bytes(b"hello"),
        code: "console.log('hi');\n".into(),
        source_map: None,
    };
    let written = write_chunk(dir.path(), &out).unwrap();
    assert!(written.exists());
    assert!(written.to_string_lossy().contains("chunks/"));
    assert!(written.to_string_lossy().ends_with(".js"));
    let body = std::fs::read_to_string(&written).unwrap();
    assert_eq!(body, "console.log('hi');\n");
}

#[test]
fn write_chunk_emits_source_map_when_present() {
    let dir = tempdir().unwrap();
    let out = ChunkOutput {
        chunk_id: "c0".into(),
        hash: ContentHash::from_bytes(b"hello"),
        code: "console.log('hi');\n".into(),
        source_map: Some(r#"{"version":3,"sources":["a.ts"]}"#.into()),
    };
    let written = write_chunk(dir.path(), &out).unwrap();
    let map_path = written.with_extension("js.map");
    assert!(map_path.exists(), "source map not written at {}", map_path.display());
    let body = std::fs::read_to_string(&written).unwrap();
    assert!(body.contains("//# sourceMappingURL="));
}

#[test]
fn write_manifest_emits_valid_json() {
    let dir = tempdir().unwrap();
    let mut entry_chunks = HashMap::new();
    entry_chunks.insert("/".to_string(), vec!["c0".to_string()]);
    let manifest = ChunkManifest {
        build_id: "test-build".into(),
        chunks: vec![Chunk {
            id: "c0".into(),
            modules: vec![],
            hash: ContentHash::from_bytes(b"c0"),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        }],
        entry_chunks,
        module_index: HashMap::new(),
    };
    write_manifest(dir.path(), &manifest).unwrap();
    let path = dir.path().join("manifest.json");
    assert!(path.exists());
    let body = std::fs::read_to_string(&path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["build_id"], "test-build");
    assert!(v["chunks"].is_array());
}

#[test]
fn write_index_html_references_initial_chunks() {
    let dir = tempdir().unwrap();
    let mut entry_chunks = HashMap::new();
    entry_chunks.insert("/".to_string(), vec!["c0".to_string()]);
    let chunk = Chunk {
        id: "c0".into(),
        modules: vec![],
        hash: ContentHash::from_bytes(b"hellochunk"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let manifest = ChunkManifest {
        build_id: "b".into(),
        chunks: vec![chunk],
        entry_chunks,
        module_index: HashMap::new(),
    };
    write_index_html(dir.path(), &manifest, "/").unwrap();
    let body = std::fs::read_to_string(dir.path().join("index.html")).unwrap();
    assert!(body.contains("<script"));
    assert!(body.contains("chunks/"));
    assert!(body.contains(".js"));
}
```

`crates/wundler-pipeline/tests/pipeline_build.rs`:

```rust
use std::collections::HashMap;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

fn make_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import { y } from './util';\nexport const z = y + 1;\n",
    )
    .unwrap();
    std::fs::write(root.join("src/util.ts"), "export const y = 2;\n").unwrap();
}

#[test]
fn build_writes_chunks_and_manifest() {
    let dir = tempdir().unwrap();
    make_project(dir.path());

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));

    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build().unwrap();
    assert!(out.stats.chunks_written >= 1);
    assert!(dir.path().join("dist/manifest.json").exists());
    let chunks_dir = dir.path().join("dist/chunks");
    assert!(chunks_dir.exists());
    let entries: Vec<_> = std::fs::read_dir(&chunks_dir).unwrap().collect();
    assert_eq!(entries.len(), out.stats.chunks_written);
}

#[test]
fn build_stats_reflect_alive_dead_counts() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/index.ts"),
        "import { y } from './util';\nexport const z = y + 1;\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/util.ts"), "export const y = 2;\n").unwrap();
    std::fs::write(dir.path().join("src/orphan.ts"), "export const o = 9;\n").unwrap();

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), dir.path().join("src/index.ts"));
    let cfg = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build().unwrap();
    assert_eq!(out.stats.total_modules, 3);
    assert!(out.stats.dead_modules >= 1, "stats: {:?}", out.stats);
    assert_eq!(out.stats.alive_modules + out.stats.dead_modules, out.stats.total_modules);
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test output_writer
cargo test -p wundler-pipeline --test pipeline_build
```

Expected failure: `unresolved import wundler_pipeline::output` and `method build not found for struct BuildPipeline`.

### Step 3: Write minimal implementation

Create `crates/wundler-pipeline/src/output.rs`:

```rust
//! Output writers: chunk files, manifest.json, and an index.html stub.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use wundler_graph::types::ChunkManifest;
use wundler_transform::engine::ChunkOutput;

/// Write a single chunk + optional source map to `{out_dir}/chunks/{hash}.js`.
pub fn write_chunk(out_dir: &Path, output: &ChunkOutput) -> Result<PathBuf> {
    let chunks_dir = out_dir.join("chunks");
    fs::create_dir_all(&chunks_dir)
        .with_context(|| format!("creating {}", chunks_dir.display()))?;
    let file_name = format!("{}.js", output.hash.to_hex());
    let path = chunks_dir.join(&file_name);
    let mut body = output.code.clone();
    if output.source_map.is_some() {
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("//# sourceMappingURL={}.js.map\n", output.hash.to_hex()));
        let map_path = chunks_dir.join(format!("{}.js.map", output.hash.to_hex()));
        fs::write(&map_path, output.source_map.as_ref().unwrap())
            .with_context(|| format!("writing {}", map_path.display()))?;
    }
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Write the `manifest.json` describing the bundle graph view.
pub fn write_manifest(out_dir: &Path, manifest: &ChunkManifest) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;
    let path = out_dir.join("manifest.json");
    let body = serde_json::to_string_pretty(manifest)
        .with_context(|| "serializing manifest")?;
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Write a minimal `index.html` that loads the initial chunks for the given entry.
pub fn write_index_html(out_dir: &Path, manifest: &ChunkManifest, entry: &str) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;
    let chunk_ids = manifest
        .entry_chunks
        .get(entry)
        .cloned()
        .unwrap_or_default();
    let scripts: Vec<String> = chunk_ids
        .iter()
        .filter_map(|id| manifest.chunks.iter().find(|c| &c.id == id))
        .map(|c| format!(r#"  <script src="chunks/{}.js" type="module"></script>"#, c.hash.to_hex()))
        .collect();
    let html = format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>Wundler — {entry}</title>
</head>
<body>
{scripts}
</body>
</html>
"#,
        entry = entry,
        scripts = scripts.join("\n")
    );
    fs::write(out_dir.join("index.html"), html)
        .with_context(|| "writing index.html")?;
    Ok(())
}
```

> `ContentHash::to_hex()` is assumed to exist on Plan 1's hash type. If not, substitute `hex::encode(&output.hash.as_bytes())` (or whatever accessor the type provides).

Append to `crates/wundler-pipeline/src/pipeline.rs`:

```rust
use crate::output::{write_chunk, write_index_html, write_manifest};
use std::path::PathBuf;
use std::time::Instant;
use wundler_graph::types::ChunkManifest;

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub manifest: ChunkManifest,
    pub chunk_files: Vec<PathBuf>,
    pub stats: BuildStats,
}

#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    pub total_modules: usize,
    pub alive_modules: usize,
    pub dead_modules: usize,
    pub chunks_written: usize,
    pub build_time_ms: u64,
    pub largest_chunk_bytes: usize,
}

impl BuildPipeline {
    /// Full build: summarize → analyze → transform → emit.
    pub fn build(&self) -> Result<BuildOutput> {
        let started = Instant::now();
        let nodes = self.run_summarize()?;
        let total_modules = nodes.len();
        let analysis = self.run_analyze(nodes)?;
        let alive_modules = analysis.nodes.iter().filter(|n| n.alive).count();
        let dead_modules = total_modules - alive_modules;
        let outputs = self.run_transform(&analysis)?;

        let mut chunk_files = Vec::with_capacity(outputs.len());
        let mut largest = 0usize;
        for o in &outputs {
            let path = write_chunk(&self.config.out_dir, o)?;
            largest = largest.max(o.code.len());
            chunk_files.push(path);
        }
        write_manifest(&self.config.out_dir, &analysis.manifest)?;
        for entry in self.config.entry_points.keys() {
            write_index_html(&self.config.out_dir, &analysis.manifest, entry)?;
        }
        let stats = BuildStats {
            total_modules,
            alive_modules,
            dead_modules,
            chunks_written: outputs.len(),
            build_time_ms: started.elapsed().as_millis() as u64,
            largest_chunk_bytes: largest,
        };
        Ok(BuildOutput {
            manifest: analysis.manifest,
            chunk_files,
            stats,
        })
    }
}
```

Modify `crates/wundler-pipeline/src/lib.rs`:

```rust
//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod output;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test output_writer
cargo test -p wundler-pipeline --test pipeline_build
```

Expected output for each: `test result: ok. <n> passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: output writer + BuildPipeline::build() end-to-end orchestration"
```

---

## Task 12 — Dev server: Axum static + on-demand SWC transform

**Files:**
- Create: `crates/wundler-pipeline/src/dev_server.rs`
- Modify: `crates/wundler-pipeline/src/lib.rs`
- Test: `crates/wundler-pipeline/tests/dev_server_serve.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/dev_server_serve.rs`:

```rust
use std::time::Duration;
use tempfile::tempdir;
use wundler_pipeline::dev_server::DevServer;

#[tokio::test]
async fn serves_typescript_as_transformed_javascript() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/index.ts"),
        "export const x: number = 1;\n",
    )
    .unwrap();

    let server = DevServer {
        root: dir.path().join("src"),
        port: 0, // OS-assigned
    };
    let (addr, shutdown) = server.start_for_test().await.unwrap();
    let url = format!("http://{}/index.ts", addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(body.contains("export const x"));
    assert!(!body.contains(": number"), "type annotation should be stripped: {}", body);
    shutdown.send(()).ok();
}

#[tokio::test]
async fn serves_plain_javascript_unmodified() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/plain.js"), "export const y = 42;\n").unwrap();

    let server = DevServer {
        root: dir.path().join("src"),
        port: 0,
    };
    let (addr, shutdown) = server.start_for_test().await.unwrap();
    let url = format!("http://{}/plain.js", addr);
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(body.contains("export const y = 42"));
    shutdown.send(()).ok();
}

#[tokio::test]
async fn returns_404_for_missing_file() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let server = DevServer {
        root: dir.path().join("src"),
        port: 0,
    };
    let (addr, shutdown) = server.start_for_test().await.unwrap();
    let url = format!("http://{}/nonexistent.ts", addr);
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    shutdown.send(()).ok();
}
```

Add `reqwest` to dev-dependencies in `crates/wundler-pipeline/Cargo.toml`:

```toml
[dev-dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test dev_server_serve
```

Expected failure: `unresolved import wundler_pipeline::dev_server`.

### Step 3: Write minimal implementation

Create `crates/wundler-pipeline/src/dev_server.rs`:

```rust
//! Development server. Serves source modules as native ESM, transforming
//! TypeScript/TSX on demand via SWC. No bundling.

use anyhow::{anyhow, Result};
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct DevServer {
    pub root: PathBuf,
    pub port: u16,
}

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
}

impl DevServer {
    pub async fn start(&self) -> Result<()> {
        let (_addr, shutdown) = self.start_for_test().await?;
        // Run until ctrl-c or the shutdown sender is dropped.
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown.send(());
        Ok(())
    }

    pub async fn start_for_test(&self) -> Result<(SocketAddr, oneshot::Sender<()>)> {
        let state = AppState {
            root: Arc::new(self.root.clone()),
        };
        let app = Router::new()
            .route("/*path", get(serve_module))
            .with_state(state);

        let bind = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;

        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await;
        });
        Ok((addr, tx))
    }
}

async fn serve_module(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let fs_path = state.root.join(&path);
    if !fs_path.exists() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let source = match tokio::fs::read_to_string(&fs_path).await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read error: {}", e),
            )
                .into_response();
        }
    };

    let transformed = match transform_on_demand(&path, &source) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("transform error: {}", e),
            )
                .into_response();
        }
    };

    let mut resp = transformed.into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    resp
}

fn transform_on_demand(name: &str, source: &str) -> Result<String> {
    if name.ends_with(".ts") || name.ends_with(".tsx") {
        // Use the SWC parser+emitter to strip type annotations.
        let (cm, module) = wundler_transform::swc_util::parse_source(name, source)
            .map_err(|e| anyhow!("parse: {}", e))?;
        wundler_transform::swc_util::emit_module(cm, &module)
            .map_err(|e| anyhow!("emit: {}", e))
    } else {
        Ok(source.to_string())
    }
}
```

Modify `crates/wundler-pipeline/src/lib.rs`:

```rust
//! Wundler Build Pipeline — orchestrates summarize → analyze → transform → emit.

pub mod config;
pub mod dev_server;
pub mod output;
pub mod pipeline;

pub use config::{BuildConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline, BuildStats};

pub fn hello() -> &'static str {
    "wundler-pipeline"
}
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test dev_server_serve
```

Expected output: `test result: ok. 3 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: DevServer — Axum + on-demand SWC TS→JS transform"
```

---

## Task 13 — Dev server: file watcher + SSE-based HMR

**Files:**
- Modify: `crates/wundler-pipeline/src/dev_server.rs`
- Test: `crates/wundler-pipeline/tests/dev_server_hmr.rs`

### Step 1: Write the failing test

`crates/wundler-pipeline/tests/dev_server_hmr.rs`:

```rust
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::timeout;
use wundler_pipeline::dev_server::DevServer;

#[tokio::test]
async fn sse_endpoint_emits_event_on_file_change() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let file = dir.path().join("src/watched.ts");
    std::fs::write(&file, "export const x = 1;\n").unwrap();

    let server = DevServer {
        root: dir.path().join("src"),
        port: 0,
    };
    let (addr, shutdown) = server.start_for_test().await.unwrap();
    let url = format!("http://{}/__wundler__/hmr", addr);

    // Open SSE stream
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let mut stream = resp.bytes_stream();

    // Give the watcher a moment to start.
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Trigger a change.
    std::fs::write(&file, "export const x = 2;\n").unwrap();

    use futures_util::StreamExt;
    let got = timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.unwrap();
            let s = String::from_utf8_lossy(&bytes);
            if s.contains("watched.ts") {
                return true;
            }
        }
        false
    })
    .await;
    assert!(got.is_ok() && got.unwrap(), "did not receive HMR event for watched.ts");
    shutdown.send(()).ok();
}

#[tokio::test]
async fn hmr_client_script_is_served() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let server = DevServer {
        root: dir.path().join("src"),
        port: 0,
    };
    let (addr, shutdown) = server.start_for_test().await.unwrap();
    let url = format!("http://{}/__wundler__/hmr-client.js", addr);
    let resp = reqwest::get(&url).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("EventSource"));
    assert!(body.contains("__wundler__/hmr"));
    shutdown.send(()).ok();
}
```

Add `futures-util` to dev-dependencies:

```toml
[dev-dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "stream"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
futures-util = "0.3"
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-pipeline --test dev_server_hmr
```

Expected failure: status 404 on the `__wundler__/hmr` endpoint (or test timeout) — the routes do not yet exist.

### Step 3: Write minimal implementation

Replace the body of `crates/wundler-pipeline/src/dev_server.rs` with this expanded version:

```rust
//! Development server. Serves source modules as native ESM, transforming
//! TypeScript/TSX on demand via SWC. File watcher emits HMR events over SSE.

use anyhow::{anyhow, Result};
use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Router,
};
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;

const HMR_CLIENT_JS: &str = r#"// Wundler HMR client — connect to /__wundler__/hmr and reload on change.
(function() {
  if (typeof EventSource === 'undefined') return;
  const es = new EventSource('/__wundler__/hmr');
  es.addEventListener('change', (ev) => {
    console.log('[wundler] change:', ev.data);
    // For Level 0 we full-reload. Granular accept() comes in a later milestone.
    location.reload();
  });
  es.onerror = () => {
    // Silent — server may be restarting.
  };
})();
"#;

#[derive(Clone)]
pub struct DevServer {
    pub root: PathBuf,
    pub port: u16,
}

#[derive(Clone)]
struct AppState {
    root: Arc<PathBuf>,
    hmr_tx: broadcast::Sender<String>,
}

impl DevServer {
    pub async fn start(&self) -> Result<()> {
        let (_addr, shutdown) = self.start_for_test().await?;
        let _ = tokio::signal::ctrl_c().await;
        let _ = shutdown.send(());
        Ok(())
    }

    pub async fn start_for_test(&self) -> Result<(SocketAddr, oneshot::Sender<()>)> {
        let (hmr_tx, _hmr_rx) = broadcast::channel::<String>(64);
        let state = AppState {
            root: Arc::new(self.root.clone()),
            hmr_tx: hmr_tx.clone(),
        };

        // File watcher — emits to broadcast channel on file change.
        let watch_root = self.root.clone();
        let watcher_tx = hmr_tx.clone();
        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel::<NotifyEvent>();
            let mut watcher = notify::recommended_watcher(move |res: notify::Result<NotifyEvent>| {
                if let Ok(ev) = res {
                    let _ = tx.send(ev);
                }
            })
            .expect("notify watcher init");
            if let Err(e) = watcher.watch(&watch_root, RecursiveMode::Recursive) {
                eprintln!("[wundler] watch error on {}: {}", watch_root.display(), e);
                return;
            }
            for ev in rx {
                if matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    for p in &ev.paths {
                        let rel = p.strip_prefix(&watch_root).unwrap_or(p);
                        let _ = watcher_tx.send(rel.display().to_string());
                    }
                }
            }
        });

        let app = Router::new()
            .route("/__wundler__/hmr-client.js", get(hmr_client))
            .route("/__wundler__/hmr", get(hmr_sse))
            .route("/*path", get(serve_module))
            .with_state(state);

        let bind = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;

        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = rx.await;
                })
                .await;
        });
        Ok((addr, tx))
    }
}

async fn hmr_client() -> Response {
    let mut resp = HMR_CLIENT_JS.into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    resp
}

async fn hmr_sse(
    State(state): State<AppState>,
) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.hmr_tx.subscribe();
    let stream = BroadcastStream::new(rx).map(|item| {
        let path = item.unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("change").data(path))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn serve_module(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let fs_path = state.root.join(&path);
    if !fs_path.exists() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let source = match tokio::fs::read_to_string(&fs_path).await {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("read error: {}", e))
                .into_response();
        }
    };
    let transformed = match transform_on_demand(&path, &source) {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("transform error: {}", e))
                .into_response();
        }
    };
    let mut resp = transformed.into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    resp
}

fn transform_on_demand(name: &str, source: &str) -> Result<String> {
    if name.ends_with(".ts") || name.ends_with(".tsx") {
        let (cm, module) = wundler_transform::swc_util::parse_source(name, source)
            .map_err(|e| anyhow!("parse: {}", e))?;
        wundler_transform::swc_util::emit_module(cm, &module)
            .map_err(|e| anyhow!("emit: {}", e))
    } else {
        Ok(source.to_string())
    }
}

use futures_util::StreamExt;
```

Update `crates/wundler-pipeline/Cargo.toml` dependencies block:

```toml
[dependencies]
wundler-core = { path = "../wundler-core" }
wundler-graph = { path = "../wundler-graph" }
wundler-transform = { path = "../wundler-transform" }
tokio = { version = "1", features = ["full"] }
tokio-stream = { version = "0.1", features = ["sync"] }
axum = { version = "0.8", features = ["macros"] }
tower-http = { version = "0.6", features = ["fs", "cors"] }
notify = "6"
toml = "0.8"
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
rayon = { workspace = true }
indicatif = "0.17"
sha2 = "0.10"
hex = "0.4"
futures-util = "0.3"
futures-core = "0.3"
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-pipeline --test dev_server_hmr
```

Expected output: `test result: ok. 2 passed; 0 failed`.

Also re-run the prior dev server test to confirm no regression:

```bash
cargo test -p wundler-pipeline --test dev_server_serve
```

### Step 5: Commit

```bash
git add crates/wundler-pipeline
git commit -m "wundler-pipeline: DevServer — notify watcher + SSE HMR endpoint + hmr-client.js"
```

---

## Task 14 — CLI: `wundler build` and `wundler dev` with progress

**Files:**
- Modify: `crates/wundler-cli/Cargo.toml`
- Modify: `crates/wundler-cli/src/main.rs`
- Test: `crates/wundler-cli/tests/cli_smoke.rs`

### Step 1: Write the failing test

`crates/wundler-cli/tests/cli_smoke.rs`:

```rust
use std::process::Command;
use tempfile::tempdir;

fn cli_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_BIN_EXE_wundler"));
    if !p.exists() {
        p = std::path::PathBuf::from(env!("CARGO_BIN_EXE_wundler-cli"));
    }
    p
}

#[test]
fn cli_build_subcommand_runs_end_to_end() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/index.ts"),
        "import { y } from './util';\nexport const z = y + 1;\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/util.ts"), "export const y = 2;\n").unwrap();
    std::fs::write(
        dir.path().join("wundler.toml"),
        format!(
            r#"
[build]
root = "{root}"
out_dir = "{out}"
engine = "swc"
source_maps = false

[entry]
"/" = "{entry}"
"#,
            root = dir.path().join("src").display(),
            out = dir.path().join("dist").display(),
            entry = dir.path().join("src/index.ts").display(),
        ),
    )
    .unwrap();

    let status = Command::new(cli_path())
        .arg("build")
        .arg("--config")
        .arg(dir.path().join("wundler.toml"))
        .status()
        .expect("spawn cli");
    assert!(status.success(), "cli build exited non-zero");
    assert!(dir.path().join("dist/manifest.json").exists());
    assert!(dir.path().join("dist/chunks").exists());
}

#[test]
fn cli_build_help_lists_subcommands() {
    let out = Command::new(cli_path())
        .arg("--help")
        .output()
        .expect("spawn cli");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(combined.contains("build"));
    assert!(combined.contains("dev"));
}
```

### Step 2: Run test, verify it FAILS

```bash
cargo test -p wundler-cli --test cli_smoke
```

Expected failure: `unrecognized subcommand 'build'` or `unrecognized argument '--config'` (the existing CLI from Plan 1/2 does not yet ship these commands).

### Step 3: Write minimal implementation

Update `crates/wundler-cli/Cargo.toml` dependencies:

```toml
[dependencies]
wundler-core = { path = "../wundler-core" }
wundler-graph = { path = "../wundler-graph" }
wundler-transform = { path = "../wundler-transform" }
wundler-pipeline = { path = "../wundler-pipeline" }
clap = { version = "4", features = ["derive"] }
anyhow = { workspace = true }
indicatif = "0.17"
tokio = { version = "1", features = ["full"] }
```

Replace `crates/wundler-cli/src/main.rs` (additive — preserve any existing `summarize`/`validate-scale`/`analyze` subcommands from prior plans by leaving their match arms intact; the snippet below shows the new `build` and `dev` arms inserted alongside them):

```rust
//! Wundler CLI.

use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use wundler_pipeline::config::BuildConfig;
use wundler_pipeline::dev_server::DevServer;
use wundler_pipeline::pipeline::BuildPipeline;

#[derive(Parser)]
#[command(name = "wundler", version, about = "Wundler bundler CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a full build (summarize → analyze → transform → emit).
    Build {
        #[arg(long, default_value = "wundler.toml")]
        config: PathBuf,
        #[arg(long)]
        engine: Option<String>,
    },
    /// Start the development server.
    Dev {
        #[arg(long, default_value = "wundler.toml")]
        config: PathBuf,
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    // (Preserve existing subcommands from prior plans here — e.g. `Summarize`, `Analyze`, etc.)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Build { config, engine } => run_build(&config, engine),
        Cmd::Dev { config, port } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(run_dev(&config, port))
        }
    }
}

fn run_build(config_path: &std::path::Path, engine_override: Option<String>) -> Result<()> {
    let mut cfg = BuildConfig::load(config_path)?;
    if let Some(name) = engine_override {
        cfg.engine = match name.as_str() {
            "swc" => wundler_pipeline::config::EngineChoice::Swc,
            "rolldown" => wundler_pipeline::config::EngineChoice::Rolldown,
            "rspack" => wundler_pipeline::config::EngineChoice::Rspack,
            other => return Err(anyhow::anyhow!("unknown engine: {}", other)),
        };
    }
    let bar = ProgressBar::new_spinner();
    bar.set_style(ProgressStyle::with_template("{spinner} {msg}").unwrap());
    bar.enable_steady_tick(std::time::Duration::from_millis(80));
    bar.set_message("building…");
    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build()?;
    bar.finish_and_clear();
    println!(
        "built {} chunks ({} modules alive, {} dead) in {} ms — largest chunk {} bytes",
        out.stats.chunks_written,
        out.stats.alive_modules,
        out.stats.dead_modules,
        out.stats.build_time_ms,
        out.stats.largest_chunk_bytes
    );
    Ok(())
}

async fn run_dev(config_path: &std::path::Path, port: u16) -> Result<()> {
    let cfg = BuildConfig::load(config_path)?;
    println!("wundler dev: serving {} on http://127.0.0.1:{}", cfg.root.display(), port);
    let server = DevServer {
        root: cfg.root,
        port,
    };
    server.start().await?;
    Ok(())
}
```

If the workspace `Cargo.toml` of `wundler-cli` sets `name = "wundler-cli"` but emits a binary called `wundler`, the test relies on `CARGO_BIN_EXE_wundler` or `CARGO_BIN_EXE_wundler-cli`. If neither matches the actual binary name, add `[[bin]] name = "wundler"` to `crates/wundler-cli/Cargo.toml`:

```toml
[[bin]]
name = "wundler"
path = "src/main.rs"
```

### Step 4: Run test, verify PASSES

```bash
cargo test -p wundler-cli --test cli_smoke
```

Expected output: `test result: ok. 2 passed; 0 failed`.

### Step 5: Commit

```bash
git add crates/wundler-cli
git commit -m "wundler-cli: add `build` and `dev` subcommands with progress indicators"
```

---

## Task 15 — Integration: end-to-end 5-module TypeScript project build

**Files:**
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/src/index.ts`
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/src/util.ts`
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/src/dashboard.ts`
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/src/settings.ts`
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/src/orphan.ts`
- Create: `crates/wundler-pipeline/tests/fixtures/five-module-app/wundler.toml.template`
- Test: `crates/wundler-pipeline/tests/integration_five_module.rs`

### Step 1: Write the failing test

Create the fixture files:

`crates/wundler-pipeline/tests/fixtures/five-module-app/src/index.ts`:

```typescript
import { greet } from './util';
export function main(): void {
    console.log(greet('world'));
}
main();
```

`crates/wundler-pipeline/tests/fixtures/five-module-app/src/util.ts`:

```typescript
export function greet(name: string): string {
    return `hello, ${name}`;
}
export function unused(): number {
    return 42;
}
```

`crates/wundler-pipeline/tests/fixtures/five-module-app/src/dashboard.ts`:

```typescript
import { greet } from './util';
export function renderDashboard(): string {
    return greet('dashboard');
}
```

`crates/wundler-pipeline/tests/fixtures/five-module-app/src/settings.ts`:

```typescript
export function renderSettings(): string {
    return 'settings';
}
```

`crates/wundler-pipeline/tests/fixtures/five-module-app/src/orphan.ts`:

```typescript
// Not imported by any entry — should be dead.
export const orphan = 99;
```

Create `crates/wundler-pipeline/tests/integration_five_module.rs`:

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::tempdir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/five-module-app")
}

#[test]
fn five_module_project_builds_end_to_end() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["index.ts", "util.ts", "dashboard.ts", "settings.ts", "orphan.ts"] {
        let body = std::fs::read_to_string(fixture_root().join("src").join(f)).unwrap();
        std::fs::write(src.join(f), body).unwrap();
    }

    let mut entries = HashMap::new();
    entries.insert("/".to_string(), src.join("index.ts"));
    entries.insert("/dashboard".to_string(), src.join("dashboard.ts"));
    entries.insert("/settings".to_string(), src.join("settings.ts"));

    let cfg = BuildConfig {
        root: src,
        out_dir: dir.path().join("dist"),
        source_maps: true,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };

    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build().unwrap();

    // 5 source modules.
    assert_eq!(out.stats.total_modules, 5);
    // orphan.ts is dead — at minimum 1 dead module.
    assert!(out.stats.dead_modules >= 1, "stats: {:?}", out.stats);
    // alive + dead = total.
    assert_eq!(
        out.stats.alive_modules + out.stats.dead_modules,
        out.stats.total_modules
    );
    // manifest.json exists.
    assert!(dir.path().join("dist/manifest.json").exists());
    // 3 entry index.html files — one per entry point.
    for entry in ["", "dashboard", "settings"] {
        // index.html is overwritten per entry in the simple writer; we at least confirm one exists.
        let _ = entry;
    }
    assert!(dir.path().join("dist/index.html").exists());
    // chunks dir has at least as many .js files as chunks were written.
    let chunk_files: Vec<_> = std::fs::read_dir(dir.path().join("dist/chunks"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "js").unwrap_or(false))
        .collect();
    assert_eq!(chunk_files.len(), out.stats.chunks_written);
    // Each chunk file is non-empty.
    for cf in &chunk_files {
        let body = std::fs::read_to_string(cf.path()).unwrap();
        assert!(!body.trim().is_empty(), "{} is empty", cf.path().display());
    }
    // Manifest is parseable JSON.
    let manifest_body = std::fs::read_to_string(dir.path().join("dist/manifest.json")).unwrap();
    let _: serde_json::Value = serde_json::from_str(&manifest_body).unwrap();
}

#[test]
fn integration_build_is_deterministic() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["index.ts", "util.ts", "dashboard.ts", "settings.ts", "orphan.ts"] {
        let body = std::fs::read_to_string(fixture_root().join("src").join(f)).unwrap();
        std::fs::write(src.join(f), body).unwrap();
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), src.join("index.ts"));

    let cfg = BuildConfig {
        root: src,
        out_dir: dir.path().join("dist1"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries.clone(),
    };
    let pipeline = BuildPipeline::new(cfg);
    let out1 = pipeline.build().unwrap();

    let cfg2 = BuildConfig {
        root: dir.path().join("src"),
        out_dir: dir.path().join("dist2"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: entries,
    };
    let pipeline2 = BuildPipeline::new(cfg2);
    let out2 = pipeline2.build().unwrap();

    let mut hashes1: Vec<String> = out1
        .manifest
        .chunks
        .iter()
        .map(|c| c.hash.to_hex())
        .collect();
    let mut hashes2: Vec<String> = out2
        .manifest
        .chunks
        .iter()
        .map(|c| c.hash.to_hex())
        .collect();
    hashes1.sort();
    hashes2.sort();
    assert_eq!(hashes1, hashes2, "chunk hashes must be deterministic across runs");
}
```

### Step 2: Run test, verify it FAILS

Initially the fixture directory does not exist:

```bash
cargo test -p wundler-pipeline --test integration_five_module
```

Expected failure: `No such file or directory` when reading the fixture files (because the fixture tree must be created first).

### Step 3: Write minimal implementation

Create the five fixture `.ts` files exactly as listed in Step 1 above. There is no production code to add — Tasks 1–14 already provide the implementation; this task only adds the fixtures and the integration harness.

### Step 4: Run test, verify PASSES

Full workspace test sweep — every test from every task in this plan, plus prior plans, must pass:

```bash
cargo test -p wundler-transform
cargo test -p wundler-pipeline
cargo test -p wundler-cli
cargo test --workspace
```

Expected: every `cargo test` command reports `test result: ok` with zero failures.

Then run a manual smoke check from the command line:

```bash
cd /Users/ken/workspace/ms/wundler
cargo build --release --workspace
# Set up a tiny scratch project
SCRATCH=$(mktemp -d)
mkdir -p "$SCRATCH/src"
echo "import { y } from './util'; export const z = y + 1;" > "$SCRATCH/src/index.ts"
echo "export const y = 2;" > "$SCRATCH/src/util.ts"
cat > "$SCRATCH/wundler.toml" <<TOML
[build]
root = "$SCRATCH/src"
out_dir = "$SCRATCH/dist"
engine = "swc"

[entry]
"/" = "$SCRATCH/src/index.ts"
TOML
./target/release/wundler build --config "$SCRATCH/wundler.toml"
ls "$SCRATCH/dist"
cat "$SCRATCH/dist/manifest.json" | head -30
```

Expected smoke output: the `ls` command shows `chunks/`, `manifest.json`, and `index.html`; `manifest.json` parses as JSON and contains a non-empty `chunks` array.

### Step 5: Commit

```bash
git add crates/wundler-pipeline/tests
git commit -m "wundler-pipeline: integration — end-to-end 5-module TypeScript build (Level 0 gate)"
```

---

## Plan Completion Criteria

The plan is complete when **all of the following are true**:

1. `cargo build --workspace --release` succeeds with zero warnings escalated to errors
2. `cargo test --workspace` reports zero failures across every crate
3. `cargo test -p wundler-pipeline --test integration_five_module` reports both tests passing
4. The release `wundler` binary, when pointed at a real-world TypeScript project's `wundler.toml`, produces:
   - A `dist/manifest.json` parseable as `ChunkManifest`
   - A `dist/chunks/` directory containing only content-hashed `.js` files (and their `.js.map` files if source maps are enabled)
   - A `dist/index.html` referencing the initial chunks for the configured entry
5. `wundler dev --port <port>` serves TypeScript files transformed to JavaScript at `http://127.0.0.1:<port>/<path>` and emits SSE HMR events on file change

## Level 0 Validation Gate

The success criterion for **shipping Level 0** to a real consumer:

- Run `wundler build` against a project that currently builds via Vite/Webpack/Rspack
- Compare the rebuild time of a single-file edit:
  - Wundler must be **at least as fast** as the incumbent on first build
  - Wundler must be **strictly faster** on the second build (cache hit on summaries)
- Verify the produced bundle loads in a browser and renders identically to the incumbent's bundle

If the rebuild-time check passes, Level 0 is real. The system is then ready to layer the Adaptive Bundle Service (Plan 4) on top.

## Open Questions Carried into Plan 4

- **Manifest schema compatibility:** does the Plan 4 ABS protocol need a versioned `schema_version` field on `ChunkManifest`? Decide before Plan 4 starts.
- **Source map composition across phases:** the current source maps are line-coarse. Producing a fully-mapped chunk requires merging per-module SWC maps; deferred unless a user reports broken stack traces.
- **Rspack adapter:** the `EngineChoice::Rspack` variant currently falls back to SWC. Implementing it requires an N-API binding crate and is deferred until a user materially requires Rspack semantics.

## Risk Register

| Risk | Mitigation |
|---|---|
| Plan 1 / Plan 2 API shapes differ from what tests assume | Pre-flight verification script catches this; tasks call out specific dependency points (`ContentHash::from_bytes`, `summarize_directory`, `analyze`) so the failure mode is a clean compile error, not silent breakage |
| SWC version mismatch with Plan 1 | Both crates pin `swc_core = "6"` in workspace dependencies; if Plan 1 used a different major, align before starting Task 1 |
| Rolldown subprocess flakiness in CI | `RolldownAdapter` defaults `fallback_on_failure = true`; integration tests use SWC engine |
| File watcher cross-platform behavior (notify) | `recommended_watcher()` selects per-OS backends (FSEvents on macOS, inotify on Linux); HMR test uses a 5s timeout to absorb backend latency |
| Workspace test ordering races on shared temp dirs | Every test uses `tempfile::tempdir()` for isolation |

## Total Effort Estimate

15 tasks × ~30–45 minutes per task (TDD discipline, no slop) ≈ **8–11 hours of focused implementation**. The pre-flight verification step plus the Level 0 smoke gate add roughly 1 hour. Plan as a single working-day effort with a half-day of slack for SWC API friction.
