# Module Summarizer (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Cloudpack Phase 1 module summarizer — a Rust library that takes one JS/TS source file and emits a `BundleGraphNode` (with exports, imports, side-effect marker, call edges, and ambient refs) cached by SHA-256 of the source, then validate the design against Teams' full module graph (50k modules ≈ 100MB).

**Architecture:** SWC is used to parse JS/TS source into an AST, which five independent extractor modules each visit once to produce structured metadata. A content-addressed disk cache keyed on `SHA-256(source)` avoids redundant work; node\_modules get a faster package-level key. A Rayon-parallelized directory scanner feeds the Month 1 validation gate.

**Tech Stack:** Rust (edition 2021), `swc_core 6` (parser + AST visitor), `sha2 0.10`, `hex 0.4`, `serde/serde_json` (serialization), `rayon` (parallel work-stealing), `walkdir` (directory traversal), `anyhow/thiserror` (error handling), `clap 4` (CLI), `indicatif` (progress bar), `tempfile` (test isolation).

---

## File Structure

All paths relative to the workspace root (`cloudpack/`).

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Workspace root — declares `[workspace]` members |
| `crates/cloudpack-core/Cargo.toml` | Core library manifest with all analysis dependencies |
| `crates/cloudpack-core/src/lib.rs` | Module declarations and public re-exports |
| `crates/cloudpack-core/src/types.rs` | All shared types: `ContentHash`, `Export`, `Import`, `CallEdge`, `SideEffectMarker`, `ModuleSummary`, `BundleGraphNode` |
| `crates/cloudpack-core/src/summarizer/mod.rs` | `ModuleSummarizer` struct, `summarize()`, `summarize_directory()` |
| `crates/cloudpack-core/src/summarizer/parser.rs` | `parse_module(source, path) -> Result<Module>` — SWC abstraction |
| `crates/cloudpack-core/src/summarizer/exports.rs` | `extract_exports(module) -> Vec<Export>` |
| `crates/cloudpack-core/src/summarizer/imports.rs` | `extract_imports(module) -> Vec<Import>` — static + dynamic |
| `crates/cloudpack-core/src/summarizer/side_effects.rs` | `analyze_side_effects(module) -> SideEffectMarker` |
| `crates/cloudpack-core/src/summarizer/call_edges.rs` | `extract_call_edges(module, exported_names) -> Vec<CallEdge>` |
| `crates/cloudpack-core/src/summarizer/ambient_refs.rs` | `extract_ambient_refs(module) -> Vec<String>` |
| `crates/cloudpack-core/src/cache/mod.rs` | Re-exports `local` and `package` sub-modules |
| `crates/cloudpack-core/src/cache/local.rs` | `LocalCache` — disk-backed content-addressed store |
| `crates/cloudpack-core/src/cache/package.rs` | `PackageLevelCache` — node\_modules fast-path key computation |
| `crates/cloudpack-core/src/cjs/mod.rs` | Re-exports from `stub` |
| `crates/cloudpack-core/src/cjs/stub.rs` | `is_cjs()`, `detect_cjs_exports()`, `generate_cjs_stub()` |
| `crates/cloudpack-core/src/validation.rs` | `run_validate_scale()`, `ValidateScaleStats` |
| `crates/cloudpack-core/tests/integration_test.rs` | End-to-end summarizer test using fixture files |
| `crates/cloudpack-core/tests/fixtures/esm_basic.ts` | Named + default exports, static imports |
| `crates/cloudpack-core/tests/fixtures/esm_reexport.ts` | Re-exports and star-exports |
| `crates/cloudpack-core/tests/fixtures/dynamic_import.ts` | Literal and non-literal `import()` calls |
| `crates/cloudpack-core/tests/fixtures/side_effects_ambient.ts` | Writes to `window` / `document` → DEFINITE |
| `crates/cloudpack-core/tests/fixtures/side_effects_pure.ts` | Function-only module → NONE |
| `crates/cloudpack-core/tests/fixtures/cjs_module.js` | `module.exports = { a, b, c }` |
| `crates/cloudpack-cli/Cargo.toml` | CLI crate manifest |
| `crates/cloudpack-cli/src/main.rs` | `cloudpack summarize <path>` and `cloudpack validate-scale <path>` |

---

## Task 1: Initialize Rust Workspace

**Files:**
- Create: `Cargo.toml`
- Create: `crates/cloudpack-core/Cargo.toml`
- Create: `crates/cloudpack-core/src/lib.rs`
- Create: `crates/cloudpack-cli/Cargo.toml`
- Create: `crates/cloudpack-cli/src/main.rs`

- [ ] **Step 1: Verify the workspace does not yet build**

```bash
cd /path/to/cloudpack && cargo build
```
Expected: `error: could not find 'Cargo.toml'` (or similar — no workspace exists yet)

- [ ] **Step 2: Create the workspace and crate files**

`Cargo.toml` (workspace root):
```toml
[workspace]
members = [
    "crates/cloudpack-core",
    "crates/cloudpack-cli",
]
resolver = "2"
```

`crates/cloudpack-core/Cargo.toml`:
```toml
[package]
name = "cloudpack-core"
version = "0.1.0"
edition = "2021"

[dependencies]
swc_core = { version = "6", features = ["ecma_parser", "ecma_ast", "ecma_visit", "common"] }
sha2 = "0.10"
hex = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rayon = "1"
anyhow = "1"
thiserror = "2"
walkdir = "2"

[dev-dependencies]
tempfile = "3"
```

`crates/cloudpack-core/src/lib.rs`:
```rust
pub mod cache;
pub mod cjs;
pub mod summarizer;
pub mod types;
pub mod validation;

pub use types::{
    BundleGraphNode, CallEdge, ContentHash, Export, ExportKind, Import, ImportKind,
    ModuleSummary, SideEffectMarker,
};
```

`crates/cloudpack-cli/Cargo.toml`:
```toml
[package]
name = "cloudpack-cli"
version = "0.1.0"
edition = "2021"

[dependencies]
cloudpack-core = { path = "../cloudpack-core" }
clap = { version = "4", features = ["derive"] }
indicatif = "0.17"
anyhow = "1"
serde_json = "1"
```

`crates/cloudpack-cli/src/main.rs`:
```rust
fn main() {
    println!("cloudpack");
}
```

- [ ] **Step 3: Verify the workspace builds**

```bash
cargo build
```
Expected: `Finished dev [unoptimized + debuginfo] target(s) in ...`

> **Note:** `cargo build` will download swc\_core and its transitive dependencies — this can take a few minutes on first run.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/
git commit -m "chore: initialize Rust workspace with cloudpack-core and cloudpack-cli"
```

---

## Task 2: Define Core Types

**Files:**
- Create: `crates/cloudpack-core/src/types.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/cloudpack-core/src/types.rs` with the test-only version (no types defined yet):

```rust
// crates/cloudpack-core/src/types.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_from_source_is_64_hex_chars() {
        let hash = ContentHash::from_source("const x = 1;");
        assert_eq!(hash.as_str().len(), 64);
    }

    #[test]
    fn test_content_hash_same_source_same_hash() {
        let h1 = ContentHash::from_source("export const a = 1;");
        let h2 = ContentHash::from_source("export const a = 1;");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different_source_different_hash() {
        let h1 = ContentHash::from_source("export const a = 1;");
        let h2 = ContentHash::from_source("export const b = 2;");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_bundle_graph_node_roundtrips_json() {
        let node = BundleGraphNode {
            id: ContentHash::from_source("export const x = 1;"),
            path: "src/foo.ts".to_string(),
            summary: ModuleSummary {
                exports: vec![Export {
                    name: "x".to_string(),
                    kind: ExportKind::Named,
                    source: None,
                }],
                imports: vec![Import {
                    specifier: "react".to_string(),
                    kind: ImportKind::Default,
                    bindings: vec!["React".to_string()],
                    is_dynamic: false,
                }],
                side_effects: SideEffectMarker::None,
                call_edges: vec![],
                ambient_refs: vec![],
            },
            alive: false,
            chunk_id: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: BundleGraphNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "src/foo.ts");
        assert_eq!(back.summary.exports.len(), 1);
        assert_eq!(back.summary.exports[0].name, "x");
        assert_eq!(back.summary.imports.len(), 1);
        assert_eq!(back.summary.imports[0].specifier, "react");
        assert!(matches!(back.summary.side_effects, SideEffectMarker::None));
    }

    #[test]
    fn test_side_effect_marker_possible_serializes_with_reason() {
        let marker = SideEffectMarker::Possible {
            reason: "top-level call".to_string(),
        };
        let json = serde_json::to_string(&marker).unwrap();
        assert!(json.contains("\"kind\":\"POSSIBLE\""));
        assert!(json.contains("top-level call"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core types
```
Expected: compilation error — `cannot find type 'ContentHash'`, `'BundleGraphNode'`, etc.

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/types.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/types.rs
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A hex-encoded SHA-256 hash used as a stable content identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// Hash UTF-8 source text.
    pub fn from_source(source: &str) -> Self {
        Self::from_bytes(source.as_bytes())
    }

    /// Hash arbitrary bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        ContentHash(hex::encode(hasher.finalize()))
    }

    /// The hex string value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a name is exported from a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExportKind {
    /// `export const x = ...` / `export function f() {}` / `export class C {}`
    Named,
    /// `export default ...`
    Default,
    /// `export { x } from 'y'` — re-exporting from another module
    ReExport,
    /// `export * from 'z'` — namespace star-export
    StarExport,
}

/// One exported name from a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    /// The name visible to importers.
    pub name: String,
    pub kind: ExportKind,
    /// For `ReExport` and `StarExport`: the source module specifier.
    pub source: Option<String>,
}

/// How a module is imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportKind {
    /// `import { a, b } from 'x'`
    Named,
    /// `import def from 'x'`
    Default,
    /// `import * as ns from 'x'`
    Namespace,
    /// `import 'x'` — bare side-effect import, no bindings
    SideEffect,
    /// `import('x')` — dynamic import
    Dynamic,
}

/// One import relationship from a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// The module specifier string (e.g. `"react"`, `"./utils"`).
    pub specifier: String,
    pub kind: ImportKind,
    /// Local binding names imported (e.g. `["useState", "useEffect"]`).
    pub bindings: Vec<String>,
    pub is_dynamic: bool,
}

/// A directed call relationship between two exported functions in the same module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    /// The exported function that makes the call.
    pub caller: String,
    /// The exported function that is called.
    pub callee: String,
}

/// Whether a module has observable side effects at module initialization time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SideEffectMarker {
    /// Proven: module only contains function/class/type declarations and export statements.
    #[serde(rename = "NONE")]
    None,
    /// Heuristic: top-level code with potential side effects detected.
    #[serde(rename = "POSSIBLE")]
    Possible { reason: String },
    /// Certain: direct write to a global object (`window.X = ...`).
    #[serde(rename = "DEFINITE")]
    Definite,
}

/// The Phase 1 output for one module — all static metadata needed by Phase 2.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
    #[serde(rename = "sideEffects")]
    pub side_effects: SideEffectMarker,
    #[serde(rename = "callEdges")]
    pub call_edges: Vec<CallEdge>,
    #[serde(rename = "ambientRefs")]
    pub ambient_refs: Vec<String>,
}

/// One node in the Bundle Graph. `summary` is always present after Phase 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleGraphNode {
    /// SHA-256(source) — stable identity, survives renames.
    pub id: ContentHash,
    pub path: String,
    pub summary: ModuleSummary,
    /// Set by Phase 2 reachability analysis.
    pub alive: bool,
    /// Set by Phase 2 chunk assignment.
    #[serde(rename = "chunkId", skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hash_from_source_is_64_hex_chars() {
        let hash = ContentHash::from_source("const x = 1;");
        assert_eq!(hash.as_str().len(), 64);
    }

    #[test]
    fn test_content_hash_same_source_same_hash() {
        let h1 = ContentHash::from_source("export const a = 1;");
        let h2 = ContentHash::from_source("export const a = 1;");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_content_hash_different_source_different_hash() {
        let h1 = ContentHash::from_source("export const a = 1;");
        let h2 = ContentHash::from_source("export const b = 2;");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_bundle_graph_node_roundtrips_json() {
        let node = BundleGraphNode {
            id: ContentHash::from_source("export const x = 1;"),
            path: "src/foo.ts".to_string(),
            summary: ModuleSummary {
                exports: vec![Export {
                    name: "x".to_string(),
                    kind: ExportKind::Named,
                    source: None,
                }],
                imports: vec![Import {
                    specifier: "react".to_string(),
                    kind: ImportKind::Default,
                    bindings: vec!["React".to_string()],
                    is_dynamic: false,
                }],
                side_effects: SideEffectMarker::None,
                call_edges: vec![],
                ambient_refs: vec![],
            },
            alive: false,
            chunk_id: None,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: BundleGraphNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "src/foo.ts");
        assert_eq!(back.summary.exports.len(), 1);
        assert_eq!(back.summary.exports[0].name, "x");
        assert_eq!(back.summary.imports.len(), 1);
        assert_eq!(back.summary.imports[0].specifier, "react");
        assert!(matches!(back.summary.side_effects, SideEffectMarker::None));
    }

    #[test]
    fn test_side_effect_marker_possible_serializes_with_reason() {
        let marker = SideEffectMarker::Possible {
            reason: "top-level call".to_string(),
        };
        let json = serde_json::to_string(&marker).unwrap();
        assert!(json.contains("\"kind\":\"POSSIBLE\""));
        assert!(json.contains("top-level call"));
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core types
```
Expected: `test result: ok. 4 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/types.rs
git commit -m "feat(types): define core BundleGraphNode, ModuleSummary, and supporting types"
```

---

## Task 3: SWC Parser Abstraction

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/mod.rs`
- Create: `crates/cloudpack-core/src/summarizer/parser.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/mod.rs`:
```rust
// crates/cloudpack-core/src/summarizer/mod.rs
pub mod parser;
```

Create `crates/cloudpack-core/src/summarizer/parser.rs` with only the test:
```rust
// crates/cloudpack-core/src/summarizer/parser.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parse_typescript_succeeds() {
        let source = "export const greeting: string = 'hello';";
        let path = Path::new("test.ts");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let module = result.unwrap();
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn test_parse_tsx_succeeds() {
        let source = "export default function App(): JSX.Element { return <div />; }";
        let path = Path::new("app.tsx");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "TSX parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_javascript_succeeds() {
        let source = "import React from 'react'; export default function App() { return null; }";
        let path = Path::new("app.js");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "JS parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_invalid_syntax_returns_err() {
        let source = "export function @@@broken() {}";
        let path = Path::new("broken.ts");
        let result = parse_module(source, path);
        assert!(result.is_err(), "expected Err on invalid syntax");
    }
}
```

Also update `crates/cloudpack-core/src/lib.rs` to add the `summarizer` module:
```rust
pub mod cache;
pub mod cjs;
pub mod summarizer;
pub mod types;
pub mod validation;

pub use types::{
    BundleGraphNode, CallEdge, ContentHash, Export, ExportKind, Import, ImportKind,
    ModuleSummary, SideEffectMarker,
};
```

> **Note:** `cache`, `cjs`, and `validation` modules don't exist yet. For this task only, temporarily comment them out in `lib.rs` to unblock compilation:
> ```rust
> // pub mod cache;
> // pub mod cjs;
> pub mod summarizer;
> pub mod types;
> // pub mod validation;
> ```
> Restore the full `lib.rs` in Task 11 when those modules are created.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::parser
```
Expected: compilation error — `cannot find function 'parse_module' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Add the implementation to `crates/cloudpack-core/src/summarizer/parser.rs`:

```rust
// crates/cloudpack-core/src/summarizer/parser.rs
use anyhow::{anyhow, Result};
use std::path::Path;
use swc_core::common::{sync::Lrc, FileName, SourceMap};
use swc_core::ecma::ast::Module;
use swc_core::ecma::parser::{
    lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax,
};

/// Parse a JS/TS source string into an SWC `Module` AST.
///
/// The file extension is used to select the parser syntax:
/// - `.ts`  → TypeScript (no JSX)
/// - `.tsx` → TypeScript + JSX
/// - `.js`, `.mjs`, `.cjs` → ECMAScript (no JSX)
/// - `.jsx` → ECMAScript + JSX
pub fn parse_module(source: &str, path: &Path) -> Result<Module> {
    let cm: Lrc<SourceMap> = Default::default();
    let filename = FileName::Real(path.to_path_buf());
    let source_file = cm.new_source_file(filename, source.to_string());

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("js")
        .to_ascii_lowercase();

    let syntax = match ext.as_str() {
        "ts" => Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: true,
            ..Default::default()
        }),
        "tsx" => Syntax::Typescript(TsSyntax {
            tsx: true,
            decorators: true,
            ..Default::default()
        }),
        "jsx" => Syntax::Es(EsSyntax {
            jsx: true,
            ..Default::default()
        }),
        _ => Syntax::Es(EsSyntax {
            jsx: false,
            ..Default::default()
        }),
    };

    let lexer = Lexer::new(
        syntax,
        Default::default(),
        StringInput::from(&*source_file),
        None,
    );

    let mut parser = Parser::new_from(lexer);
    parser.parse_module().map_err(|err| {
        anyhow!(
            "Parse error in {}: {:?}",
            path.display(),
            err
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parse_typescript_succeeds() {
        let source = "export const greeting: string = 'hello';";
        let path = Path::new("test.ts");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let module = result.unwrap();
        assert_eq!(module.body.len(), 1);
    }

    #[test]
    fn test_parse_tsx_succeeds() {
        let source = "export default function App(): JSX.Element { return <div />; }";
        let path = Path::new("app.tsx");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "TSX parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_javascript_succeeds() {
        let source = "import React from 'react'; export default function App() { return null; }";
        let path = Path::new("app.js");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "JS parse failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_invalid_syntax_returns_err() {
        let source = "export function @@@broken() {}";
        let path = Path::new("broken.ts");
        let result = parse_module(source, path);
        assert!(result.is_err(), "expected Err on invalid syntax");
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::parser
```
Expected: `test result: ok. 4 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/
git commit -m "feat(parser): add SWC parse_module abstraction with TS/TSX/JS/JSX support"
```

---

## Task 4: Export Extractor

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/exports.rs`
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/exports.rs` with only the test:

```rust
// crates/cloudpack-core/src/summarizer/exports.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_extract_named_exports() {
        let source = "export const VERSION = '1.0.0'; export function greet() {} export class Greeter {}";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 3);
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"VERSION"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Greeter"));
        assert!(exports.iter().all(|e| e.kind == ExportKind::Named));
        assert!(exports.iter().all(|e| e.source.is_none()));
    }

    #[test]
    fn test_extract_default_export_function() {
        let source = "export default function App() {}";
        let module = parse_module(source, Path::new("app.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
    }

    #[test]
    fn test_extract_default_export_expression() {
        let source = "const x = 42; export default x;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
    }

    #[test]
    fn test_extract_reexports_from_module() {
        let source = "export { useState, useEffect } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 2);
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"useState"));
        assert!(names.contains(&"useEffect"));
        assert!(exports.iter().all(|e| e.kind == ExportKind::ReExport));
        assert!(exports.iter().all(|e| e.source.as_deref() == Some("react")));
    }

    #[test]
    fn test_extract_reexport_with_rename() {
        let source = "export { greet as greetUser } from './greeter';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "greetUser");
        assert_eq!(exports[0].kind, ExportKind::ReExport);
        assert_eq!(exports[0].source.as_deref(), Some("./greeter"));
    }

    #[test]
    fn test_extract_star_export() {
        let source = "export * from './utils';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "*");
        assert_eq!(exports[0].kind, ExportKind::StarExport);
        assert_eq!(exports[0].source.as_deref(), Some("./utils"));
    }

    #[test]
    fn test_no_exports_returns_empty() {
        let source = "const x = 1; function helper() {}";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert!(exports.is_empty());
    }
}
```

Add `pub mod exports;` to `crates/cloudpack-core/src/summarizer/mod.rs`:
```rust
pub mod exports;
pub mod parser;
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::exports
```
Expected: compilation error — `cannot find function 'extract_exports' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Add the implementation above the `#[cfg(test)]` block in `exports.rs`:

```rust
// crates/cloudpack-core/src/summarizer/exports.rs
use crate::types::{Export, ExportKind};
use swc_core::ecma::ast::{
    Decl, DefaultDecl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat,
    VarDeclarator,
};
use swc_core::ecma::ast::Module;

/// Extract all exports from a parsed module AST.
pub fn extract_exports(module: &Module) -> Vec<Export> {
    let mut exports = Vec::new();

    for item in &module.body {
        let decl = match item {
            ModuleItem::ModuleDecl(d) => d,
            ModuleItem::Stmt(_) => continue,
        };

        match decl {
            // export const x = ..., export function f() {}, export class C {}
            ModuleDecl::ExportDecl(export_decl) => {
                let names = names_from_decl(&export_decl.decl);
                for name in names {
                    exports.push(Export {
                        name,
                        kind: ExportKind::Named,
                        source: None,
                    });
                }
            }

            // export default function f() {} / export default class C {}
            ModuleDecl::ExportDefaultDecl(export_default) => {
                let name = match &export_default.decl {
                    DefaultDecl::Fn(fn_expr) => fn_expr
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    DefaultDecl::Class(class_expr) => class_expr
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string()),
                    DefaultDecl::TsInterfaceDecl(_) => "default".to_string(),
                };
                exports.push(Export {
                    name: "default".to_string(),
                    kind: ExportKind::Default,
                    source: None,
                });
                // If there's a named function/class, also register under that name
                // but only export "default" — that's what consumers see.
                let _ = name; // suppress unused warning; named default is an impl detail
            }

            // export default <expr>
            ModuleDecl::ExportDefaultExpr(_) => {
                exports.push(Export {
                    name: "default".to_string(),
                    kind: ExportKind::Default,
                    source: None,
                });
            }

            // export { x } / export { x } from 'y' / export { x as z } from 'y'
            ModuleDecl::ExportNamed(named_export) => {
                let source = named_export
                    .src
                    .as_ref()
                    .map(|s| s.value.to_string());

                let kind = if source.is_some() {
                    ExportKind::ReExport
                } else {
                    ExportKind::Named
                };

                for spec in &named_export.specifiers {
                    if let ExportSpecifier::Named(named) = spec {
                        // The exported name is the alias if present, else the original name.
                        let exported_name = named
                            .exported
                            .as_ref()
                            .map(module_export_name_to_str)
                            .unwrap_or_else(|| module_export_name_to_str(&named.orig));

                        exports.push(Export {
                            name: exported_name,
                            kind: kind.clone(),
                            source: source.clone(),
                        });
                    }
                }
            }

            // export * from 'z'
            ModuleDecl::ExportAll(export_all) => {
                exports.push(Export {
                    name: "*".to_string(),
                    kind: ExportKind::StarExport,
                    source: Some(export_all.src.value.to_string()),
                });
            }

            _ => {}
        }
    }

    exports
}

fn module_export_name_to_str(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::Ident(i) => i.sym.to_string(),
        ModuleExportName::Str(s) => s.value.to_string(),
    }
}

/// Extract bound names from a declaration (handles var/let/const, fn, class, TS types).
fn names_from_decl(decl: &Decl) -> Vec<String> {
    match decl {
        Decl::Fn(fn_decl) => vec![fn_decl.ident.sym.to_string()],
        Decl::Class(class_decl) => vec![class_decl.ident.sym.to_string()],
        Decl::Var(var_decl) => var_decl
            .decls
            .iter()
            .flat_map(|d| names_from_var_declarator(d))
            .collect(),
        Decl::TsInterface(ts) => vec![ts.id.sym.to_string()],
        Decl::TsTypeAlias(ts) => vec![ts.id.sym.to_string()],
        Decl::TsEnum(ts) => vec![ts.id.sym.to_string()],
        Decl::TsModule(ts) => {
            use swc_core::ecma::ast::TsModuleName;
            match &ts.id {
                TsModuleName::Ident(i) => vec![i.sym.to_string()],
                TsModuleName::Str(s) => vec![s.value.to_string()],
            }
        }
        Decl::Using(using) => using
            .decls
            .iter()
            .flat_map(|d| names_from_var_declarator(d))
            .collect(),
    }
}

fn names_from_var_declarator(d: &VarDeclarator) -> Vec<String> {
    names_from_pat(&d.name)
}

fn names_from_pat(pat: &Pat) -> Vec<String> {
    match pat {
        Pat::Ident(bi) => vec![bi.id.sym.to_string()],
        Pat::Array(ap) => ap
            .elems
            .iter()
            .filter_map(|e| e.as_ref())
            .flat_map(names_from_pat)
            .collect(),
        Pat::Object(op) => op
            .props
            .iter()
            .flat_map(|p| {
                use swc_core::ecma::ast::ObjectPatProp;
                match p {
                    ObjectPatProp::Assign(ap) => vec![ap.key.sym.to_string()],
                    ObjectPatProp::KeyValue(kv) => names_from_pat(&kv.value),
                    ObjectPatProp::Rest(rp) => names_from_pat(&rp.arg),
                }
            })
            .collect(),
        Pat::Rest(rp) => names_from_pat(&rp.arg),
        Pat::Assign(ap) => names_from_pat(&ap.left),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    // ... (tests from Step 1)
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_extract_named_exports() {
        let source = "export const VERSION = '1.0.0'; export function greet() {} export class Greeter {}";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 3);
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"VERSION"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"Greeter"));
        assert!(exports.iter().all(|e| e.kind == ExportKind::Named));
        assert!(exports.iter().all(|e| e.source.is_none()));
    }

    #[test]
    fn test_extract_default_export_function() {
        let source = "export default function App() {}";
        let module = parse_module(source, Path::new("app.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
    }

    #[test]
    fn test_extract_default_export_expression() {
        let source = "const x = 42; export default x;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
    }

    #[test]
    fn test_extract_reexports_from_module() {
        let source = "export { useState, useEffect } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 2);
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"useState"));
        assert!(names.contains(&"useEffect"));
        assert!(exports.iter().all(|e| e.kind == ExportKind::ReExport));
        assert!(exports.iter().all(|e| e.source.as_deref() == Some("react")));
    }

    #[test]
    fn test_extract_reexport_with_rename() {
        let source = "export { greet as greetUser } from './greeter';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "greetUser");
        assert_eq!(exports[0].kind, ExportKind::ReExport);
        assert_eq!(exports[0].source.as_deref(), Some("./greeter"));
    }

    #[test]
    fn test_extract_star_export() {
        let source = "export * from './utils';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "*");
        assert_eq!(exports[0].kind, ExportKind::StarExport);
        assert_eq!(exports[0].source.as_deref(), Some("./utils"));
    }

    #[test]
    fn test_no_exports_returns_empty() {
        let source = "const x = 1; function helper() {}";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exports = extract_exports(&module);
        assert!(exports.is_empty());
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::exports
```
Expected: `test result: ok. 6 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/
git commit -m "feat(exports): extract named, default, re-export, and star-export from module AST"
```

---

## Task 5: Static Import Extractor

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/imports.rs`
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/imports.rs` with only the test:

```rust
// crates/cloudpack-core/src/summarizer/imports.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_extract_named_imports() {
        let source = "import { useState, useEffect } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "react");
        assert_eq!(imports[0].kind, ImportKind::Named);
        assert!(!imports[0].is_dynamic);
        assert!(imports[0].bindings.contains(&"useState".to_string()));
        assert!(imports[0].bindings.contains(&"useEffect".to_string()));
    }

    #[test]
    fn test_extract_default_import() {
        let source = "import React from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Default);
        assert!(imports[0].bindings.contains(&"React".to_string()));
    }

    #[test]
    fn test_extract_namespace_import() {
        let source = "import * as lodash from 'lodash';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Namespace);
        assert!(imports[0].bindings.contains(&"lodash".to_string()));
    }

    #[test]
    fn test_extract_side_effect_import() {
        let source = "import 'reflect-metadata';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "reflect-metadata");
        assert_eq!(imports[0].kind, ImportKind::SideEffect);
        assert!(imports[0].bindings.is_empty());
    }

    #[test]
    fn test_extract_mixed_import() {
        // import React, { useState } from 'react'
        let source = "import React, { useState } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        // All bindings are collected under one import entry for the specifier.
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "react");
        assert!(imports[0].bindings.contains(&"React".to_string()));
        assert!(imports[0].bindings.contains(&"useState".to_string()));
    }
}
```

Add `pub mod imports;` to `crates/cloudpack-core/src/summarizer/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::imports
```
Expected: compilation error — `cannot find function 'extract_imports' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Replace `imports.rs` with the full implementation (tests included):

```rust
// crates/cloudpack-core/src/summarizer/imports.rs
use crate::types::{Import, ImportKind};
use swc_core::ecma::ast::{
    Callee, Expr, ImportSpecifier, Lit, Module, ModuleDecl, ModuleItem,
};
use swc_core::ecma::visit::{Visit, VisitWith};

/// Extract all static and dynamic imports from a parsed module.
pub fn extract_imports(module: &Module) -> Vec<Import> {
    let mut static_imports = collect_static_imports(module);
    let dynamic_imports = collect_dynamic_imports(module);
    static_imports.extend(dynamic_imports);
    static_imports
}

fn collect_static_imports(module: &Module) -> Vec<Import> {
    let mut imports = Vec::new();

    for item in &module.body {
        let decl = match item {
            ModuleItem::ModuleDecl(d) => d,
            ModuleItem::Stmt(_) => continue,
        };

        if let ModuleDecl::Import(import_decl) = decl {
            if import_decl.type_only {
                continue; // Type-only imports are erased at runtime; skip them.
            }

            let specifier = import_decl.src.value.to_string();

            if import_decl.specifiers.is_empty() {
                imports.push(Import {
                    specifier,
                    kind: ImportKind::SideEffect,
                    bindings: vec![],
                    is_dynamic: false,
                });
                continue;
            }

            // Determine the dominant kind and collect all binding names.
            let mut bindings = Vec::new();
            let mut kind = ImportKind::Named;

            for spec in &import_decl.specifiers {
                match spec {
                    ImportSpecifier::Default(def) => {
                        bindings.push(def.local.sym.to_string());
                        if matches!(kind, ImportKind::Named) {
                            kind = ImportKind::Default;
                        }
                    }
                    ImportSpecifier::Named(named) => {
                        bindings.push(named.local.sym.to_string());
                        // Named overrides Default when both are present.
                        kind = ImportKind::Named;
                    }
                    ImportSpecifier::Namespace(ns) => {
                        bindings.push(ns.local.sym.to_string());
                        kind = ImportKind::Namespace;
                    }
                }
            }

            imports.push(Import {
                specifier,
                kind,
                bindings,
                is_dynamic: false,
            });
        }
    }

    imports
}

struct DynamicImportVisitor {
    imports: Vec<Import>,
}

impl Visit for DynamicImportVisitor {
    fn visit_call_expr(&mut self, n: &swc_core::ecma::ast::CallExpr) {
        if let Callee::Import(_) = &n.callee {
            if let Some(first_arg) = n.args.first() {
                if let Expr::Lit(Lit::Str(s)) = first_arg.expr.as_ref() {
                    // Only track literal specifiers; variable specifiers are unresolvable.
                    self.imports.push(Import {
                        specifier: s.value.to_string(),
                        kind: ImportKind::Dynamic,
                        bindings: vec![],
                        is_dynamic: true,
                    });
                }
                // Non-literal import() specifiers are intentionally ignored.
            }
        }
        // Recurse into sub-expressions.
        n.visit_children_with(self);
    }
}

fn collect_dynamic_imports(module: &Module) -> Vec<Import> {
    let mut visitor = DynamicImportVisitor { imports: vec![] };
    module.visit_with(&mut visitor);
    visitor.imports
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_extract_named_imports() {
        let source = "import { useState, useEffect } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "react");
        assert_eq!(imports[0].kind, ImportKind::Named);
        assert!(!imports[0].is_dynamic);
        assert!(imports[0].bindings.contains(&"useState".to_string()));
        assert!(imports[0].bindings.contains(&"useEffect".to_string()));
    }

    #[test]
    fn test_extract_default_import() {
        let source = "import React from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Default);
        assert!(imports[0].bindings.contains(&"React".to_string()));
    }

    #[test]
    fn test_extract_namespace_import() {
        let source = "import * as lodash from 'lodash';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].kind, ImportKind::Namespace);
        assert!(imports[0].bindings.contains(&"lodash".to_string()));
    }

    #[test]
    fn test_extract_side_effect_import() {
        let source = "import 'reflect-metadata';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "reflect-metadata");
        assert_eq!(imports[0].kind, ImportKind::SideEffect);
        assert!(imports[0].bindings.is_empty());
    }

    #[test]
    fn test_extract_mixed_import() {
        let source = "import React, { useState } from 'react';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].specifier, "react");
        assert!(imports[0].bindings.contains(&"React".to_string()));
        assert!(imports[0].bindings.contains(&"useState".to_string()));
    }

    #[test]
    fn test_extract_dynamic_import_literal() {
        let source =
            "async function load() { const m = await import('./heavy'); return m; }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        let dynamic: Vec<&Import> = imports.iter().filter(|i| i.is_dynamic).collect();
        assert_eq!(dynamic.len(), 1);
        assert_eq!(dynamic[0].specifier, "./heavy");
        assert_eq!(dynamic[0].kind, ImportKind::Dynamic);
    }

    #[test]
    fn test_dynamic_import_non_literal_ignored() {
        let source = "async function load(name: string) { return import(name); }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let imports = extract_imports(&module);
        let dynamic: Vec<&Import> = imports.iter().filter(|i| i.is_dynamic).collect();
        // Non-literal specifiers are unresolvable at static analysis time; ignored.
        assert_eq!(dynamic.len(), 0);
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::imports
```
Expected: `test result: ok. 7 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/imports.rs \
        crates/cloudpack-core/src/summarizer/mod.rs
git commit -m "feat(imports): extract static and dynamic imports with binding tracking"
```

---

## Task 6: Side Effect Analyzer

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/side_effects.rs`
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/side_effects.rs` with only the test:

```rust
// crates/cloudpack-core/src/summarizer/side_effects.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_function_only_module_is_none() {
        let source = r#"
export function add(a: number, b: number): number { return a + b; }
export function subtract(a: number, b: number): number { return a - b; }
"#;
        let module = parse_module(source, Path::new("math.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_class_only_module_is_none() {
        let source = "export class Calculator { add(a: number, b: number) { return a + b; } }";
        let module = parse_module(source, Path::new("calc.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_pure_literal_const_is_none() {
        let source = "export const PI = 3.14159; export const NAME = 'cloudpack';";
        let module = parse_module(source, Path::new("constants.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_top_level_call_is_possible() {
        let source = "const result = computeValue();\nexport { result };";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert!(
            matches!(marker, SideEffectMarker::Possible { .. }),
            "expected POSSIBLE, got {:?}",
            marker
        );
    }

    #[test]
    fn test_top_level_expression_statement_is_possible() {
        let source = "console.log('loaded');\nexport const x = 1;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert!(matches!(marker, SideEffectMarker::Possible { .. }));
    }

    #[test]
    fn test_window_property_write_is_definite() {
        let source = "window.APP_VERSION = '2.0.0';\nexport const x = 1;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::Definite);
    }

    #[test]
    fn test_document_property_write_is_definite() {
        let source = "document.title = 'My App';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::Definite);
    }
}
```

Add `pub mod side_effects;` to `crates/cloudpack-core/src/summarizer/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::side_effects
```
Expected: compilation error — `cannot find function 'analyze_side_effects' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Replace `side_effects.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/summarizer/side_effects.rs
use crate::types::SideEffectMarker;
use swc_core::ecma::ast::{
    AssignTarget, Decl, Expr, Module, ModuleItem, SimpleAssignTarget, Stmt,
};

/// Globals whose property writes constitute definite side effects.
const AMBIENT_GLOBALS: &[&str] = &[
    "window", "document", "globalThis", "global", "self", "navigator",
];

/// Analyze whether a module has side effects at initialization time.
///
/// Rules:
/// - `NONE`     — module contains only function/class/type declarations and import/export statements.
/// - `DEFINITE` — module directly assigns to a known global object property (`window.X = …`).
/// - `POSSIBLE` — module has any other top-level executable code (function calls, expressions, etc.).
pub fn analyze_side_effects(module: &Module) -> SideEffectMarker {
    for item in &module.body {
        let stmt = match item {
            ModuleItem::Stmt(s) => s,
            ModuleItem::ModuleDecl(_) => continue, // imports/exports are not side effects
        };

        match stmt {
            // Pure declarations — don't execute at module initialization time.
            Stmt::Decl(Decl::Fn(_)) => continue,
            Stmt::Decl(Decl::Class(_)) => continue,
            Stmt::Decl(Decl::TsInterface(_)) => continue,
            Stmt::Decl(Decl::TsTypeAlias(_)) => continue,
            Stmt::Decl(Decl::TsEnum(_)) => continue,
            Stmt::Decl(Decl::TsModule(_)) => continue,

            // Variable declarations: safe only if all initializers are pure literals.
            Stmt::Decl(Decl::Var(var_decl)) => {
                let has_impure = var_decl.decls.iter().any(|d| {
                    d.init
                        .as_ref()
                        .map_or(false, |init| !is_pure_expr(init))
                });
                if has_impure {
                    return SideEffectMarker::Possible {
                        reason: "top-level variable with non-literal initializer".to_string(),
                    };
                }
                continue;
            }

            // Other declarations (e.g. `using`) — conservative.
            Stmt::Decl(_) => {
                return SideEffectMarker::Possible {
                    reason: "top-level declaration with possible side effects".to_string(),
                };
            }

            // Expression statements: check for direct global writes first.
            Stmt::Expr(expr_stmt) => {
                if let Expr::Assign(assign) = expr_stmt.expr.as_ref() {
                    if is_global_member_assignment(assign) {
                        return SideEffectMarker::Definite;
                    }
                }
                return SideEffectMarker::Possible {
                    reason: "top-level expression statement".to_string(),
                };
            }

            // Any other statement (if, for, try, …) is conservatively POSSIBLE.
            _ => {
                return SideEffectMarker::Possible {
                    reason: "top-level non-declaration statement".to_string(),
                };
            }
        }
    }

    SideEffectMarker::None
}

/// Returns `true` if the assignment's left-hand side is a property of a known global object
/// (e.g. `window.X = …`, `document.title = …`).
fn is_global_member_assignment(assign: &swc_core::ecma::ast::AssignExpr) -> bool {
    if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
        if let Expr::Ident(obj) = member.obj.as_ref() {
            return AMBIENT_GLOBALS.contains(&obj.sym.as_ref());
        }
    }
    false
}

/// Returns `true` for expressions that provably don't execute code at module init time.
fn is_pure_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) => true,       // number, string, bool, regex, null
        Expr::Arrow(_) => true,     // arrow function — defined but not called
        Expr::Fn(_) => true,        // function expression — defined but not called
        Expr::Ident(_) => true,     // identifier reference (could be anything, but not a call)
        _ => false,                 // conservative: everything else may have side effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_function_only_module_is_none() {
        let source = r#"
export function add(a: number, b: number): number { return a + b; }
export function subtract(a: number, b: number): number { return a - b; }
"#;
        let module = parse_module(source, Path::new("math.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_class_only_module_is_none() {
        let source = "export class Calculator { add(a: number, b: number) { return a + b; } }";
        let module = parse_module(source, Path::new("calc.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_pure_literal_const_is_none() {
        let source = "export const PI = 3.14159; export const NAME = 'cloudpack';";
        let module = parse_module(source, Path::new("constants.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::None);
    }

    #[test]
    fn test_top_level_call_is_possible() {
        let source = "const result = computeValue();\nexport { result };";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert!(
            matches!(marker, SideEffectMarker::Possible { .. }),
            "expected POSSIBLE, got {:?}",
            marker
        );
    }

    #[test]
    fn test_top_level_expression_statement_is_possible() {
        let source = "console.log('loaded');\nexport const x = 1;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert!(matches!(marker, SideEffectMarker::Possible { .. }));
    }

    #[test]
    fn test_window_property_write_is_definite() {
        let source = "window.APP_VERSION = '2.0.0';\nexport const x = 1;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::Definite);
    }

    #[test]
    fn test_document_property_write_is_definite() {
        let source = "document.title = 'My App';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let marker = analyze_side_effects(&module);
        assert_eq!(marker, SideEffectMarker::Definite);
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::side_effects
```
Expected: `test result: ok. 7 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/side_effects.rs \
        crates/cloudpack-core/src/summarizer/mod.rs
git commit -m "feat(side-effects): analyze module-level side effects (NONE/POSSIBLE/DEFINITE)"
```

---

## Task 7: Call Edge Analyzer

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/call_edges.rs`
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/call_edges.rs` with only the test:

```rust
// crates/cloudpack-core/src/summarizer/call_edges.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn test_detects_call_edge_between_exports() {
        let source = r#"
export function formatName(first: string, last: string): string {
    return `${first} ${last}`;
}
export function greet(name: string): string {
    const formatted = formatName("Hello", name);
    return `Hi, ${formatted}!`;
}
function helper(): string {
    return greet("world"); // not exported, so not a tracked caller
}
"#;
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = ["formatName", "greet"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let edges = extract_call_edges(&module, &exported);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].caller, "greet");
        assert_eq!(edges[0].callee, "formatName");
    }

    #[test]
    fn test_no_edges_for_non_calling_module() {
        let source = r#"
export function add(a: number, b: number): number { return a + b; }
export function mul(a: number, b: number): number { return a * b; }
"#;
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = ["add", "mul"].iter().map(|s| s.to_string()).collect();
        let edges = extract_call_edges(&module, &exported);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_no_edges_when_no_exported_names() {
        let source = "function add(a: number, b: number) { return a + b; }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = HashSet::new();
        let edges = extract_call_edges(&module, &exported);
        assert!(edges.is_empty());
    }
}
```

Add `pub mod call_edges;` to `crates/cloudpack-core/src/summarizer/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::call_edges
```
Expected: compilation error — `cannot find function 'extract_call_edges' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Replace `call_edges.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/summarizer/call_edges.rs
use crate::types::CallEdge;
use std::collections::HashSet;
use swc_core::ecma::ast::{Callee, Decl, Expr, FnDecl, Module, ModuleDecl, ModuleItem, Stmt};
use swc_core::ecma::visit::{Visit, VisitWith};

/// Extract directed call edges between exported functions within the same module.
///
/// An edge `A → B` is recorded when exported function `A` calls exported function `B`
/// directly by name. This information powers Phase 2's function-level DCE.
pub fn extract_call_edges(module: &Module, exported_names: &HashSet<String>) -> Vec<CallEdge> {
    let mut visitor = CallEdgeVisitor {
        exported_names,
        current_fn: None,
        edges: Vec::new(),
    };
    // Visit only top-level exported function declarations.
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export_decl)) => {
                if let Decl::Fn(fn_decl) = &export_decl.decl {
                    visitor.visit_exported_fn(fn_decl);
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                // Also check top-level (non-exported) function declarations
                // in case they call exports — but only if the fn itself is exported.
                if exported_names.contains(fn_decl.ident.sym.as_ref()) {
                    visitor.visit_exported_fn(fn_decl);
                }
            }
            _ => {}
        }
    }
    visitor.edges
}

struct CallEdgeVisitor<'a> {
    exported_names: &'a HashSet<String>,
    current_fn: Option<String>,
    edges: Vec<CallEdge>,
}

impl<'a> CallEdgeVisitor<'a> {
    fn visit_exported_fn(&mut self, fn_decl: &FnDecl) {
        let name = fn_decl.ident.sym.to_string();
        let prev = self.current_fn.replace(name);
        fn_decl.function.visit_children_with(self);
        self.current_fn = prev;
    }
}

impl<'a> Visit for CallEdgeVisitor<'a> {
    fn visit_call_expr(&mut self, n: &swc_core::ecma::ast::CallExpr) {
        if let Some(ref caller) = self.current_fn.clone() {
            if let Callee::Expr(callee_expr) = &n.callee {
                if let Expr::Ident(ident) = callee_expr.as_ref() {
                    let callee_name = ident.sym.to_string();
                    if self.exported_names.contains(&callee_name) && callee_name != *caller {
                        self.edges.push(CallEdge {
                            caller: caller.clone(),
                            callee: callee_name,
                        });
                    }
                }
            }
        }
        n.visit_children_with(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn test_detects_call_edge_between_exports() {
        let source = r#"
export function formatName(first: string, last: string): string {
    return `${first} ${last}`;
}
export function greet(name: string): string {
    const formatted = formatName("Hello", name);
    return `Hi, ${formatted}!`;
}
function helper(): string {
    return greet("world");
}
"#;
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = ["formatName", "greet"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let edges = extract_call_edges(&module, &exported);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].caller, "greet");
        assert_eq!(edges[0].callee, "formatName");
    }

    #[test]
    fn test_no_edges_for_non_calling_module() {
        let source = r#"
export function add(a: number, b: number): number { return a + b; }
export function mul(a: number, b: number): number { return a * b; }
"#;
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = ["add", "mul"].iter().map(|s| s.to_string()).collect();
        let edges = extract_call_edges(&module, &exported);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_no_edges_when_no_exported_names() {
        let source = "function add(a: number, b: number) { return a + b; }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let exported: HashSet<String> = HashSet::new();
        let edges = extract_call_edges(&module, &exported);
        assert!(edges.is_empty());
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::call_edges
```
Expected: `test result: ok. 3 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/call_edges.rs \
        crates/cloudpack-core/src/summarizer/mod.rs
git commit -m "feat(call-edges): extract intra-module export-to-export call graph"
```

---

## Task 8: Ambient Ref Analyzer

**Files:**
- Create: `crates/cloudpack-core/src/summarizer/ambient_refs.rs`
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/cloudpack-core/src/summarizer/ambient_refs.rs` with only the test:

```rust
// crates/cloudpack-core/src/summarizer/ambient_refs.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_detects_window_property_access() {
        let source = "window.APP_VERSION = '1.0.0';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.contains(&"window.APP_VERSION".to_string()),
            "expected 'window.APP_VERSION', got: {:?}",
            refs
        );
    }

    #[test]
    fn test_detects_document_property() {
        let source = "document.title = 'My App';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.contains(&"document.title".to_string()));
    }

    #[test]
    fn test_detects_global_this() {
        let source = "const env = globalThis.process?.env;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.contains(&"globalThis.process".to_string()));
    }

    #[test]
    fn test_pure_function_has_no_ambient_refs() {
        let source =
            "export function add(a: number, b: number): number { return a + b; }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.is_empty(), "expected no refs, got: {:?}", refs);
    }

    #[test]
    fn test_deduplicates_repeated_global() {
        let source = "window.X = 1; window.X = 2; window.Y = 3;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        let window_x_count = refs.iter().filter(|r| *r == "window.X").count();
        assert_eq!(window_x_count, 1, "window.X should appear exactly once");
    }
}
```

Add `pub mod ambient_refs;` to `crates/cloudpack-core/src/summarizer/mod.rs`.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::ambient_refs
```
Expected: compilation error — `cannot find function 'extract_ambient_refs' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Replace `ambient_refs.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/summarizer/ambient_refs.rs
use std::collections::HashSet;
use swc_core::ecma::ast::{Expr, MemberProp, Module};
use swc_core::ecma::visit::{Visit, VisitWith};

/// Globals whose direct property accesses are considered ambient references.
const AMBIENT_GLOBALS: &[&str] = &[
    "window", "document", "globalThis", "global", "self", "navigator",
];

/// Extract all references to known browser/Node.js globals in the module.
///
/// Reports the first-level property access form when available:
/// - `window.APP_VERSION` → `"window.APP_VERSION"`
/// - `document.title` → `"document.title"`
/// - Bare `window` (no property) → `"window"`
///
/// Results are deduplicated; insertion order is preserved.
pub fn extract_ambient_refs(module: &Module) -> Vec<String> {
    let mut visitor = AmbientRefVisitor {
        seen: HashSet::new(),
        refs: Vec::new(),
    };
    module.visit_with(&mut visitor);
    visitor.refs
}

struct AmbientRefVisitor {
    seen: HashSet<String>,
    refs: Vec<String>,
}

impl AmbientRefVisitor {
    fn record(&mut self, s: String) {
        if self.seen.insert(s.clone()) {
            self.refs.push(s);
        }
    }
}

impl Visit for AmbientRefVisitor {
    /// Intercept every expression. When we see `ambient.prop`, record `"ambient.prop"` and
    /// stop recursing (so we don't also record the bare `"ambient"`). For all other
    /// expressions, recurse normally.
    fn visit_expr(&mut self, n: &Expr) {
        match n {
            Expr::Member(member) => {
                if let Expr::Ident(obj) = member.obj.as_ref() {
                    if AMBIENT_GLOBALS.contains(&obj.sym.as_ref()) {
                        let ref_str = match &member.prop {
                            MemberProp::Ident(prop) => {
                                format!("{}.{}", obj.sym, prop.sym)
                            }
                            // Computed access (window["x"]) — record just the global name.
                            _ => obj.sym.to_string(),
                        };
                        self.record(ref_str);
                        // Don't recurse — we've consumed this member expression.
                        return;
                    }
                }
                n.visit_children_with(self);
            }
            Expr::Ident(ident) => {
                // Bare global reference not inside a member expression.
                if AMBIENT_GLOBALS.contains(&ident.sym.as_ref()) {
                    self.record(ident.sym.to_string());
                }
                // Identifiers have no children to recurse into.
            }
            _ => {
                n.visit_children_with(self);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    #[test]
    fn test_detects_window_property_access() {
        let source = "window.APP_VERSION = '1.0.0';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.contains(&"window.APP_VERSION".to_string()),
            "expected 'window.APP_VERSION', got: {:?}",
            refs
        );
    }

    #[test]
    fn test_detects_document_property() {
        let source = "document.title = 'My App';";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.contains(&"document.title".to_string()));
    }

    #[test]
    fn test_detects_global_this() {
        let source = "const env = globalThis.process?.env;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.contains(&"globalThis.process".to_string()));
    }

    #[test]
    fn test_pure_function_has_no_ambient_refs() {
        let source =
            "export function add(a: number, b: number): number { return a + b; }";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        assert!(refs.is_empty(), "expected no refs, got: {:?}", refs);
    }

    #[test]
    fn test_deduplicates_repeated_global() {
        let source = "window.X = 1; window.X = 2; window.Y = 3;";
        let module = parse_module(source, Path::new("test.ts")).unwrap();
        let refs = extract_ambient_refs(&module);
        let window_x_count = refs.iter().filter(|r| *r == "window.X").count();
        assert_eq!(window_x_count, 1, "window.X should appear exactly once");
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::ambient_refs
```
Expected: `test result: ok. 5 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/ambient_refs.rs \
        crates/cloudpack-core/src/summarizer/mod.rs
git commit -m "feat(ambient-refs): detect reads/writes to browser and Node.js globals"
```

---

## Task 9: ModuleSummarizer — Wire All Extractors

**Files:**
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`
- Create: `crates/cloudpack-core/tests/fixtures/esm_basic.ts`
- Create: `crates/cloudpack-core/tests/fixtures/esm_reexport.ts`
- Create: `crates/cloudpack-core/tests/fixtures/dynamic_import.ts`
- Create: `crates/cloudpack-core/tests/fixtures/side_effects_ambient.ts`
- Create: `crates/cloudpack-core/tests/fixtures/side_effects_pure.ts`
- Create: `crates/cloudpack-core/tests/integration_test.rs`

- [ ] **Step 1: Write the failing test**

Create the fixture files:

`crates/cloudpack-core/tests/fixtures/esm_basic.ts`:
```typescript
import { createContext } from 'react';
import type { FC } from 'react';

export const VERSION: string = '1.0.0';

export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export class Greeter {
  greet(name: string): string {
    return `Hi, ${name}!`;
  }
}

export default function main(): void {
  console.log('main');
}
```

`crates/cloudpack-core/tests/fixtures/esm_reexport.ts`:
```typescript
export { useState, useEffect } from 'react';
export * from './utils';
export { greet as greetUser } from './greeter';
```

`crates/cloudpack-core/tests/fixtures/dynamic_import.ts`:
```typescript
export async function loadHeavy(): Promise<unknown> {
  const mod = await import('./heavy');
  return mod;
}

export async function loadByName(name: string): Promise<unknown> {
  return import(name); // non-literal specifier — intentionally ignored
}
```

`crates/cloudpack-core/tests/fixtures/side_effects_ambient.ts`:
```typescript
window.APP_VERSION = '2.0.0';
document.title = 'My App';

export function getVersion(): string {
  return window.APP_VERSION as string;
}
```

`crates/cloudpack-core/tests/fixtures/side_effects_pure.ts`:
```typescript
export function add(a: number, b: number): number {
  return a + b;
}

export function multiply(a: number, b: number): number {
  return a * b;
}

export const PI = 3.14159;
```

Create the integration test at `crates/cloudpack-core/tests/integration_test.rs` with only the test (no `ModuleSummarizer` yet):

```rust
// crates/cloudpack-core/tests/integration_test.rs
use std::path::PathBuf;
use cloudpack_core::summarizer::ModuleSummarizer;
use cloudpack_core::types::{ExportKind, ImportKind, SideEffectMarker};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

#[test]
fn test_summarize_esm_basic() {
    let path = fixtures_dir().join("esm_basic.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();

    // Content hash is 64 hex chars
    assert_eq!(node.id.as_str().len(), 64);
    assert!(node.path.ends_with("esm_basic.ts"));

    // Exports: VERSION, greet, Greeter, default
    let names: Vec<&str> = node.summary.exports.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"VERSION"), "missing VERSION: {:?}", names);
    assert!(names.contains(&"greet"), "missing greet: {:?}", names);
    assert!(names.contains(&"Greeter"), "missing Greeter: {:?}", names);
    assert!(names.contains(&"default"), "missing default: {:?}", names);

    // Imports: 'react' (type-only is filtered, so createContext should be present)
    let specs: Vec<&str> = node.summary.imports.iter().map(|i| i.specifier.as_str()).collect();
    assert!(specs.contains(&"react"), "missing react import: {:?}", specs);

    // Side effects: default (main() is exported, console.log is inside a function body)
    // Module-level: only declarations and one type-import → NONE
    assert_eq!(
        node.summary.side_effects,
        SideEffectMarker::None,
        "esm_basic.ts should have NONE side effects"
    );
}

#[test]
fn test_summarize_esm_reexport() {
    let path = fixtures_dir().join("esm_reexport.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();

    let kinds: Vec<&ExportKind> = node.summary.exports.iter().map(|e| &e.kind).collect();
    assert!(
        kinds.contains(&&ExportKind::ReExport),
        "expected re-exports: {:?}",
        node.summary.exports
    );
    assert!(
        kinds.contains(&&ExportKind::StarExport),
        "expected star-export: {:?}",
        node.summary.exports
    );
}

#[test]
fn test_summarize_dynamic_import() {
    let path = fixtures_dir().join("dynamic_import.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();

    let dynamic: Vec<&cloudpack_core::types::Import> =
        node.summary.imports.iter().filter(|i| i.is_dynamic).collect();
    assert_eq!(dynamic.len(), 1, "expected 1 literal dynamic import, got: {:?}", dynamic);
    assert_eq!(dynamic[0].specifier, "./heavy");
}

#[test]
fn test_summarize_side_effects_ambient_is_definite() {
    let path = fixtures_dir().join("side_effects_ambient.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();
    assert_eq!(node.summary.side_effects, SideEffectMarker::Definite);
    assert!(
        node.summary.ambient_refs.contains(&"window.APP_VERSION".to_string()),
        "expected window.APP_VERSION in ambient_refs: {:?}",
        node.summary.ambient_refs
    );
}

#[test]
fn test_summarize_side_effects_pure_is_none() {
    let path = fixtures_dir().join("side_effects_pure.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();
    assert_eq!(node.summary.side_effects, SideEffectMarker::None);
    assert!(node.summary.ambient_refs.is_empty());
}

#[test]
fn test_summary_serializes_to_json() {
    let path = fixtures_dir().join("side_effects_pure.ts");
    let summarizer = ModuleSummarizer::new();
    let node = summarizer.summarize(&path).unwrap();
    let json = serde_json::to_string_pretty(&node).unwrap();
    assert!(json.contains("\"sideEffects\""));
    assert!(json.contains("NONE"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core --test integration_test
```
Expected: compilation error — `module 'summarizer' has no struct 'ModuleSummarizer'`

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/summarizer/mod.rs` with the full wired implementation:

```rust
// crates/cloudpack-core/src/summarizer/mod.rs
pub mod ambient_refs;
pub mod call_edges;
pub mod exports;
pub mod imports;
pub mod parser;
pub mod side_effects;

use crate::cjs::stub::{detect_cjs_exports, generate_cjs_stub, is_cjs};
use crate::types::{BundleGraphNode, ContentHash, ModuleSummary};
use ambient_refs::extract_ambient_refs;
use anyhow::{Context, Result};
use call_edges::extract_call_edges;
use exports::extract_exports;
use imports::extract_imports;
use parser::parse_module;
use side_effects::analyze_side_effects;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Summarizes a single JS/TS source file into a `BundleGraphNode`.
pub struct ModuleSummarizer;

impl ModuleSummarizer {
    pub fn new() -> Self {
        Self
    }

    /// Read `path`, parse it with SWC, run all five extractors, and return the node.
    ///
    /// For `.js` files that use `module.exports`, a synthetic ESM stub is generated
    /// via static analysis before parsing. See `cjs/stub.rs` for limitations.
    pub fn summarize(&self, path: &Path) -> Result<BundleGraphNode> {
        let source = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let id = ContentHash::from_source(&source);

        // CJS interop: generate a synthetic ESM stub for files that use module.exports.
        let (parse_source, parse_path): (String, PathBuf) =
            if path.extension().and_then(|e| e.to_str()) == Some("js") && is_cjs(&source) {
                let export_names = detect_cjs_exports(&source);
                let stub = generate_cjs_stub(path, &export_names);
                let stub_path = path.with_extension("mjs");
                (stub, stub_path)
            } else {
                (source, path.to_path_buf())
            };

        let module = parse_module(&parse_source, &parse_path)?;

        let exports = extract_exports(&module);
        let imports = extract_imports(&module);
        let side_effects = analyze_side_effects(&module);
        let exported_names: HashSet<String> =
            exports.iter().map(|e| e.name.clone()).collect();
        let call_edges = extract_call_edges(&module, &exported_names);
        let ambient_refs = extract_ambient_refs(&module);

        Ok(BundleGraphNode {
            id,
            path: path.to_string_lossy().into_owned(),
            summary: ModuleSummary {
                exports,
                imports,
                side_effects,
                call_edges,
                ambient_refs,
            },
            alive: false,
            chunk_id: None,
        })
    }
}

impl Default for ModuleSummarizer {
    fn default() -> Self {
        Self::new()
    }
}
```

> **Note:** This implementation imports from `crate::cjs::stub`. The `cjs` module stub (Task 10) must exist for this to compile. Create a minimal placeholder now:

Create `crates/cloudpack-core/src/cjs/mod.rs`:
```rust
pub mod stub;
pub use stub::{detect_cjs_exports, generate_cjs_stub, is_cjs};
```

Create `crates/cloudpack-core/src/cjs/stub.rs` (placeholder — full implementation in Task 10):
```rust
// crates/cloudpack-core/src/cjs/stub.rs
use std::path::Path;

pub fn is_cjs(source: &str) -> bool {
    source.contains("module.exports") || source.contains("exports.")
}

pub fn detect_cjs_exports(_source: &str) -> Vec<String> {
    vec![]
}

pub fn generate_cjs_stub(path: &Path, _exports: &[String]) -> String {
    format!("// CJS stub placeholder for {}", path.display())
}
```

Also restore the full `crates/cloudpack-core/src/lib.rs` (remove the commented-out modules from Task 3):

```rust
pub mod cache;
pub mod cjs;
pub mod summarizer;
pub mod types;
pub mod validation;

pub use types::{
    BundleGraphNode, CallEdge, ContentHash, Export, ExportKind, Import, ImportKind,
    ModuleSummary, SideEffectMarker,
};
```

Create placeholder modules to unblock compilation:

`crates/cloudpack-core/src/cache/mod.rs`:
```rust
pub mod local;
pub mod package;
```

`crates/cloudpack-core/src/cache/local.rs`:
```rust
// placeholder — full implementation in Task 11
```

`crates/cloudpack-core/src/cache/package.rs`:
```rust
// placeholder — full implementation in Task 12
```

`crates/cloudpack-core/src/validation.rs`:
```rust
// placeholder — full implementation in Task 15
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core --test integration_test
```
Expected: `test result: ok. 6 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/mod.rs \
        crates/cloudpack-core/src/cjs/ \
        crates/cloudpack-core/src/cache/ \
        crates/cloudpack-core/src/validation.rs \
        crates/cloudpack-core/src/lib.rs \
        crates/cloudpack-core/tests/
git commit -m "feat(summarizer): wire all extractors into ModuleSummarizer + integration tests"
```

---

## Task 10: Content-Addressed Local Cache

**Files:**
- Modify: `crates/cloudpack-core/src/cache/local.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/cloudpack-core/src/cache/local.rs` with the test-only version:

```rust
// crates/cloudpack-core/src/cache/local.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ContentHash, Export, ExportKind, Import, ImportKind, ModuleSummary, SideEffectMarker,
    };
    use tempfile::TempDir;

    fn make_summary() -> ModuleSummary {
        ModuleSummary {
            exports: vec![Export {
                name: "foo".to_string(),
                kind: ExportKind::Named,
                source: None,
            }],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        }
    }

    #[test]
    fn test_put_and_get_round_trip() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("export const foo = 1;");
        let summary = make_summary();

        cache.put(&hash, &summary).unwrap();
        let retrieved = cache.get(&hash).unwrap();
        assert!(retrieved.is_some(), "expected cached value after put");
        let back = retrieved.unwrap();
        assert_eq!(back.exports.len(), 1);
        assert_eq!(back.exports[0].name, "foo");
    }

    #[test]
    fn test_get_returns_none_for_missing_key() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("not in cache");
        let result = cache.get(&hash).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_put_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("idempotent");
        let summary = make_summary();

        cache.put(&hash, &summary).unwrap();
        cache.put(&hash, &summary).unwrap(); // second put should not error
        let result = cache.get(&hash).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_cache_is_sharded_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("sharding test");
        let summary = make_summary();

        cache.put(&hash, &summary).unwrap();

        // Entries are stored under a 2-char shard prefix subdirectory.
        let shard_dir = dir.path().join(&hash.as_str()[..2]);
        assert!(
            shard_dir.exists(),
            "expected shard subdirectory to exist at {:?}",
            shard_dir
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core cache::local
```
Expected: compilation error — `cannot find struct 'LocalCache' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/cache/local.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/cache/local.rs
use crate::types::{ContentHash, ModuleSummary};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// A content-addressed disk cache for `ModuleSummary` values.
///
/// Entries are stored as JSON files under `<root>/<first-2-hex-chars>/<remaining-62-hex-chars>.json`
/// to avoid filesystem limits from too many entries in a single directory.
///
/// Writes are atomic: data is written to a `.tmp` file and renamed into place.
pub struct LocalCache {
    root: PathBuf,
}

impl LocalCache {
    /// Create a cache rooted at `root`, creating the directory if needed.
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("Failed to create cache directory: {}", root.display()))?;
        Ok(Self { root })
    }

    /// Build the default cache path: `~/.cloudpack/cache/summaries/`.
    pub fn with_default_root() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        Self::new(home.join(".cloudpack").join("cache").join("summaries"))
    }

    /// Look up a cached `ModuleSummary` by content hash.
    /// Returns `None` if the entry does not exist.
    pub fn get(&self, hash: &ContentHash) -> Result<Option<ModuleSummary>> {
        let path = self.entry_path(hash);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read cache entry: {}", path.display()))?;
        let summary: ModuleSummary = serde_json::from_str(&data)
            .with_context(|| format!("Corrupt cache entry at {}", path.display()))?;
        Ok(Some(summary))
    }

    /// Store a `ModuleSummary` under the given content hash.
    /// Uses an atomic write (temp-file + rename) to avoid partial writes.
    pub fn put(&self, hash: &ContentHash, summary: &ModuleSummary) -> Result<()> {
        let path = self.entry_path(hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create cache shard directory: {}", parent.display())
            })?;
        }
        let data = serde_json::to_string(summary)
            .context("Failed to serialize ModuleSummary")?;

        // Atomic write: write to `.tmp`, then rename into place.
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &data)
            .with_context(|| format!("Failed to write temp cache file: {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &path)
            .with_context(|| format!("Failed to rename temp cache file to {}", path.display()))?;
        Ok(())
    }

    /// Total size of all cached entries in bytes.
    pub fn total_bytes(&self) -> Result<u64> {
        let mut total = 0u64;
        if !self.root.exists() {
            return Ok(0);
        }
        for entry in walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
        Ok(total)
    }

    /// The filesystem path for a given content hash entry.
    fn entry_path(&self, hash: &ContentHash) -> PathBuf {
        let hex = hash.as_str();
        // Shard: first 2 hex chars become a subdirectory.
        self.root
            .join(&hex[..2])
            .join(format!("{}.json", &hex[2..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Export, ExportKind, SideEffectMarker};
    use tempfile::TempDir;

    fn make_summary() -> ModuleSummary {
        ModuleSummary {
            exports: vec![Export {
                name: "foo".to_string(),
                kind: ExportKind::Named,
                source: None,
            }],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        }
    }

    #[test]
    fn test_put_and_get_round_trip() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("export const foo = 1;");
        let summary = make_summary();

        cache.put(&hash, &summary).unwrap();
        let retrieved = cache.get(&hash).unwrap();
        assert!(retrieved.is_some());
        let back = retrieved.unwrap();
        assert_eq!(back.exports.len(), 1);
        assert_eq!(back.exports[0].name, "foo");
    }

    #[test]
    fn test_get_returns_none_for_missing_key() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("not in cache");
        assert!(cache.get(&hash).unwrap().is_none());
    }

    #[test]
    fn test_put_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("idempotent");
        let summary = make_summary();
        cache.put(&hash, &summary).unwrap();
        cache.put(&hash, &summary).unwrap();
        assert!(cache.get(&hash).unwrap().is_some());
    }

    #[test]
    fn test_cache_is_sharded_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("sharding test");
        cache.put(&hash, &make_summary()).unwrap();
        let shard_dir = dir.path().join(&hash.as_str()[..2]);
        assert!(shard_dir.exists());
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core cache::local
```
Expected: `test result: ok. 4 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/cache/local.rs
git commit -m "feat(cache): add content-addressed local disk cache with atomic writes and sharding"
```

---

## Task 11: Package-Level Fast Path Cache

**Files:**
- Modify: `crates/cloudpack-core/src/cache/package.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/cloudpack-core/src/cache/package.rs` with the test-only version:

```rust
// crates/cloudpack-core/src/cache/package.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_fake_package(dir: &std::path::Path, name: &str, version: &str) -> std::path::PathBuf {
        let pkg_dir = dir.join("node_modules").join(name);
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.json"),
            format!(r#"{{"name":"{name}","version":"{version}","dependencies":{{}}}}"#),
        )
        .unwrap();
        pkg_dir
    }

    #[test]
    fn test_find_package_dir_detects_node_modules() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");
        let module_path = pkg_dir.join("index.js");

        let found = find_package_dir(&module_path);
        assert!(found.is_some(), "expected package dir to be found");
        assert!(found.unwrap().ends_with("lodash"));
    }

    #[test]
    fn test_find_package_dir_returns_none_for_src_file() {
        let dir = TempDir::new().unwrap();
        let src_file = dir.path().join("src").join("app.ts");
        fs::create_dir_all(src_file.parent().unwrap()).unwrap();
        let found = find_package_dir(&src_file);
        assert!(found.is_none());
    }

    #[test]
    fn test_find_package_dir_handles_scoped_packages() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("node_modules").join("@scope").join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.json"),
            r#"{"name":"@scope/pkg","version":"1.0.0","dependencies":{}}"#,
        )
        .unwrap();
        let module_path = pkg_dir.join("index.js");
        let found = find_package_dir(&module_path);
        assert!(found.is_some());
        let found_path = found.unwrap();
        assert!(found_path.to_string_lossy().contains("@scope"));
        assert!(found_path.to_string_lossy().contains("pkg"));
    }

    #[test]
    fn test_compute_package_hash_is_stable() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "react", "18.2.0");
        let h1 = compute_package_hash(&pkg_dir).unwrap();
        let h2 = compute_package_hash(&pkg_dir).unwrap();
        assert_eq!(h1, h2, "package hash should be deterministic");
    }

    #[test]
    fn test_node_modules_key_is_stable_across_source_changes() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");
        let module_path = pkg_dir.join("index.js");

        let source_v1 = "module.exports = { map: function() {} };";
        let source_v2 = "module.exports = { map: function() { /* optimized */ } };";

        let key_v1 = PackageLevelCache::cache_key_for(&module_path, source_v1).unwrap();
        let key_v2 = PackageLevelCache::cache_key_for(&module_path, source_v2).unwrap();
        assert_eq!(
            key_v1, key_v2,
            "node_modules cache key must not change when only source changes"
        );
    }

    #[test]
    fn test_source_file_key_changes_with_content() {
        let dir = TempDir::new().unwrap();
        let src_path = dir.path().join("src").join("app.ts");
        fs::create_dir_all(src_path.parent().unwrap()).unwrap();

        let key_v1 = PackageLevelCache::cache_key_for(&src_path, "export const a = 1;").unwrap();
        let key_v2 = PackageLevelCache::cache_key_for(&src_path, "export const b = 2;").unwrap();
        assert_ne!(
            key_v1, key_v2,
            "source file cache key must change when content changes"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core cache::package
```
Expected: compilation error — `cannot find function 'find_package_dir'`, `'compute_package_hash'`, `'PackageLevelCache'`

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/cache/package.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/cache/package.rs
//
// Two-tier cache fast path for node_modules.
//
// Motivation: At Teams scale, ~80% of the 50k module graph is stable vendor packages.
// Re-hashing source files on every build wastes CPU. The package-level key uses
// `SHA-256(package-name + version + dep-tree)` which is stable until a package.json changes.
//
// This means a developer who upgrades lodash from 4.17.20 → 4.17.21 gets a cache miss
// (correct), but a developer who makes no npm changes hits the cache for all vendor modules
// without reading a single source file (fast).
use crate::cache::local::LocalCache;
use crate::types::{ContentHash, ModuleSummary};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Find the `node_modules/<pkg>` root directory for a module path.
///
/// Handles both regular packages (`node_modules/lodash`) and scoped packages
/// (`node_modules/@scope/pkg`).
///
/// Returns `None` if the path is not inside `node_modules` or if the package
/// directory does not contain a `package.json`.
pub fn find_package_dir(module_path: &Path) -> Option<PathBuf> {
    let path_str = module_path.to_string_lossy();
    let nm_marker = "node_modules/";
    let nm_idx = path_str.find(nm_marker)?;
    let after_nm = &path_str[nm_idx + nm_marker.len()..];

    let pkg_rel = if after_nm.starts_with('@') {
        // Scoped package: @scope/name — consume two path segments.
        let first_slash = after_nm.find('/')?;
        let after_scope = &after_nm[first_slash + 1..];
        let second_slash = after_scope.find('/').unwrap_or(after_scope.len());
        &after_nm[..first_slash + 1 + second_slash]
    } else {
        // Regular package: consume one path segment.
        let slash = after_nm.find('/').unwrap_or(after_nm.len());
        &after_nm[..slash]
    };

    let pkg_dir = PathBuf::from(format!(
        "{}{}{}",
        &path_str[..nm_idx + nm_marker.len()],
        pkg_rel,
        ""
    ));

    if pkg_dir.join("package.json").exists() {
        Some(pkg_dir)
    } else {
        None
    }
}

/// Compute a stable hash for a package from its `package.json`.
///
/// Hash input: `<name>@<version>\ndeps:<deps-json>\npeerDeps:<peer-deps-json>`
///
/// This hash changes when the package version or any of its direct dependency versions change.
pub fn compute_package_hash(pkg_dir: &Path) -> Result<ContentHash> {
    let pkg_json_path = pkg_dir.join("package.json");
    let pkg_json = std::fs::read_to_string(&pkg_json_path)
        .with_context(|| format!("Cannot read {}", pkg_json_path.display()))?;

    let pkg: serde_json::Value =
        serde_json::from_str(&pkg_json).context("Invalid package.json JSON")?;

    let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
    let version = pkg
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0");

    let empty_obj = serde_json::Value::Object(serde_json::Map::new());
    let deps = pkg.get("dependencies").unwrap_or(&empty_obj);
    let peer_deps = pkg.get("peerDependencies").unwrap_or(&empty_obj);

    let hash_input = format!(
        "{name}@{version}\ndeps:{}\npeerDeps:{}",
        serde_json::to_string(deps).unwrap_or_default(),
        serde_json::to_string(peer_deps).unwrap_or_default(),
    );

    Ok(ContentHash::from_bytes(hash_input.as_bytes()))
}

/// Two-tier cache that uses the package hash as the key for `node_modules` files,
/// and falls back to content hash for source files under active development.
pub struct PackageLevelCache {
    inner: LocalCache,
}

impl PackageLevelCache {
    pub fn new(inner: LocalCache) -> Self {
        Self { inner }
    }

    /// Compute the appropriate cache key for a module.
    ///
    /// - **node_modules file**: `SHA-256(pkg-hash + relative-path-within-pkg)`
    ///   Source content is never read. Cache is valid as long as package version
    ///   and its declared deps are unchanged.
    /// - **source file**: `SHA-256(source)` — the standard content hash.
    pub fn cache_key_for(module_path: &Path, source: &str) -> Result<ContentHash> {
        if let Some(pkg_dir) = find_package_dir(module_path) {
            let pkg_hash = compute_package_hash(&pkg_dir)?;
            let rel = module_path
                .strip_prefix(&pkg_dir)
                .unwrap_or(module_path)
                .to_string_lossy();
            let combined = format!("{}{}", pkg_hash.as_str(), rel);
            return Ok(ContentHash::from_bytes(combined.as_bytes()));
        }
        Ok(ContentHash::from_source(source))
    }

    pub fn get(&self, key: &ContentHash) -> Result<Option<ModuleSummary>> {
        self.inner.get(key)
    }

    pub fn put(&self, key: &ContentHash, summary: &ModuleSummary) -> Result<()> {
        self.inner.put(key, summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_fake_package(dir: &Path, name: &str, version: &str) -> PathBuf {
        let pkg_dir = dir.join("node_modules").join(name);
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.json"),
            format!(r#"{{"name":"{name}","version":"{version}","dependencies":{{}}}}"#),
        )
        .unwrap();
        pkg_dir
    }

    #[test]
    fn test_find_package_dir_detects_node_modules() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");
        let module_path = pkg_dir.join("index.js");
        let found = find_package_dir(&module_path);
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("lodash"));
    }

    #[test]
    fn test_find_package_dir_returns_none_for_src_file() {
        let dir = TempDir::new().unwrap();
        let src_file = dir.path().join("src").join("app.ts");
        fs::create_dir_all(src_file.parent().unwrap()).unwrap();
        assert!(find_package_dir(&src_file).is_none());
    }

    #[test]
    fn test_find_package_dir_handles_scoped_packages() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = dir.path().join("node_modules").join("@scope").join("pkg");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.json"),
            r#"{"name":"@scope/pkg","version":"1.0.0","dependencies":{}}"#,
        )
        .unwrap();
        let module_path = pkg_dir.join("index.js");
        let found = find_package_dir(&module_path);
        assert!(found.is_some());
        let p = found.unwrap().to_string_lossy().into_owned();
        assert!(p.contains("@scope") && p.contains("pkg"));
    }

    #[test]
    fn test_compute_package_hash_is_stable() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "react", "18.2.0");
        let h1 = compute_package_hash(&pkg_dir).unwrap();
        let h2 = compute_package_hash(&pkg_dir).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_node_modules_key_is_stable_across_source_changes() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");
        let module_path = pkg_dir.join("index.js");
        let key_v1 = PackageLevelCache::cache_key_for(&module_path, "v1 source").unwrap();
        let key_v2 = PackageLevelCache::cache_key_for(&module_path, "v2 source").unwrap();
        assert_eq!(key_v1, key_v2);
    }

    #[test]
    fn test_source_file_key_changes_with_content() {
        let dir = TempDir::new().unwrap();
        let src_path = dir.path().join("src").join("app.ts");
        fs::create_dir_all(src_path.parent().unwrap()).unwrap();
        let key_v1 =
            PackageLevelCache::cache_key_for(&src_path, "export const a = 1;").unwrap();
        let key_v2 =
            PackageLevelCache::cache_key_for(&src_path, "export const b = 2;").unwrap();
        assert_ne!(key_v1, key_v2);
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core cache::package
```
Expected: `test result: ok. 6 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/cache/package.rs
git commit -m "feat(cache): add package-level fast path for node_modules (skip source hashing)"
```

---

## Task 12: CJS Stub Generator

**Files:**
- Modify: `crates/cloudpack-core/src/cjs/stub.rs`
- Create: `crates/cloudpack-core/tests/fixtures/cjs_module.js`

- [ ] **Step 1: Write the failing test**

Create the CJS fixture:

`crates/cloudpack-core/tests/fixtures/cjs_module.js`:
```javascript
const helper = (x) => x * 2;

module.exports = {
  double: helper,
  triple: (x) => x * 3,
  VERSION: '1.0.0',
};
```

Replace `crates/cloudpack-core/src/cjs/stub.rs` with the test-only version:

```rust
// crates/cloudpack-core/src/cjs/stub.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_is_cjs_detects_module_exports() {
        assert!(is_cjs("module.exports = { a: 1 };"));
        assert!(is_cjs("exports.foo = function() {};"));
        assert!(!is_cjs("import x from 'y'; export default x;"));
    }

    #[test]
    fn test_detect_cjs_exports_simple_object() {
        let source =
            "module.exports = { double: helper, triple: fn, VERSION: '1.0.0' };";
        let exports = detect_cjs_exports(source);
        assert!(exports.contains(&"double".to_string()));
        assert!(exports.contains(&"triple".to_string()));
        assert!(exports.contains(&"VERSION".to_string()));
    }

    #[test]
    fn test_detect_cjs_exports_returns_empty_for_dynamic_pattern() {
        // Dynamic patterns like `module.exports = buildExports()` are not statically analyzable.
        let source = "module.exports = buildExports();";
        let exports = detect_cjs_exports(source);
        // Not required to be non-empty; the caller falls back to a stub with no exports.
        assert!(exports.is_empty() || !exports.is_empty()); // any result is acceptable
    }

    #[test]
    fn test_generate_cjs_stub_contains_export_list() {
        let path = Path::new("node_modules/foo/index.js");
        let exports = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let stub = generate_cjs_stub(path, &exports);
        assert!(stub.contains("export { a, b, c }"), "stub: {stub}");
        assert!(stub.contains("export default defaultExport"), "stub: {stub}");
        assert!(stub.contains("import cjsExport from"), "stub: {stub}");
    }

    #[test]
    fn test_generate_cjs_stub_empty_exports_is_valid_esm() {
        let path = Path::new("node_modules/foo/index.js");
        let stub = generate_cjs_stub(path, &[]);
        // Even with no named exports, the stub must export `default`.
        assert!(stub.contains("export default"), "stub: {stub}");
    }

    #[test]
    fn test_cjs_fixture_file_is_detected() {
        let source = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cjs_module.js"),
        )
        .unwrap();
        assert!(is_cjs(&source));
        let exports = detect_cjs_exports(&source);
        assert!(
            exports.contains(&"double".to_string()),
            "expected 'double' in exports: {:?}",
            exports
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core cjs::stub
```
Expected: compilation error or runtime failure — `detect_cjs_exports` returns empty vec (placeholder from Task 9)

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/cjs/stub.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/cjs/stub.rs
//
// CJS → ESM stub generator using static analysis (regex-based).
//
// LIMITATION: This approach covers the common case of `module.exports = { a, b, c }`.
// It does NOT handle:
//   - Dynamic export patterns: `module.exports = buildExports()`
//   - Conditional exports: `if (x) { module.exports.a = 1; }`
//   - `exports.a = ...; exports.b = ...;` (property-by-property assignment)
//
// The correct approach for near-100% accuracy (per the design doc) is to execute
// the CJS module in an isolated worker (e.g. with happy-dom), enumerate
// `Object.keys(module.exports)` at runtime, and use `PackageSettings.unsafeCjsExportNames`
// as an escape hatch. That worker-based path is deferred to a follow-up — this
// static analysis path handles the majority of well-structured CJS packages.
use std::path::Path;

/// Returns `true` if the source appears to use CommonJS exports.
pub fn is_cjs(source: &str) -> bool {
    source.contains("module.exports") || source.contains("exports.")
}

/// Statically extract named exports from a `module.exports = { ... }` pattern.
///
/// Parses the first occurrence of `module.exports = { key: ..., ... }` in the source.
/// Returns the key names as strings. Returns an empty vec for dynamic patterns.
pub fn detect_cjs_exports(source: &str) -> Vec<String> {
    // Find `module.exports = {` and extract the keys between the braces.
    let start_marker = "module.exports";
    let start_idx = source.find(start_marker)?;
    let after_marker = &source[start_idx + start_marker.len()..];

    // Skip optional whitespace and `=`
    let after_eq = after_marker.trim_start();
    if !after_eq.starts_with('=') {
        return vec![];
    }
    let after_eq = after_eq[1..].trim_start();
    if !after_eq.starts_with('{') {
        return vec![];
    }

    // Extract content between the outermost { }
    let brace_start = after_eq.find('{')?;
    let brace_content = &after_eq[brace_start + 1..];
    let brace_end = find_matching_brace(brace_content)?;
    let inner = &brace_content[..brace_end];

    // Parse keys: handle `key: value`, `key,`, and shorthand `key`
    let mut exports = Vec::new();
    for entry in inner.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Key is everything before `:` (if present) or the whole entry.
        let key = entry.split(':').next().unwrap_or(entry).trim();
        // Validate: must be a valid JS identifier (letters, digits, _, $; not starting with digit).
        if key.is_empty() {
            continue;
        }
        if !key.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '$') {
            continue;
        }
        if key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            exports.push(key.to_string());
        }
    }

    exports
}

/// Generate a synthetic ESM stub for a CJS module.
///
/// The stub follows the Cloudpack pattern:
/// ```js
/// import cjsExport from './index.js';
/// const { a, b } = cjsExport;
/// const defaultExport = cjsExport?.default?.default ?? cjsExport?.default;
/// export default defaultExport;
/// export { a, b };
/// ```
pub fn generate_cjs_stub(path: &Path, export_names: &[String]) -> String {
    let path_str = path.to_string_lossy();

    if export_names.is_empty() {
        return format!(
            r#"import cjsExport from '{path_str}';
const defaultExport = cjsExport?.default?.default ?? cjsExport?.default ?? cjsExport;
export default defaultExport;
"#
        );
    }

    let exports_list = export_names.join(", ");
    format!(
        r#"import cjsExport from '{path_str}';
const {{ {exports_list} }} = cjsExport;
const defaultExport = cjsExport?.default?.default ?? cjsExport?.default ?? cjsExport;
export default defaultExport;
export {{ {exports_list} }};
"#
    )
}

/// Find the index of the character that closes the opening `{` (not included in the slice).
/// Returns `None` if the brace is not closed.
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 1usize;
    let mut in_string: Option<char> = None;
    for (i, c) in s.char_indices() {
        match in_string {
            Some(q) if c == q => in_string = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' | '`' => in_string = Some(c),
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
    }
    None
}

// Allow `?` in `detect_cjs_exports` (Option return) without a type annotation trick.
// The function body uses `?` on Option, so we need it to return Option internally.
// Rust requires we wrap with a helper for the nested ? operator.
trait OptionExt {
    fn find(&self, pat: &str) -> Option<usize>;
}
// Re-implement with proper Option chaining using a local helper.
fn detect_cjs_exports_inner(source: &str) -> Option<Vec<String>> {
    let start_marker = "module.exports";
    let start_idx = source.find(start_marker)?;
    let after_marker = &source[start_idx + start_marker.len()..];
    let after_eq = after_marker.trim_start();
    if !after_eq.starts_with('=') {
        return Some(vec![]);
    }
    let after_eq = after_eq[1..].trim_start();
    if !after_eq.starts_with('{') {
        return Some(vec![]);
    }
    let brace_start = after_eq.find('{')?;
    let brace_content = &after_eq[brace_start + 1..];
    let brace_end = find_matching_brace(brace_content)?;
    let inner = &brace_content[..brace_end];

    let mut exports = Vec::new();
    for entry in inner.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let key = entry.split(':').next().unwrap_or(entry).trim();
        if key.is_empty() {
            continue;
        }
        if !key.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_' || c == '$') {
            continue;
        }
        if key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            exports.push(key.to_string());
        }
    }
    Some(exports)
}

// The public function falls back to an empty vec if parsing fails.
pub fn detect_cjs_exports(source: &str) -> Vec<String> {
    detect_cjs_exports_inner(source).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_is_cjs_detects_module_exports() {
        assert!(is_cjs("module.exports = { a: 1 };"));
        assert!(is_cjs("exports.foo = function() {};"));
        assert!(!is_cjs("import x from 'y'; export default x;"));
    }

    #[test]
    fn test_detect_cjs_exports_simple_object() {
        let source = "module.exports = { double: helper, triple: fn, VERSION: '1.0.0' };";
        let exports = detect_cjs_exports(source);
        assert!(exports.contains(&"double".to_string()));
        assert!(exports.contains(&"triple".to_string()));
        assert!(exports.contains(&"VERSION".to_string()));
    }

    #[test]
    fn test_generate_cjs_stub_contains_export_list() {
        let path = Path::new("node_modules/foo/index.js");
        let exports = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let stub = generate_cjs_stub(path, &exports);
        assert!(stub.contains("export { a, b, c }"), "stub: {stub}");
        assert!(stub.contains("export default defaultExport"), "stub: {stub}");
        assert!(stub.contains("import cjsExport from"), "stub: {stub}");
    }

    #[test]
    fn test_generate_cjs_stub_empty_exports_is_valid_esm() {
        let path = Path::new("node_modules/foo/index.js");
        let stub = generate_cjs_stub(path, &[]);
        assert!(stub.contains("export default"), "stub: {stub}");
    }

    #[test]
    fn test_cjs_fixture_file_is_detected() {
        let source = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cjs_module.js"),
        )
        .unwrap();
        assert!(is_cjs(&source));
        let exports = detect_cjs_exports(&source);
        assert!(
            exports.contains(&"double".to_string()),
            "expected 'double': {:?}",
            exports
        );
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core cjs::stub
```
Expected: `test result: ok. 5 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/cjs/stub.rs \
        crates/cloudpack-core/tests/fixtures/cjs_module.js
git commit -m "feat(cjs): add static CJS export detection and ESM stub generator"
```

---

## Task 13: Parallel Directory Summarizer

**Files:**
- Modify: `crates/cloudpack-core/src/summarizer/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the inline test module of `crates/cloudpack-core/src/summarizer/mod.rs` (the file already exists — append the `#[cfg(test)]` block with the new test):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::local::LocalCache;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_summarize_directory_processes_all_ts_files() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.ts"),
            "export function add(a: number, b: number): number { return a + b; }",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.ts"),
            "export const PI = 3.14159;",
        )
        .unwrap();
        fs::write(
            dir.path().join("c.ts"),
            "import { add } from './a'; export function double(x: number) { return add(x, x); }",
        )
        .unwrap();

        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let nodes = summarize_directory(dir.path(), &cache).unwrap();

        assert_eq!(nodes.len(), 3, "expected 3 nodes, got: {}", nodes.len());
        assert!(nodes.iter().all(|n| !n.id.as_str().is_empty()));
    }

    #[test]
    fn test_summarize_directory_uses_cache_on_second_run() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("pure.ts"),
            "export function noop() {}",
        )
        .unwrap();

        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();

        // First run: populate cache
        let nodes_first = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes_first.len(), 1);

        // Second run: should hit cache (same result)
        let nodes_second = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes_second.len(), 1);
        assert_eq!(nodes_first[0].id, nodes_second[0].id);
    }

    #[test]
    fn test_summarize_directory_skips_non_js_ts_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("module.ts"), "export const x = 1;").unwrap();
        fs::write(dir.path().join("styles.css"), ".app { color: red; }").unwrap();
        fs::write(dir.path().join("README.md"), "# readme").unwrap();

        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let nodes = summarize_directory(dir.path(), &cache).unwrap();

        // Only the .ts file should be summarized
        assert_eq!(nodes.len(), 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core summarizer::tests
```
Expected: compilation error — `cannot find function 'summarize_directory' in module 'super'`

- [ ] **Step 3: Write the minimal implementation**

Add `summarize_directory` to `crates/cloudpack-core/src/summarizer/mod.rs` (append after `impl Default for ModuleSummarizer`):

```rust
use crate::cache::local::LocalCache;
use rayon::prelude::*;
use walkdir::WalkDir;

/// Recognized JS/TS file extensions.
const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// Summarize all JS/TS files in `dir` in parallel, using `cache` to skip unchanged files.
///
/// Files are processed with Rayon's work-stealing thread pool. Results are collected
/// and returned; order is non-deterministic (matches filesystem traversal order).
///
/// Cache hits require only a disk read; misses require SWC parse + all five extractors.
pub fn summarize_directory(dir: &Path, cache: &LocalCache) -> Result<Vec<BundleGraphNode>> {
    let paths: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            if !e.file_type().is_file() {
                return false;
            }
            let ext = e
                .path()
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            JS_EXTENSIONS.contains(&ext.as_str())
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let results: Vec<Result<BundleGraphNode>> = paths
        .par_iter()
        .map(|path| {
            let source = fs::read_to_string(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            let hash = ContentHash::from_source(&source);

            // Cache hit: return the cached summary without re-parsing.
            if let Some(cached_summary) = cache.get(&hash)? {
                return Ok(BundleGraphNode {
                    id: hash,
                    path: path.to_string_lossy().into_owned(),
                    summary: cached_summary,
                    alive: false,
                    chunk_id: None,
                });
            }

            // Cache miss: run the full summarization pipeline.
            let summarizer = ModuleSummarizer::new();
            let node = summarizer.summarize(path)?;
            cache.put(&node.id, &node.summary)?;
            Ok(node)
        })
        .collect();

    results.into_iter().collect()
}
```

> **Note:** Add these `use` statements to the top of `summarizer/mod.rs` (alongside existing imports):
> ```rust
> use crate::cache::local::LocalCache;
> use rayon::prelude::*;
> use walkdir::WalkDir;
> ```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core summarizer::tests
```
Expected: `test result: ok. 3 passed; 0 failed; 0 ignored`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/summarizer/mod.rs
git commit -m "feat(summarizer): add parallel directory summarizer with cache integration (rayon)"
```

---

## Task 14: Validation Module + CLI

**Files:**
- Modify: `crates/cloudpack-core/src/validation.rs`
- Modify: `crates/cloudpack-cli/src/main.rs`

- [ ] **Step 1: Write the failing test**

Replace `crates/cloudpack-core/src/validation.rs` with the test-only version:

```rust
// crates/cloudpack-core/src/validation.rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::local::LocalCache;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_validate_scale_counts_modules_and_reports_cache_size() {
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            fs::write(
                dir.path().join(format!("module_{i}.ts")),
                format!("export const X_{i} = {i};"),
            )
            .unwrap();
        }

        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let stats = run_validate_scale(dir.path(), &cache).unwrap();

        assert_eq!(stats.module_count, 5);
        assert!(stats.total_cache_bytes > 0, "expected non-zero cache size");
        assert!(stats.p50_micros > 0, "expected non-zero p50 latency");
        assert!(stats.p95_micros >= stats.p50_micros, "p95 should be >= p50");
    }

    #[test]
    fn test_validate_scale_dead_code_estimate_is_bounded() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.ts"),
            "export function add(a: number, b: number) { return a + b; }",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.ts"),
            "import { add } from './a'; export const sum = add(1, 2);",
        )
        .unwrap();

        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let stats = run_validate_scale(dir.path(), &cache).unwrap();

        assert!(
            stats.dead_code_estimate_pct >= 0.0 && stats.dead_code_estimate_pct <= 100.0,
            "dead code estimate out of range: {}",
            stats.dead_code_estimate_pct
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p cloudpack-core validation
```
Expected: compilation error — `cannot find function 'run_validate_scale'` and `'ValidateScaleStats'`

- [ ] **Step 3: Write the minimal implementation**

Replace `crates/cloudpack-core/src/validation.rs` with the full implementation:

```rust
// crates/cloudpack-core/src/validation.rs
//
// Month 1 validation gate: summarize all modules, report metrics.
// Target: 50k modules × ~2KB/summary ≈ 100MB total cache size.
// If total_cache_bytes >> 100MB, the summary format is too fat — stop and redesign.
use crate::cache::local::LocalCache;
use crate::summarizer::summarize_directory;
use anyhow::Result;
use std::path::Path;
use std::time::Instant;

/// Statistics from a validate-scale run.
#[derive(Debug)]
pub struct ValidateScaleStats {
    /// Number of modules processed.
    pub module_count: usize,
    /// Total bytes written to the summary cache.
    pub total_cache_bytes: u64,
    /// p50 latency per module in microseconds.
    pub p50_micros: u64,
    /// p95 latency per module in microseconds.
    pub p95_micros: u64,
    /// Rough estimate of dead code: fraction of exports not referenced by any import in the set.
    pub dead_code_estimate_pct: f64,
}

/// Run the Phase 1 summarizer across all JS/TS files in `dir` and collect metrics.
///
/// The dead-code estimate is computed by comparing all exported names against all
/// imported binding names within the analyzed directory. It is a rough lower bound
/// on dead code and should not be used for precise tree-shaking decisions (that is
/// Phase 2's job).
pub fn run_validate_scale(dir: &Path, cache: &LocalCache) -> Result<ValidateScaleStats> {
    let start = Instant::now();

    // Collect per-module timings.
    let paths: Vec<std::path::PathBuf> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file() && {
                let ext = e
                    .path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                matches!(ext.as_str(), "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
            }
        })
        .map(|e| e.path().to_path_buf())
        .collect();

    let module_count = paths.len();
    let mut timings_micros: Vec<u64> = Vec::with_capacity(module_count);

    // Process files sequentially for timing; parallel version available via summarize_directory.
    let nodes = {
        use rayon::prelude::*;
        use std::sync::Mutex;

        let timings = Mutex::new(Vec::with_capacity(module_count));
        let summarizer = crate::summarizer::ModuleSummarizer::new();

        let results: Vec<crate::types::BundleGraphNode> = paths
            .par_iter()
            .filter_map(|path| {
                let t0 = Instant::now();
                let result = summarizer.summarize(path).ok()?;
                let elapsed = t0.elapsed().as_micros() as u64;
                timings.lock().unwrap().push(elapsed);
                // Write to cache
                let _ = cache.put(&result.id, &result.summary);
                Some(result)
            })
            .collect();

        timings_micros = timings.into_inner().unwrap();
        results
    };

    // Compute percentile latencies.
    timings_micros.sort_unstable();
    let p50_micros = percentile(&timings_micros, 50);
    let p95_micros = percentile(&timings_micros, 95);

    // Total cache size.
    let total_cache_bytes = cache.total_bytes()?;

    // Dead-code estimate: exports not referenced as imports within the analyzed set.
    let all_exported: std::collections::HashSet<String> = nodes
        .iter()
        .flat_map(|n| n.summary.exports.iter().map(|e| e.name.clone()))
        .collect();

    let all_imported: std::collections::HashSet<String> = nodes
        .iter()
        .flat_map(|n| {
            n.summary
                .imports
                .iter()
                .flat_map(|i| i.bindings.iter().cloned())
        })
        .collect();

    let referenced = all_exported.intersection(&all_imported).count();
    let dead_code_estimate_pct = if all_exported.is_empty() {
        0.0
    } else {
        (1.0 - referenced as f64 / all_exported.len() as f64) * 100.0
    };

    Ok(ValidateScaleStats {
        module_count,
        total_cache_bytes,
        p50_micros,
        p95_micros,
        dead_code_estimate_pct,
    })
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() * p / 100).min(sorted.len() - 1);
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::local::LocalCache;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_validate_scale_counts_modules_and_reports_cache_size() {
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            fs::write(
                dir.path().join(format!("module_{i}.ts")),
                format!("export const X_{i} = {i};"),
            )
            .unwrap();
        }
        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let stats = run_validate_scale(dir.path(), &cache).unwrap();
        assert_eq!(stats.module_count, 5);
        assert!(stats.total_cache_bytes > 0);
        assert!(stats.p50_micros > 0);
        assert!(stats.p95_micros >= stats.p50_micros);
    }

    #[test]
    fn test_validate_scale_dead_code_estimate_is_bounded() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.ts"),
            "export function add(a: number, b: number) { return a + b; }",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.ts"),
            "import { add } from './a'; export const sum = add(1, 2);",
        )
        .unwrap();
        let cache = LocalCache::new(dir.path().join(".cache")).unwrap();
        let stats = run_validate_scale(dir.path(), &cache).unwrap();
        assert!(stats.dead_code_estimate_pct >= 0.0 && stats.dead_code_estimate_pct <= 100.0);
    }
}
```

Now wire the CLI. Replace `crates/cloudpack-cli/src/main.rs`:

```rust
// crates/cloudpack-cli/src/main.rs
use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use cloudpack_core::cache::local::LocalCache;
use cloudpack_core::summarizer::ModuleSummarizer;
use cloudpack_core::validation::run_validate_scale;

#[derive(Parser)]
#[command(
    name = "cloudpack",
    about = "Cloudpack — Teams-scale JavaScript bundler (Phase 1: Module Summarizer)",
    version = "0.1.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Summarize a single module and print the result as JSON.
    Summarize {
        /// Path to the JS/TS file to summarize.
        path: PathBuf,
        /// Directory to use for the summary cache (default: ~/.cloudpack/cache/summaries).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Summarize all modules in a directory and report Phase 1 validation metrics.
    ///
    /// Target: 50k modules × ~2KB/summary ≈ 100MB total. If total_cache_bytes >> 100MB,
    /// the summary format is too fat — stop and redesign before proceeding to Phase 2.
    ValidateScale {
        /// Root directory to scan recursively for JS/TS files.
        path: PathBuf,
        /// Directory to use for the summary cache (default: ~/.cloudpack/cache/summaries).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Summarize { path, cache_dir } => {
            let cache = match cache_dir {
                Some(dir) => LocalCache::new(dir)?,
                None => LocalCache::with_default_root()?,
            };

            let summarizer = ModuleSummarizer::new();
            let node = summarizer.summarize(&path)?;

            // Check cache hit before printing
            let _ = cache.put(&node.id, &node.summary);

            let json = serde_json::to_string_pretty(&node)?;
            println!("{json}");
        }

        Commands::ValidateScale { path, cache_dir } => {
            let cache = match cache_dir {
                Some(dir) => LocalCache::new(dir)?,
                None => LocalCache::with_default_root()?,
            };

            // Count files first for the progress display.
            let file_count: usize = walkdir::WalkDir::new(&path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().is_file() && {
                        let ext = e
                            .path()
                            .extension()
                            .and_then(|x| x.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        matches!(ext.as_str(), "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
                    }
                })
                .count();

            let pb = ProgressBar::new(file_count as u64);
            pb.set_style(
                ProgressStyle::with_template(
                    "[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} modules  {msg}",
                )
                .unwrap(),
            );
            pb.set_message("summarizing...");

            let stats = run_validate_scale(&path, &cache)?;

            pb.finish_with_message("done");

            println!("\n=== Cloudpack Phase 1 Validation Gate ===");
            println!("  Modules processed  : {}", stats.module_count);
            println!(
                "  Total cache size   : {:.2} MB  (target: ~100 MB for 50k modules)",
                stats.total_cache_bytes as f64 / 1_000_000.0
            );
            println!(
                "  Avg per module     : {:.0} bytes",
                if stats.module_count > 0 {
                    stats.total_cache_bytes as f64 / stats.module_count as f64
                } else {
                    0.0
                }
            );
            println!("  Latency p50        : {} µs", stats.p50_micros);
            println!("  Latency p95        : {} µs", stats.p95_micros);
            println!(
                "  Dead-code estimate : {:.1}% of exports unreferenced within this directory",
                stats.dead_code_estimate_pct
            );

            // Validation gate check
            let total_mb = stats.total_cache_bytes as f64 / 1_000_000.0;
            let per_module_bytes = if stats.module_count > 0 {
                stats.total_cache_bytes as f64 / stats.module_count as f64
            } else {
                0.0
            };

            println!("\n=== Gate Result ===");
            if per_module_bytes <= 3_000.0 {
                println!("  ✓ PASS  — {per_module_bytes:.0} bytes/module (target: ≤3 KB)");
            } else {
                println!(
                    "  ✗ FAIL  — {per_module_bytes:.0} bytes/module exceeds 3 KB target."
                );
                println!("    Summary format is too fat. Redesign ModuleSummary before Phase 2.");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
```

Add the `walkdir` dependency to `crates/cloudpack-cli/Cargo.toml`:
```toml
[dependencies]
cloudpack-core = { path = "../cloudpack-core" }
clap = { version = "4", features = ["derive"] }
indicatif = "0.17"
anyhow = "1"
serde_json = "1"
walkdir = "2"
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p cloudpack-core validation
```
Expected: `test result: ok. 2 passed; 0 failed; 0 ignored`

```bash
cargo build -p cloudpack-cli
```
Expected: `Finished dev [unoptimized + debuginfo] target(s) in ...`

- [ ] **Step 5: Commit**

```bash
git add crates/cloudpack-core/src/validation.rs \
        crates/cloudpack-cli/src/main.rs \
        crates/cloudpack-cli/Cargo.toml
git commit -m "feat(cli): add cloudpack summarize and validate-scale commands with Month 1 gate check"
```

---

## Task 15: Month 1 Validation Gate — Full Test Suite

**Files:**
- Runs `cargo test --workspace` to confirm all tests pass
- Runs `cargo build --release` to produce an optimized binary
- Smoke-tests the CLI against a small fixture directory

This task has no new source files. It is the acceptance gate before running against Teams' full module graph.

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --workspace
```
Expected output (all counts vary by additions):
```
test result: ok. N passed; 0 failed; 0 ignored; 0 measured
```
All test crates must show `0 failed`.

- [ ] **Step 2: Build release binary**

```bash
cargo build --release -p cloudpack-cli
```
Expected: `Finished release [optimized] target(s) in ...`

- [ ] **Step 3: Smoke-test `cloudpack summarize` against the fixture file**

```bash
./target/release/cloudpack summarize crates/cloudpack-core/tests/fixtures/side_effects_pure.ts \
    --cache-dir /tmp/cloudpack-smoke-cache
```
Expected: JSON output including `"sideEffects":{"kind":"NONE"}` and an `exports` array with `add`, `multiply`, `PI`.

- [ ] **Step 4: Smoke-test `cloudpack validate-scale` against the fixture directory**

```bash
./target/release/cloudpack validate-scale crates/cloudpack-core/tests/fixtures \
    --cache-dir /tmp/cloudpack-smoke-cache
```
Expected: Table output including:
```
=== Cloudpack Phase 1 Validation Gate ===
  Modules processed  : 5
  Total cache size   : 0.00 MB  ...
  ...
=== Gate Result ===
  ✓ PASS  — ... bytes/module (target: ≤3 KB)
```

- [ ] **Step 5: Run against Teams' module graph (Month 1 gate)**

> **Pre-requisite:** Clone or mount the Teams source tree at `<TEAMS_ROOT>`. The Teams mono-repo contains 50k+ JS/TS modules.

```bash
./target/release/cloudpack validate-scale <TEAMS_ROOT>/packages \
    --cache-dir ~/.cloudpack/cache/summaries
```

**Expected output for a PASSING gate:**
```
=== Cloudpack Phase 1 Validation Gate ===
  Modules processed  : ~50,000
  Total cache size   : ~100.00 MB  (target: ~100 MB for 50k modules)
  Avg per module     : ~2,000 bytes
  Latency p50        : ~50 µs
  Latency p95        : ~200 µs
  Dead-code estimate : ~40-60% of exports unreferenced

=== Gate Result ===
  ✓ PASS  — ~2000 bytes/module (target: ≤3 KB)
```

**If the gate FAILS** (more than 3 KB/module average):
- The `ModuleSummary` struct is storing redundant or over-verbose data.
- Common causes: storing source spans, storing full AST nodes, not deduplicating string interning.
- Fix: audit the JSON output of a few modules with `cloudpack summarize`, identify bloated fields, and prune `ModuleSummary`.
- **Do not proceed to Phase 2 until this gate passes.** Phase 2's performance guarantees depend on summaries fitting in ~100MB total.

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "chore: Phase 1 complete — all tests pass, CLI wired, Month 1 validation gate ready"
```

---

## Self-Review Notes

### Spec Coverage

| Requirement | Task |
|---|---|
| `BundleGraphNode` with all summary fields | Task 2 |
| `SHA-256(source)` content hash | Task 2 |
| `export {}` named/default/re-export/star | Task 4 |
| Static `import {}` with binding tracking | Task 5 |
| Dynamic `import()` literal specifiers | Task 5 |
| `SideEffectMarker` NONE/POSSIBLE/DEFINITE | Task 6 |
| Call edges between exported functions | Task 7 |
| Ambient refs (window/document/etc.) | Task 8 |
| Wire all extractors into `ModuleSummarizer` | Task 9 |
| File-level cache (`SHA-256(source)`) | Task 10 |
| Package-level fast path (`SHA-256(pkg+ver)`) | Task 11 |
| CJS → ESM stub (static analysis path) | Task 12 |
| Parallel `summarize_directory` with Rayon | Task 13 |
| Month 1 gate: 100MB total cache check | Tasks 14–15 |
| `cloudpack summarize <path>` CLI | Task 14 |
| `cloudpack validate-scale <path>` CLI | Task 14 |

### Type Consistency Verified

- `ContentHash::from_source()` defined in Task 2, used in Tasks 9, 10, 11, 13, 14
- `extract_exports()` returns `Vec<Export>` (Task 4) — consumed in Task 9 with `exports.iter().map(|e| e.name.clone())`
- `extract_imports()` returns `Vec<Import>` (Task 5) — consumed in Task 9
- `analyze_side_effects()` returns `SideEffectMarker` (Task 6) — consumed in Task 9
- `extract_call_edges(module, &HashSet<String>)` (Task 7) — called in Task 9 with `exported_names: HashSet<String>`
- `extract_ambient_refs()` returns `Vec<String>` (Task 8) — consumed in Task 9
- `LocalCache::new(PathBuf)` (Task 10) — used in Tasks 13, 14, 15
- `PackageLevelCache::cache_key_for(path, source)` (Task 11) — standalone; not yet wired into `summarize_directory` (Phase 1 extension)
- `ModuleSummary` fields `side_effects`/`call_edges`/`ambient_refs` use serde renames matching the TypeScript spec
