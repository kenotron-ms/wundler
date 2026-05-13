# Graph Analyzer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Plan:** 2 of 5
**Date:** 2026-05-13
**Owner:** Wundler core
**Status:** Ready to execute
**Depends on:** Plan 1 (Summarizer, `wundler-core`)

**Goal:** Take the `Vec<BundleGraphNode>` produced by the Summarizer and turn it into a chunked, reachability-pruned, dead-code-eliminated `ChunkManifest`. The Analyzer is the second stage of the Wundler pipeline; its output is the immutable description that downstream emitters consume.

**Architecture:** A new crate `wundler-graph` with five focused modules — `types` (chunk/manifest DTOs), `graph` (adjacency builder + Tarjan SCC), `reachability` (BFS pruning), `dce` (call-edge dead-export detection), `chunks` (route-based splitter + commons extraction), and `manifest` (assembly + content hashing). One top-level `GraphAnalyzer::analyze()` wires them together. A `wundler analyze` CLI subcommand in `wundler-cli` exposes the pipeline at the command line.

**Tech Stack:** Rust (edition 2021), `petgraph 0.6` (Tarjan SCC, BFS traversal helpers), `sha2 0.10` + `hex 0.4` (chunk and build hashing), `serde` + `serde_json` (manifest JSON), `indexmap 2` (deterministic chunk-member ordering), `anyhow` + `thiserror` (error handling), `wundler-core` (re-uses `BundleGraphNode`, `ContentHash`, `ModuleSummary`, `Import`, `ImportKind`, `CallEdge`).

---

## Scope Boundaries

| In Scope | Out of Scope |
|---|---|
| Adjacency map: ContentHash → Vec<ContentHash> | Module resolution (handled by Summarizer / fixtures) |
| Tarjan SCC for circular import groups | Bundle-splitting heuristics beyond route-based + commons |
| BFS reachability from configured entry points | PGO-driven chunk merges (Plan 5) |
| Call-edge transitive DCE on exports | Tree-shaking inside a single export's body (Plan 3 transform) |
| Route-based INITIAL chunks (static-import BFS) | Cross-route prefetch hint scheduling |
| Dynamic import boundaries → LAZY chunks | Async chunk preload graph optimization |
| Commons extraction (modules in ≥ N chunks) | Co-request scoring (PGO, Plan 5) |
| Deterministic chunk and build hashing | Adaptive Bundle Service (Plan 4) |
| `ChunkManifest` JSON round-trip | Service worker / CDN integration |
| `wundler analyze` CLI subcommand | `wundler build` (Plan 3) |

## Pre-Flight Verification

Before starting, confirm Plan 1 state:

```bash
cd /Users/ken/workspace/ms/wundler
cargo build -p wundler-core
cargo test -p wundler-core --lib
```

Both must succeed. Verify required types exist:

```bash
grep -rn "pub struct BundleGraphNode" crates/wundler-core/src
grep -rn "pub struct ModuleSummary"   crates/wundler-core/src
grep -rn "pub struct ContentHash"     crates/wundler-core/src
grep -rn "pub struct Import"          crates/wundler-core/src
grep -rn "pub enum ImportKind"        crates/wundler-core/src
grep -rn "pub struct CallEdge"        crates/wundler-core/src
grep -rn "pub fn summarize_directory" crates/wundler-core/src
```

Each `grep` must return at least one match. If any type or function is missing, stop and resolve before continuing.

## Types Used Directly From `wundler-core` (Plan 1)

This plan does **not** redefine these — it imports them from `wundler_core::types`:

```rust
pub struct ContentHash(pub String);                   // SHA-256 hex; ContentHash::of(&str) -> Self
pub struct Export    { name, kind: ExportKind, from: Option<String> }
pub enum   ExportKind { Named, Default, Namespace, ReExport }
pub struct Import    { source: String, bindings: Vec<String>, kind: ImportKind }
pub enum   ImportKind { Static, Dynamic }
pub struct CallEdge  { from_export: String, to_export: String }
pub enum   SideEffectMarker { None, Possible { reason: String }, Definite }
pub struct ModuleSummary { exports, imports, side_effects, call_edges, ambient_refs }
pub struct BundleGraphNode { id: ContentHash, path: String, summary: ModuleSummary, alive: bool, chunk_id: Option<String> }
```

**Resolution assumption:** `Import.source` strings have been resolved by the Summarizer so they equal the `BundleGraphNode.path` of the target module. Test fixtures honor this contract. Unresolved imports (e.g. external `npm` modules) appear in `Import.source` as strings that do not match any node `path` and are silently dropped from the adjacency map (they cannot affect reachability of in-graph modules).

## Task Index

1. Register `wundler-graph` in the workspace; create skeleton crate
2. `types.rs` — `ChunkId`, `LoadCondition`, `Chunk`, `ChunkManifest` + JSON round-trip helpers
3. `graph.rs` — build path-indexed adjacency map (ContentHash → Vec<ContentHash>) from `&[BundleGraphNode]`
4. `graph.rs` — Tarjan SCC via `petgraph::algo::tarjan_scc`; detect circular import groups
5. `reachability.rs` — BFS from entry hashes following static + dynamic imports
6. `reachability.rs` — SCC-aware liveness (entire cycle alive or entire cycle dead)
7. `dce.rs` — call-edge transitive DCE; compute dead exports for alive nodes
8. `chunks.rs` — INITIAL chunks via static-import BFS per entry point
9. `chunks.rs` — LAZY chunks at every dynamic-import boundary
10. `chunks.rs` — commons extraction: modules in ≥ `commons_threshold` chunks
11. `chunks.rs` — chunk hashing: `Chunk.hash` = SHA-256 over sorted member hashes
12. `manifest.rs` — `ChunkManifest` assembly (`build_id`, `entry_chunks`, `module_index`)
13. `manifest.rs` — `to_json` / `from_json` round-trip
14. `analyzer.rs` — `GraphAnalyzer::analyze()` wires summarize-output → manifest; integration test on `multi_entry.json`
15. `wundler-cli` — `wundler analyze <dir> --entry route=path` subcommand

Each task is TDD-shaped: failing test first, run-and-confirm-failure, minimal implementation, run-and-confirm-pass, commit.

---

## Task 1 — Register `wundler-graph` in the workspace; create skeleton crate

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/wundler-graph/Cargo.toml`
- Create: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/smoke.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/smoke.rs`:

```rust
#[test]
fn crate_loads() {
    assert_eq!(wundler_graph::hello(), "wundler-graph");
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test smoke
```

Expected failure: `error: package ID specification 'wundler-graph' did not match any packages` — the crate does not exist yet.

### Step 3: Write minimal implementation

- [ ] Modify `Cargo.toml` (workspace root) so `[workspace] members` contains `crates/wundler-graph`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/wundler-core",
    "crates/wundler-cli",
    "crates/wundler-graph",
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
```

- [ ] Create `crates/wundler-graph/Cargo.toml`:

```toml
[package]
name = "wundler-graph"
version = "0.1.0"
edition = "2021"
license = "MIT"

[dependencies]
wundler-core = { path = "../wundler-core" }
petgraph = "0.6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
hex = "0.4"
indexmap = { version = "2", features = ["serde"] }
anyhow = "1"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] Create `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test smoke
```

Expected: `test result: ok. 1 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add Cargo.toml crates/wundler-graph
git commit -m "wundler-graph: register crate in workspace, add skeleton"
```

---

## Task 2 — `types.rs`: `ChunkId`, `LoadCondition`, `Chunk`, `ChunkManifest`

**Files:**
- Create: `crates/wundler-graph/src/types.rs`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/types_shape.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/types_shape.rs`:

```rust
use std::collections::HashMap;
use wundler_core::types::ContentHash;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

#[test]
fn chunk_construction_populates_all_fields() {
    let c = Chunk {
        id: "c0".to_string(),
        modules: vec![ContentHash::of("a"), ContentHash::of("b")],
        hash: ContentHash::of("c0-hash"),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    assert_eq!(c.id, "c0");
    assert_eq!(c.modules.len(), 2);
    assert_eq!(c.load_condition, LoadCondition::Initial);
}

#[test]
fn load_condition_round_trips_through_json() {
    for variant in [
        LoadCondition::Initial,
        LoadCondition::Lazy,
        LoadCondition::Prefetch,
    ] {
        let s = serde_json::to_string(&variant).unwrap();
        let back: LoadCondition = serde_json::from_str(&s).unwrap();
        assert_eq!(back, variant);
    }
}

#[test]
fn chunk_manifest_round_trips() {
    let chunk = Chunk {
        id: "c0".to_string(),
        modules: vec![ContentHash::of("m1")],
        hash: ContentHash::of("c0"),
        load_condition: LoadCondition::Initial,
        co_request_score: Some(0.75),
        median_load_order: Some(1.0),
        suggested_merge: None,
    };
    let mut entry_chunks = HashMap::new();
    entry_chunks.insert("/".to_string(), vec!["c0".to_string()]);
    let mut module_index = HashMap::new();
    module_index.insert(ContentHash::of("m1"), "c0".to_string());

    let manifest = ChunkManifest {
        build_id: "build-deadbeef".to_string(),
        chunks: vec![chunk],
        entry_chunks,
        module_index,
    };

    let json = manifest.to_json().unwrap();
    let parsed = ChunkManifest::from_json(&json).unwrap();
    assert_eq!(parsed.build_id, manifest.build_id);
    assert_eq!(parsed.chunks.len(), 1);
    assert_eq!(parsed.chunks[0].id, "c0");
    assert_eq!(parsed.chunks[0].co_request_score, Some(0.75));
    assert_eq!(parsed.entry_chunks.get("/").unwrap(), &vec!["c0".to_string()]);
    assert_eq!(parsed.module_index.get(&ContentHash::of("m1")).unwrap(), "c0");
}

#[test]
fn from_json_rejects_invalid_payload() {
    let result = ChunkManifest::from_json("not json at all");
    assert!(result.is_err());
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test types_shape
```

Expected failure: `error[E0432]: unresolved import wundler_graph::types` — the module does not exist yet.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/types.rs`:

```rust
//! Chunk and manifest DTOs produced by `GraphAnalyzer`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wundler_core::types::ContentHash;

/// Stable identifier for a chunk. Strings keep the manifest JSON readable.
pub type ChunkId = String;

/// Logical entry-point identifier — typically a route path like `/` or `/admin`.
pub type EntryPoint = String;

/// When the runtime should request a chunk.
///
/// * `Initial`  — part of the entry HTML payload; required for first paint.
/// * `Lazy`     — requested on demand at a `import()` boundary.
/// * `Prefetch` — speculatively requested after `Initial` chunks are loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoadCondition {
    Initial,
    Lazy,
    Prefetch,
}

/// A unit of code emission. Members are listed by their content hash; the
/// chunk's own hash is the SHA-256 of the sorted member hashes (computed in
/// Task 11 by `chunks::hash_chunk`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: ChunkId,
    pub modules: Vec<ContentHash>,
    pub hash: ContentHash,
    pub load_condition: LoadCondition,
    /// PGO-derived co-request score (Plan 5). `None` for Level-0 builds.
    pub co_request_score: Option<f64>,
    /// PGO-derived median load order (Plan 5).
    pub median_load_order: Option<f64>,
    /// PGO suggestion to merge this chunk with another (Plan 5).
    pub suggested_merge: Option<ChunkId>,
}

/// Top-level bundle-graph view. One manifest per `analyze()` invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    /// SHA-256 over the sorted entry-point paths joined with newlines.
    pub build_id: String,
    pub chunks: Vec<Chunk>,
    /// For each entry point, the ordered list of chunks needed for first paint.
    pub entry_chunks: HashMap<EntryPoint, Vec<ChunkId>>,
    /// Inverse of `Chunk.modules`: which chunk does a given module live in?
    pub module_index: HashMap<ContentHash, ChunkId>,
}

impl ChunkManifest {
    /// Serialize to a pretty-printed JSON string.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parse from a JSON string.
    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test types_shape
```

Expected: `test result: ok. 4 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: define ChunkId, LoadCondition, Chunk, ChunkManifest with JSON round-trip"
```

---

## Task 3 — `graph.rs`: build path-indexed adjacency map

**Files:**
- Create: `crates/wundler-graph/src/graph.rs`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/graph_adjacency.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/graph_adjacency.rs`:

```rust
use wundler_core::types::{
    BundleGraphNode, ContentHash, Export, ExportKind, Import, ImportKind, ModuleSummary,
    SideEffectMarker,
};
use wundler_graph::graph::build_adjacency;

fn node(path: &str, imports: Vec<(&str, ImportKind)>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::of(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .into_iter()
                .map(|(src, kind)| Import {
                    source: src.to_string(),
                    bindings: vec![],
                    kind,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
    }
}

#[test]
fn adjacency_resolves_imports_by_path() {
    let nodes = vec![
        node("a", vec![("b", ImportKind::Static), ("c", ImportKind::Dynamic)]),
        node("b", vec![]),
        node("c", vec![]),
    ];
    let adj = build_adjacency(&nodes);
    let from_a = adj.get(&ContentHash::of("a")).unwrap();
    assert!(from_a.contains(&ContentHash::of("b")));
    assert!(from_a.contains(&ContentHash::of("c")));
    assert_eq!(from_a.len(), 2);
    assert!(adj.get(&ContentHash::of("b")).unwrap().is_empty());
}

#[test]
fn adjacency_silently_drops_unresolved_imports() {
    // "external-lib" has no node — should be dropped, not panic.
    let nodes = vec![
        node("a", vec![("external-lib", ImportKind::Static), ("b", ImportKind::Static)]),
        node("b", vec![]),
    ];
    let adj = build_adjacency(&nodes);
    let from_a = adj.get(&ContentHash::of("a")).unwrap();
    assert_eq!(from_a.len(), 1);
    assert!(from_a.contains(&ContentHash::of("b")));
}

#[test]
fn adjacency_handles_empty_node_list() {
    let adj = build_adjacency(&[]);
    assert_eq!(adj.len(), 0);
}

#[test]
fn adjacency_handles_self_import() {
    // Pathological but legal: a re-exports from itself in some test fixtures.
    let nodes = vec![node("a", vec![("a", ImportKind::Static)])];
    let adj = build_adjacency(&nodes);
    let from_a = adj.get(&ContentHash::of("a")).unwrap();
    assert_eq!(from_a, &vec![ContentHash::of("a")]);
}

#[test]
fn adjacency_preserves_duplicate_edges_at_most_once() {
    // Same import target listed twice (once static, once dynamic) → adjacency
    // records the target one time; ImportKind is consulted separately in chunks.rs.
    let nodes = vec![
        node(
            "a",
            vec![("b", ImportKind::Static), ("b", ImportKind::Dynamic)],
        ),
        node("b", vec![]),
    ];
    let adj = build_adjacency(&nodes);
    let from_a = adj.get(&ContentHash::of("a")).unwrap();
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_a[0], ContentHash::of("b"));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test graph_adjacency
```

Expected failure: `error[E0432]: unresolved import wundler_graph::graph`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/graph.rs`:

```rust
//! Adjacency-map construction over `[BundleGraphNode]` and SCC detection
//! (Tarjan, via petgraph).
//!
//! The adjacency map is the input substrate for `reachability` and `chunks`.

use std::collections::{HashMap, HashSet};
use wundler_core::types::{BundleGraphNode, ContentHash};

/// Build a directed adjacency map: source-module hash → unique list of
/// imported-module hashes. Imports whose `source` does not match the `path`
/// of any node (e.g. unresolved external packages) are silently dropped.
///
/// Each `(source, target)` pair appears at most once even if multiple
/// `Import` rows reference the same target with different `ImportKind`s;
/// the static-vs-dynamic distinction is consulted directly from
/// `node.summary.imports` by the chunker, not via this map.
pub fn build_adjacency(
    nodes: &[BundleGraphNode],
) -> HashMap<ContentHash, Vec<ContentHash>> {
    let mut path_to_hash: HashMap<&str, ContentHash> =
        HashMap::with_capacity(nodes.len());
    for node in nodes {
        path_to_hash.insert(node.path.as_str(), node.id.clone());
    }

    let mut adj: HashMap<ContentHash, Vec<ContentHash>> =
        HashMap::with_capacity(nodes.len());
    for node in nodes {
        let mut seen: HashSet<ContentHash> = HashSet::new();
        let mut out: Vec<ContentHash> = Vec::new();
        for import in &node.summary.imports {
            if let Some(target) = path_to_hash.get(import.source.as_str()) {
                if seen.insert(target.clone()) {
                    out.push(target.clone());
                }
            }
        }
        adj.insert(node.id.clone(), out);
    }
    adj
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod graph;
pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test graph_adjacency
```

Expected: `test result: ok. 5 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: build_adjacency — path-indexed import map from BundleGraphNode slice"
```

---

## Task 4 — `graph.rs`: Tarjan SCC for circular import groups

**Files:**
- Modify: `crates/wundler-graph/src/graph.rs`
- Test:   `crates/wundler-graph/tests/graph_scc.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/graph_scc.rs`:

```rust
use std::collections::HashSet;
use wundler_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::graph::{build_adjacency, tarjan_sccs};

fn node(path: &str, imports: Vec<&str>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::of(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .into_iter()
                .map(|src| Import {
                    source: src.to_string(),
                    bindings: vec![],
                    kind: ImportKind::Static,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
    }
}

#[test]
fn acyclic_graph_yields_singleton_sccs() {
    let nodes = vec![
        node("a", vec!["b"]),
        node("b", vec!["c"]),
        node("c", vec![]),
    ];
    let adj = build_adjacency(&nodes);
    let sccs = tarjan_sccs(&nodes, &adj);
    assert_eq!(sccs.len(), 3);
    for scc in &sccs {
        assert_eq!(scc.len(), 1);
    }
}

#[test]
fn two_module_cycle_yields_one_scc_of_size_two() {
    let nodes = vec![
        node("cir1", vec!["cir2"]),
        node("cir2", vec!["cir1"]),
    ];
    let adj = build_adjacency(&nodes);
    let sccs = tarjan_sccs(&nodes, &adj);
    let cycle: Vec<&Vec<ContentHash>> =
        sccs.iter().filter(|s| s.len() > 1).collect();
    assert_eq!(cycle.len(), 1);
    let cycle_set: HashSet<&ContentHash> = cycle[0].iter().collect();
    assert!(cycle_set.contains(&ContentHash::of("cir1")));
    assert!(cycle_set.contains(&ContentHash::of("cir2")));
}

#[test]
fn three_module_cycle_with_external_dependency() {
    // a -> b -> c -> a, plus a -> d (no cycle)
    let nodes = vec![
        node("a", vec!["b", "d"]),
        node("b", vec!["c"]),
        node("c", vec!["a"]),
        node("d", vec![]),
    ];
    let adj = build_adjacency(&nodes);
    let sccs = tarjan_sccs(&nodes, &adj);
    let big: Vec<&Vec<ContentHash>> = sccs.iter().filter(|s| s.len() == 3).collect();
    assert_eq!(big.len(), 1);
    let s: HashSet<&ContentHash> = big[0].iter().collect();
    assert!(s.contains(&ContentHash::of("a")));
    assert!(s.contains(&ContentHash::of("b")));
    assert!(s.contains(&ContentHash::of("c")));
}

#[test]
fn empty_graph_has_no_sccs() {
    let adj = build_adjacency(&[]);
    let sccs = tarjan_sccs(&[], &adj);
    assert!(sccs.is_empty());
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test graph_scc
```

Expected failure: `error[E0432]: unresolved import wundler_graph::graph::tarjan_sccs` — the function does not exist yet.

### Step 3: Write minimal implementation

- [ ] Append to `crates/wundler-graph/src/graph.rs`:

```rust
use petgraph::algo::tarjan_scc as petgraph_tarjan;
use petgraph::graph::{DiGraph, NodeIndex};

/// Return strongly-connected components of the import graph as groups of
/// `ContentHash`. Each SCC is a `Vec<ContentHash>`; singletons (no cycle)
/// produce a 1-element Vec. Order of SCCs follows petgraph's reverse-topo
/// order; order within each SCC is implementation-defined but stable for a
/// given input.
pub fn tarjan_sccs(
    nodes: &[BundleGraphNode],
    adj: &HashMap<ContentHash, Vec<ContentHash>>,
) -> Vec<Vec<ContentHash>> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let mut graph: DiGraph<ContentHash, ()> = DiGraph::new();
    let mut idx_of: HashMap<ContentHash, NodeIndex> = HashMap::with_capacity(nodes.len());

    for node in nodes {
        let ix = graph.add_node(node.id.clone());
        idx_of.insert(node.id.clone(), ix);
    }

    for node in nodes {
        let src = idx_of[&node.id];
        if let Some(targets) = adj.get(&node.id) {
            for target in targets {
                if let Some(&dst) = idx_of.get(target) {
                    graph.add_edge(src, dst, ());
                }
            }
        }
    }

    petgraph_tarjan(&graph)
        .into_iter()
        .map(|component| {
            component
                .into_iter()
                .map(|ix| graph[ix].clone())
                .collect::<Vec<_>>()
        })
        .collect()
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test graph_scc
```

Expected: `test result: ok. 4 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: tarjan_sccs — circular-import groups via petgraph"
```

---

## Task 5 — `reachability.rs`: BFS from entry hashes (static + dynamic imports)

**Files:**
- Create: `crates/wundler-graph/src/reachability.rs`
- Create: `crates/wundler-graph/tests/fixtures/multi_entry.json`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/reachability_bfs.rs`

### Step 1: Write the failing test and fixture

- [ ] Create `crates/wundler-graph/tests/fixtures/multi_entry.json`:

```json
[
  {"id":"hash_a","path":"a","summary":{"exports":[{"name":"a","kind":"Named","from":null}],"imports":[{"source":"x1","bindings":[],"kind":"Static"},{"source":"shared","bindings":[],"kind":"Static"},{"source":"lazy_a1","bindings":[],"kind":"Dynamic"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_b","path":"b","summary":{"exports":[{"name":"b","kind":"Named","from":null}],"imports":[{"source":"y1","bindings":[],"kind":"Static"},{"source":"shared","bindings":[],"kind":"Static"},{"source":"lazy_b1","bindings":[],"kind":"Dynamic"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_c","path":"c","summary":{"exports":[{"name":"c","kind":"Named","from":null}],"imports":[{"source":"z1","bindings":[],"kind":"Static"},{"source":"z2","bindings":[],"kind":"Static"},{"source":"lazy_c1","bindings":[],"kind":"Dynamic"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_x1","path":"x1","summary":{"exports":[{"name":"x1","kind":"Named","from":null}],"imports":[{"source":"x2","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_x2","path":"x2","summary":{"exports":[{"name":"x2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_y1","path":"y1","summary":{"exports":[{"name":"y1","kind":"Named","from":null}],"imports":[{"source":"y2","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_y2","path":"y2","summary":{"exports":[{"name":"y2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_z1","path":"z1","summary":{"exports":[{"name":"z1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_z2","path":"z2","summary":{"exports":[{"name":"z2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_lazy_a1","path":"lazy_a1","summary":{"exports":[{"name":"lazy_a1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_lazy_b1","path":"lazy_b1","summary":{"exports":[{"name":"lazy_b1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_lazy_c1","path":"lazy_c1","summary":{"exports":[{"name":"lazy_c1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_shared","path":"shared","summary":{"exports":[{"name":"shared","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_cir1","path":"cir1","summary":{"exports":[{"name":"cir1","kind":"Named","from":null}],"imports":[{"source":"cir2","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_cir2","path":"cir2","summary":{"exports":[{"name":"cir2","kind":"Named","from":null}],"imports":[{"source":"cir1","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null}
]
```

> **Note:** The `id` strings are illustrative — they don't have to be valid SHA-256 hex for unit tests; they only need to be unique and stable strings.

- [ ] Create `crates/wundler-graph/tests/reachability_bfs.rs`:

```rust
use std::collections::HashSet;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::reachability::compute_reachability;

fn load_fixture(name: &str) -> Vec<BundleGraphNode> {
    let path = format!("tests/fixtures/{name}");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse fixture {path}: {e}"))
}

fn h(s: &str) -> ContentHash {
    ContentHash(s.to_string())
}

#[test]
fn bfs_visits_static_and_dynamic_imports() {
    let nodes = load_fixture("multi_entry.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_a"));
    let alive = compute_reachability(&nodes, &entries);

    // From a: x1 (static), x2 (transitive via x1), shared, lazy_a1 (dynamic)
    assert!(alive.contains(&h("hash_a")));
    assert!(alive.contains(&h("hash_x1")));
    assert!(alive.contains(&h("hash_x2")));
    assert!(alive.contains(&h("hash_shared")));
    assert!(alive.contains(&h("hash_lazy_a1")));

    // Not reachable from a alone
    assert!(!alive.contains(&h("hash_b")));
    assert!(!alive.contains(&h("hash_y1")));
    assert!(!alive.contains(&h("hash_lazy_b1")));
    assert!(!alive.contains(&h("hash_cir1")));
}

#[test]
fn multiple_entries_union_their_reachability() {
    let nodes = load_fixture("multi_entry.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_a"));
    entries.insert(h("hash_b"));
    let alive = compute_reachability(&nodes, &entries);

    assert!(alive.contains(&h("hash_a")));
    assert!(alive.contains(&h("hash_b")));
    assert!(alive.contains(&h("hash_x1")));
    assert!(alive.contains(&h("hash_y1")));
    assert!(alive.contains(&h("hash_shared")));
    // c's subtree is still unreachable
    assert!(!alive.contains(&h("hash_c")));
    assert!(!alive.contains(&h("hash_z1")));
}

#[test]
fn empty_entry_set_yields_empty_liveness() {
    let nodes = load_fixture("multi_entry.json");
    let alive = compute_reachability(&nodes, &HashSet::new());
    assert!(alive.is_empty());
}

#[test]
fn entry_with_no_imports_is_alive_alone() {
    let nodes = load_fixture("multi_entry.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_x2")); // x2 has zero imports
    let alive = compute_reachability(&nodes, &entries);
    assert_eq!(alive.len(), 1);
    assert!(alive.contains(&h("hash_x2")));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test reachability_bfs
```

Expected failure: `error[E0432]: unresolved import wundler_graph::reachability`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/reachability.rs`:

```rust
//! Liveness via BFS from entry hashes. Follows both static and dynamic imports.
//!
//! SCC-aware liveness (Task 6) is implemented as a post-processing pass.

use crate::graph::build_adjacency;
use std::collections::{HashSet, VecDeque};
use wundler_core::types::{BundleGraphNode, ContentHash};

/// Compute the set of `ContentHash`es reachable from any entry hash by
/// following imports of any kind (Static or Dynamic). Module-resolution is
/// already encoded in `build_adjacency` — unresolved imports are dropped.
pub fn compute_reachability(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
) -> HashSet<ContentHash> {
    let adj = build_adjacency(nodes);
    let mut alive: HashSet<ContentHash> = HashSet::with_capacity(nodes.len());
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    for entry in entry_hashes {
        if adj.contains_key(entry) && alive.insert(entry.clone()) {
            queue.push_back(entry.clone());
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(targets) = adj.get(&current) {
            for target in targets {
                if alive.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }
        }
    }
    alive
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod graph;
pub mod reachability;
pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test reachability_bfs
```

Expected: `test result: ok. 4 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: compute_reachability — BFS from entries across static + dynamic imports"
```

---

## Task 6 — `reachability.rs`: SCC-aware liveness

**Files:**
- Modify: `crates/wundler-graph/src/reachability.rs`
- Test:   `crates/wundler-graph/tests/reachability_scc.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/reachability_scc.rs`:

```rust
use std::collections::HashSet;
use wundler_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::reachability::compute_reachability_with_sccs;

fn node(path: &str, imports: Vec<&str>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::of(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .into_iter()
                .map(|src| Import {
                    source: src.to_string(),
                    bindings: vec![],
                    kind: ImportKind::Static,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
    }
}

#[test]
fn cycle_member_pulls_in_entire_scc() {
    // a -> cir1 <-> cir2 ; entry = a. The cycle members must be alive together.
    let nodes = vec![
        node("a", vec!["cir1"]),
        node("cir1", vec!["cir2"]),
        node("cir2", vec!["cir1"]),
    ];
    let mut entries = HashSet::new();
    entries.insert(ContentHash::of("a"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    assert!(alive.contains(&ContentHash::of("a")));
    assert!(alive.contains(&ContentHash::of("cir1")));
    assert!(alive.contains(&ContentHash::of("cir2")));
}

#[test]
fn unreachable_cycle_stays_entirely_dead() {
    // a stands alone. cir1 <-> cir2 are unreachable. Neither becomes alive.
    let nodes = vec![
        node("a", vec![]),
        node("cir1", vec!["cir2"]),
        node("cir2", vec!["cir1"]),
    ];
    let mut entries = HashSet::new();
    entries.insert(ContentHash::of("a"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    assert!(alive.contains(&ContentHash::of("a")));
    assert!(!alive.contains(&ContentHash::of("cir1")));
    assert!(!alive.contains(&ContentHash::of("cir2")));
}

#[test]
fn cycle_node_in_entry_set_marks_whole_cycle_alive() {
    // Entry is cir1; cir2 must be alive even without explicit entry.
    let nodes = vec![
        node("cir1", vec!["cir2"]),
        node("cir2", vec!["cir1"]),
    ];
    let mut entries = HashSet::new();
    entries.insert(ContentHash::of("cir1"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);
    assert!(alive.contains(&ContentHash::of("cir1")));
    assert!(alive.contains(&ContentHash::of("cir2")));
}

#[test]
fn scc_aware_matches_plain_bfs_on_acyclic_graph() {
    // No cycles → SCC-aware and plain BFS produce identical liveness sets.
    let nodes = vec![
        node("a", vec!["b"]),
        node("b", vec!["c"]),
        node("c", vec![]),
        node("d", vec![]),
    ];
    let mut entries = HashSet::new();
    entries.insert(ContentHash::of("a"));
    let alive =
        wundler_graph::reachability::compute_reachability(&nodes, &entries);
    let alive_scc = compute_reachability_with_sccs(&nodes, &entries);
    assert_eq!(alive, alive_scc);
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test reachability_scc
```

Expected failure: `error[E0432]: unresolved import wundler_graph::reachability::compute_reachability_with_sccs`.

### Step 3: Write minimal implementation

- [ ] Append to `crates/wundler-graph/src/reachability.rs`:

```rust
use crate::graph::tarjan_sccs;
use std::collections::HashMap;

/// Reachability that respects strongly-connected components: if any module
/// in an SCC is reached, every member of that SCC is alive.
///
/// Strategy:
///   1. Compute SCCs over the full node set.
///   2. Assign each node to its SCC index.
///   3. Run plain BFS; whenever a node is marked alive, union in all members
///      of its SCC.
pub fn compute_reachability_with_sccs(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
) -> HashSet<ContentHash> {
    let adj = build_adjacency(nodes);
    let sccs = tarjan_sccs(nodes, &adj);

    let mut scc_of: HashMap<ContentHash, usize> = HashMap::with_capacity(nodes.len());
    for (i, scc) in sccs.iter().enumerate() {
        for member in scc {
            scc_of.insert(member.clone(), i);
        }
    }

    let mut alive: HashSet<ContentHash> = HashSet::with_capacity(nodes.len());
    let mut alive_scc: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    let mut activate_scc = |idx: usize,
                            alive: &mut HashSet<ContentHash>,
                            alive_scc: &mut HashSet<usize>,
                            queue: &mut VecDeque<ContentHash>| {
        if alive_scc.insert(idx) {
            for member in &sccs[idx] {
                if alive.insert(member.clone()) {
                    queue.push_back(member.clone());
                }
            }
        }
    };

    for entry in entry_hashes {
        if let Some(&idx) = scc_of.get(entry) {
            activate_scc(idx, &mut alive, &mut alive_scc, &mut queue);
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(targets) = adj.get(&current) {
            for target in targets {
                if let Some(&idx) = scc_of.get(target) {
                    activate_scc(idx, &mut alive, &mut alive_scc, &mut queue);
                }
            }
        }
    }

    alive
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test reachability_scc
cargo test -p wundler-graph --test reachability_bfs
```

Expected: each test binary reports `test result: ok` with all tests passing.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: SCC-aware reachability — circular imports are alive/dead together"
```

---

## Task 7 — `dce.rs`: call-edge transitive DCE

**Files:**
- Create: `crates/wundler-graph/src/dce.rs`
- Create: `crates/wundler-graph/tests/fixtures/dead_code.json`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/dce_call_edges.rs`

### Step 1: Write the failing test and fixture

- [ ] Create `crates/wundler-graph/tests/fixtures/dead_code.json`:

```json
[
  {"id":"hash_entry","path":"entry","summary":{"exports":[{"name":"main","kind":"Default","from":null}],"imports":[{"source":"util_a","bindings":["a"],"kind":"Static"},{"source":"util_b","bindings":["b"],"kind":"Static"}],"side_effects":"None","call_edges":[{"from_export":"main","to_export":"util_a::a"},{"from_export":"main","to_export":"util_b::b"}],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_util_a","path":"util_a","summary":{"exports":[{"name":"a","kind":"Named","from":null},{"name":"a_dead","kind":"Named","from":null}],"imports":[{"source":"util_a_dep","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[{"from_export":"a","to_export":"util_a_dep::live"}],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_util_b","path":"util_b","summary":{"exports":[{"name":"b","kind":"Named","from":null},{"name":"b_dead","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_util_a_dep","path":"util_a_dep","summary":{"exports":[{"name":"live","kind":"Named","from":null},{"name":"dead","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r1","path":"r1","summary":{"exports":[{"name":"r1","kind":"Named","from":null}],"imports":[{"source":"entry","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r2","path":"r2","summary":{"exports":[{"name":"r2","kind":"Named","from":null}],"imports":[{"source":"r1","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r3","path":"r3","summary":{"exports":[{"name":"r3","kind":"Named","from":null}],"imports":[{"source":"r2","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r4","path":"r4","summary":{"exports":[{"name":"r4","kind":"Named","from":null}],"imports":[{"source":"r3","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r5","path":"r5","summary":{"exports":[{"name":"r5","kind":"Named","from":null}],"imports":[{"source":"r4","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r6","path":"r6","summary":{"exports":[{"name":"r6","kind":"Named","from":null}],"imports":[{"source":"r5","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r7","path":"r7","summary":{"exports":[{"name":"r7","kind":"Named","from":null}],"imports":[{"source":"r6","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_r8","path":"r8","summary":{"exports":[{"name":"r8","kind":"Named","from":null}],"imports":[{"source":"r7","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d1","path":"d1","summary":{"exports":[{"name":"d1","kind":"Named","from":null}],"imports":[{"source":"d2","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d2","path":"d2","summary":{"exports":[{"name":"d2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d3","path":"d3","summary":{"exports":[{"name":"d3","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d4","path":"d4","summary":{"exports":[{"name":"d4","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d5","path":"d5","summary":{"exports":[{"name":"d5","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d6","path":"d6","summary":{"exports":[{"name":"d6","kind":"Named","from":null}],"imports":[{"source":"d7","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d7","path":"d7","summary":{"exports":[{"name":"d7","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_d8","path":"d8","summary":{"exports":[{"name":"d8","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null}
]
```

> **Layout:** 20 modules. 1 entry (`entry`) → reaches `util_a`, `util_b`, `util_a_dep`, `r1..r8` via static imports (12 reachable including entry). `d1..d8` are unreachable (8 dead). Within `util_a`, only the `a` export is called; `a_dead` has no inbound call edge. Within `util_a_dep`, only `live` is called.

- [ ] Create `crates/wundler-graph/tests/dce_call_edges.rs`:

```rust
use std::collections::HashSet;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::dce::compute_dead_exports;
use wundler_graph::reachability::compute_reachability_with_sccs;

fn load_fixture(name: &str) -> Vec<BundleGraphNode> {
    let text = std::fs::read_to_string(format!("tests/fixtures/{name}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn h(s: &str) -> ContentHash {
    ContentHash(s.to_string())
}

#[test]
fn dead_export_within_alive_module_is_reported() {
    let nodes = load_fixture("dead_code.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_entry"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    let dead = compute_dead_exports(&nodes, &alive);
    let util_a_dead = dead.get(&h("hash_util_a")).expect("util_a entry present");
    assert!(util_a_dead.contains("a_dead"));
    assert!(!util_a_dead.contains("a"));
}

#[test]
fn unreached_modules_have_all_exports_dead() {
    let nodes = load_fixture("dead_code.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_entry"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    let dead = compute_dead_exports(&nodes, &alive);
    // Dead modules are absent from the alive set, so they don't appear in
    // dead_exports (the caller only computes dead exports for *alive* modules).
    assert!(!dead.contains_key(&h("hash_d1")));
    assert!(!alive.contains(&h("hash_d1")));
}

#[test]
fn transitive_call_keeps_target_export_alive() {
    let nodes = load_fixture("dead_code.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_entry"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    let dead = compute_dead_exports(&nodes, &alive);
    let util_a_dep_dead =
        dead.get(&h("hash_util_a_dep")).expect("util_a_dep present");
    // `live` is called transitively: entry.main → util_a::a → util_a_dep::live
    assert!(!util_a_dep_dead.contains("live"));
    // `dead` is never called
    assert!(util_a_dep_dead.contains("dead"));
}

#[test]
fn default_exports_with_no_inbound_calls_are_still_alive() {
    // Default exports are roots — they are the public surface of a module that
    // imported it. We treat them as alive whenever the module is alive.
    let nodes = load_fixture("dead_code.json");
    let mut entries = HashSet::new();
    entries.insert(h("hash_entry"));
    let alive = compute_reachability_with_sccs(&nodes, &entries);

    let dead = compute_dead_exports(&nodes, &alive);
    let entry_dead = dead.get(&h("hash_entry")).expect("entry present");
    // entry's `main` is the default export — not dead.
    assert!(!entry_dead.contains("main"));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test dce_call_edges
```

Expected failure: `error[E0432]: unresolved import wundler_graph::dce`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/dce.rs`:

```rust
//! Call-edge transitive dead-export detection.
//!
//! Given the set of `alive` modules and each module's `call_edges`, walk
//! from every public root (default exports of alive modules + named exports
//! reachable from any inbound call) and mark unreached named exports as dead.
//!
//! Roots:
//!   * Every `Default` and `Namespace` export of every alive module — they
//!     are the public surface and we cannot know which named bindings the
//!     importer accesses without full cross-module tree-shaking.
//!   * Every named export that appears as `to_export` in some live
//!     `from_export -> to_export` edge.
//!
//! The `to_export` string convention used in `CallEdge` is
//! `"<module_path>::<export_name>"` for cross-module calls and
//! `"<export_name>"` for same-module calls. Plan 1 emits both forms.

use std::collections::{HashMap, HashSet, VecDeque};
use wundler_core::types::{BundleGraphNode, ContentHash, ExportKind};

/// Map: module ContentHash → set of dead named-export names within that module.
/// Only alive modules appear as keys; dead modules are handled at the
/// reachability layer.
pub fn compute_dead_exports(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
) -> HashMap<ContentHash, HashSet<String>> {
    // Index nodes by both ContentHash and path for cross-module call resolution.
    let mut by_hash: HashMap<&ContentHash, &BundleGraphNode> =
        HashMap::with_capacity(nodes.len());
    let mut by_path: HashMap<&str, &BundleGraphNode> = HashMap::with_capacity(nodes.len());
    for n in nodes {
        by_hash.insert(&n.id, n);
        by_path.insert(n.path.as_str(), n);
    }

    // Seed: live (module_hash, export_name) pairs.
    let mut live_exports: HashSet<(ContentHash, String)> = HashSet::new();
    let mut queue: VecDeque<(ContentHash, String)> = VecDeque::new();

    for n in nodes {
        if !alive.contains(&n.id) {
            continue;
        }
        for ex in &n.summary.exports {
            // Default and Namespace exports are public roots — always alive
            // when the module is alive.
            if matches!(ex.kind, ExportKind::Default | ExportKind::Namespace) {
                let key = (n.id.clone(), ex.name.clone());
                if live_exports.insert(key.clone()) {
                    queue.push_back(key);
                }
            }
        }
    }

    // Transitive walk via call edges.
    while let Some((mod_hash, export_name)) = queue.pop_front() {
        let Some(node) = by_hash.get(&mod_hash) else {
            continue;
        };
        for edge in &node.summary.call_edges {
            if edge.from_export != export_name {
                continue;
            }
            // Resolve `to_export`: either "mod_path::name" or "name" (same module).
            let (target_hash, target_name) = if let Some((path, name)) =
                edge.to_export.split_once("::")
            {
                let Some(target_node) = by_path.get(path) else {
                    continue;
                };
                (target_node.id.clone(), name.to_string())
            } else {
                (mod_hash.clone(), edge.to_export.clone())
            };
            if !alive.contains(&target_hash) {
                continue;
            }
            let key = (target_hash, target_name);
            if live_exports.insert(key.clone()) {
                queue.push_back(key);
            }
        }
    }

    // Compute dead exports per alive module.
    let mut dead: HashMap<ContentHash, HashSet<String>> = HashMap::new();
    for n in nodes {
        if !alive.contains(&n.id) {
            continue;
        }
        let mut module_dead: HashSet<String> = HashSet::new();
        for ex in &n.summary.exports {
            // Default and Namespace are never dead-while-alive (see seeding).
            if matches!(ex.kind, ExportKind::Default | ExportKind::Namespace) {
                continue;
            }
            let key = (n.id.clone(), ex.name.clone());
            if !live_exports.contains(&key) {
                module_dead.insert(ex.name.clone());
            }
        }
        dead.insert(n.id.clone(), module_dead);
    }
    dead
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod dce;
pub mod graph;
pub mod reachability;
pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test dce_call_edges
```

Expected: `test result: ok. 4 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: compute_dead_exports — call-edge transitive DCE on alive modules"
```

---

## Task 8 — `chunks.rs`: INITIAL chunks via static-import BFS per entry point

**Files:**
- Create: `crates/wundler-graph/src/chunks.rs`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/chunks_initial.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/chunks_initial.rs`:

```rust
use std::collections::{HashMap, HashSet};
use wundler_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::chunks::assign_chunks;
use wundler_graph::types::LoadCondition;

fn node(path: &str, imports: Vec<(&str, ImportKind)>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::of(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .into_iter()
                .map(|(s, k)| Import {
                    source: s.to_string(),
                    bindings: vec![],
                    kind: k,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
    }
}

#[test]
fn single_entry_yields_one_initial_chunk_containing_all_static_descendants() {
    let nodes = vec![
        node("entry", vec![("a", ImportKind::Static), ("b", ImportKind::Static)]),
        node("a", vec![("c", ImportKind::Static)]),
        node("b", vec![]),
        node("c", vec![]),
    ];
    let mut alive = HashSet::new();
    for n in &nodes {
        alive.insert(n.id.clone());
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, index) = assign_chunks(&nodes, &alive, &entries, 2);

    let initial: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Initial)
        .collect();
    assert_eq!(initial.len(), 1);
    let c = initial[0];
    assert_eq!(c.modules.len(), 4);
    for n in &nodes {
        assert_eq!(index.get(&n.id), Some(&c.id));
    }
}

#[test]
fn entry_with_no_static_imports_yields_singleton_initial_chunk() {
    let nodes = vec![node("entry", vec![])];
    let mut alive = HashSet::new();
    alive.insert(ContentHash::of("entry"));
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, index) = assign_chunks(&nodes, &alive, &entries, 2);
    let initial: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Initial)
        .collect();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].modules.len(), 1);
    assert_eq!(index.get(&ContentHash::of("entry")), Some(&initial[0].id));
}

#[test]
fn dynamic_imports_do_not_pull_into_initial_chunk() {
    // entry --static--> a ; entry --dynamic--> b
    // The INITIAL chunk for / contains {entry, a}; b is *not* in the initial chunk.
    let nodes = vec![
        node("entry", vec![("a", ImportKind::Static), ("b", ImportKind::Dynamic)]),
        node("a", vec![]),
        node("b", vec![]),
    ];
    let mut alive = HashSet::new();
    for n in &nodes {
        alive.insert(n.id.clone());
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, _) = assign_chunks(&nodes, &alive, &entries, 2);
    let initial: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Initial)
        .collect();
    assert_eq!(initial.len(), 1);
    assert!(initial[0].modules.contains(&ContentHash::of("entry")));
    assert!(initial[0].modules.contains(&ContentHash::of("a")));
    assert!(!initial[0].modules.contains(&ContentHash::of("b")));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_initial
```

Expected failure: `error[E0432]: unresolved import wundler_graph::chunks`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/chunks.rs`:

```rust
//! Chunk assignment.
//!
//! Route-based day-1 strategy:
//!   1. BFS from each entry point following STATIC imports only → one INITIAL
//!      chunk per entry point.
//!   2. Every Dynamic import boundary → a new LAZY chunk rooted at the
//!      imported module, expanded along its static-import subtree (Task 9).
//!   3. Modules that appear in >= `commons_threshold` distinct chunks are
//!      extracted to a single commons INITIAL chunk (Task 10).
//!   4. Each chunk's `hash` is the SHA-256 over its sorted member hashes
//!      (Task 11).

use crate::types::{Chunk, ChunkId, LoadCondition};
use std::collections::{HashMap, HashSet, VecDeque};
use wundler_core::types::{BundleGraphNode, ContentHash, ImportKind};

/// Assign every alive module to exactly one chunk.
///
/// Returns the list of chunks plus an index mapping each module to its
/// owning `ChunkId`. Dead modules (not in `alive`) are silently skipped.
pub fn assign_chunks(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
    entry_hashes: &HashMap<String, ContentHash>,
    _commons_threshold: usize,
) -> (Vec<Chunk>, HashMap<ContentHash, ChunkId>) {
    let by_hash: HashMap<&ContentHash, &BundleGraphNode> =
        nodes.iter().map(|n| (&n.id, n)).collect();
    let path_to_hash: HashMap<&str, ContentHash> =
        nodes.iter().map(|n| (n.path.as_str(), n.id.clone())).collect();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut module_index: HashMap<ContentHash, ChunkId> = HashMap::new();

    // One INITIAL chunk per entry point. Routes are iterated in sorted order
    // so chunk ids are deterministic.
    let mut routes: Vec<&String> = entry_hashes.keys().collect();
    routes.sort();

    for route in routes {
        let entry_hash = &entry_hashes[route];
        if !alive.contains(entry_hash) {
            continue;
        }
        let chunk_id = format!("initial_{}", chunk_id_slug(route));
        let modules = static_bfs_chunk(entry_hash, &by_hash, &path_to_hash, alive);
        for m in &modules {
            module_index.entry(m.clone()).or_insert_with(|| chunk_id.clone());
        }
        chunks.push(Chunk {
            id: chunk_id,
            modules,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    (chunks, module_index)
}

/// BFS from `root` following only STATIC imports. Returns the visited module
/// hashes in BFS order.
fn static_bfs_chunk(
    root: &ContentHash,
    by_hash: &HashMap<&ContentHash, &BundleGraphNode>,
    path_to_hash: &HashMap<&str, ContentHash>,
    alive: &HashSet<ContentHash>,
) -> Vec<ContentHash> {
    let mut visited: HashSet<ContentHash> = HashSet::new();
    let mut order: Vec<ContentHash> = Vec::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    if alive.contains(root) && visited.insert(root.clone()) {
        order.push(root.clone());
        queue.push_back(root.clone());
    }

    while let Some(current) = queue.pop_front() {
        let Some(node) = by_hash.get(&current) else { continue };
        for import in &node.summary.imports {
            if !matches!(import.kind, ImportKind::Static) {
                continue;
            }
            let Some(target) = path_to_hash.get(import.source.as_str()) else {
                continue;
            };
            if !alive.contains(target) {
                continue;
            }
            if visited.insert(target.clone()) {
                order.push(target.clone());
                queue.push_back(target.clone());
            }
        }
    }
    order
}

/// Turn an arbitrary entry-point string into a safe chunk-id slug.
fn chunk_id_slug(route: &str) -> String {
    let mut s = String::with_capacity(route.len());
    for ch in route.chars() {
        if ch.is_ascii_alphanumeric() {
            s.push(ch);
        } else {
            s.push('_');
        }
    }
    if s.is_empty() || s.chars().all(|c| c == '_') {
        s.push_str("root");
    }
    s
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod chunks;
pub mod dce;
pub mod graph;
pub mod reachability;
pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_initial
```

Expected: `test result: ok. 3 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: assign_chunks — INITIAL chunks via static-import BFS per entry point"
```

---

## Task 9 — `chunks.rs`: LAZY chunks at dynamic-import boundaries

**Files:**
- Modify: `crates/wundler-graph/src/chunks.rs`
- Test:   `crates/wundler-graph/tests/chunks_lazy.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/chunks_lazy.rs`:

```rust
use std::collections::{HashMap, HashSet};
use wundler_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary, SideEffectMarker,
};
use wundler_graph::chunks::assign_chunks;
use wundler_graph::types::LoadCondition;

fn node(path: &str, imports: Vec<(&str, ImportKind)>) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::of(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: vec![],
            imports: imports
                .into_iter()
                .map(|(s, k)| Import {
                    source: s.to_string(),
                    bindings: vec![],
                    kind: k,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        },
        alive: true,
        chunk_id: None,
    }
}

#[test]
fn dynamic_import_creates_lazy_chunk() {
    // entry --static--> a ; entry --dynamic--> lazy_root --static--> lazy_dep
    let nodes = vec![
        node("entry", vec![("a", ImportKind::Static), ("lazy_root", ImportKind::Dynamic)]),
        node("a", vec![]),
        node("lazy_root", vec![("lazy_dep", ImportKind::Static)]),
        node("lazy_dep", vec![]),
    ];
    let mut alive = HashSet::new();
    for n in &nodes {
        alive.insert(n.id.clone());
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, index) = assign_chunks(&nodes, &alive, &entries, 2);
    let lazy: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy.len(), 1);
    let l = lazy[0];
    assert!(l.modules.contains(&ContentHash::of("lazy_root")));
    assert!(l.modules.contains(&ContentHash::of("lazy_dep")));
    assert_eq!(index.get(&ContentHash::of("lazy_root")), Some(&l.id));
    assert_eq!(index.get(&ContentHash::of("lazy_dep")), Some(&l.id));

    // Initial chunk still contains entry and a.
    let initial: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Initial)
        .collect();
    assert_eq!(initial.len(), 1);
    assert!(initial[0].modules.contains(&ContentHash::of("entry")));
    assert!(initial[0].modules.contains(&ContentHash::of("a")));
    assert!(!initial[0].modules.contains(&ContentHash::of("lazy_root")));
}

#[test]
fn multiple_dynamic_imports_create_distinct_lazy_chunks() {
    let nodes = vec![
        node(
            "entry",
            vec![
                ("la", ImportKind::Dynamic),
                ("lb", ImportKind::Dynamic),
            ],
        ),
        node("la", vec![]),
        node("lb", vec![]),
    ];
    let mut alive = HashSet::new();
    for n in &nodes {
        alive.insert(n.id.clone());
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, _) = assign_chunks(&nodes, &alive, &entries, 2);
    let lazy: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy.len(), 2);
}

#[test]
fn dynamic_import_from_inside_a_lazy_chunk_spawns_nested_lazy() {
    // entry --dynamic--> la --dynamic--> lb
    // → two lazy chunks; la goes in one, lb goes in another.
    let nodes = vec![
        node("entry", vec![("la", ImportKind::Dynamic)]),
        node("la", vec![("lb", ImportKind::Dynamic)]),
        node("lb", vec![]),
    ];
    let mut alive = HashSet::new();
    for n in &nodes {
        alive.insert(n.id.clone());
    }
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("entry"));

    let (chunks, index) = assign_chunks(&nodes, &alive, &entries, 2);
    let lazy: Vec<_> = chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy.len(), 2);
    assert_ne!(
        index.get(&ContentHash::of("la")),
        index.get(&ContentHash::of("lb"))
    );
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_lazy
```

Expected failure: existing `assign_chunks` has no LAZY-chunk code path — tests will report 0 lazy chunks instead of the expected 1+, e.g. `assertion left == right failed: left: 0, right: 1`.

### Step 3: Write minimal implementation

- [ ] Replace `assign_chunks` and add a helper in `crates/wundler-graph/src/chunks.rs`:

```rust
pub fn assign_chunks(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
    entry_hashes: &HashMap<String, ContentHash>,
    _commons_threshold: usize,
) -> (Vec<Chunk>, HashMap<ContentHash, ChunkId>) {
    let by_hash: HashMap<&ContentHash, &BundleGraphNode> =
        nodes.iter().map(|n| (&n.id, n)).collect();
    let path_to_hash: HashMap<&str, ContentHash> =
        nodes.iter().map(|n| (n.path.as_str(), n.id.clone())).collect();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut module_index: HashMap<ContentHash, ChunkId> = HashMap::new();
    // Queue of dynamic-import roots to expand into LAZY chunks. Each entry is
    // (chunk-id-suffix-base, root-hash). De-duplicated by root-hash.
    let mut lazy_seen: HashSet<ContentHash> = HashSet::new();
    let mut lazy_queue: VecDeque<(String, ContentHash)> = VecDeque::new();
    let mut lazy_counter: usize = 0;

    // INITIAL chunks: one per entry point (sorted for deterministic ids).
    let mut routes: Vec<&String> = entry_hashes.keys().collect();
    routes.sort();
    for route in routes {
        let entry_hash = &entry_hashes[route];
        if !alive.contains(entry_hash) {
            continue;
        }
        let chunk_id = format!("initial_{}", chunk_id_slug(route));
        let modules = static_bfs_chunk_with_dynamic_capture(
            entry_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );
        for m in &modules {
            module_index.entry(m.clone()).or_insert_with(|| chunk_id.clone());
        }
        chunks.push(Chunk {
            id: chunk_id,
            modules,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    // LAZY chunks: one per dynamic-import boundary, BFS'd by static imports.
    while let Some((_route, root)) = lazy_queue.pop_front() {
        // Skip if some earlier chunk already owns this module (e.g. it was
        // statically imported elsewhere). Commons extraction (Task 10) handles
        // the case where the same module is both lazy and static.
        if module_index.contains_key(&root) {
            continue;
        }
        lazy_counter += 1;
        let chunk_id = format!("lazy_{lazy_counter}");
        let modules = static_bfs_chunk_with_dynamic_capture(
            &root,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );
        let modules: Vec<ContentHash> = modules
            .into_iter()
            .filter(|m| !module_index.contains_key(m))
            .collect();
        if modules.is_empty() {
            continue;
        }
        for m in &modules {
            module_index.insert(m.clone(), chunk_id.clone());
        }
        chunks.push(Chunk {
            id: chunk_id,
            modules,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Lazy,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    (chunks, module_index)
}

/// BFS from `root` following STATIC imports. Whenever a Dynamic import is
/// encountered, enqueue the target as a LAZY-chunk root.
fn static_bfs_chunk_with_dynamic_capture(
    root: &ContentHash,
    by_hash: &HashMap<&ContentHash, &BundleGraphNode>,
    path_to_hash: &HashMap<&str, ContentHash>,
    alive: &HashSet<ContentHash>,
    lazy_seen: &mut HashSet<ContentHash>,
    lazy_queue: &mut VecDeque<(String, ContentHash)>,
) -> Vec<ContentHash> {
    let mut visited: HashSet<ContentHash> = HashSet::new();
    let mut order: Vec<ContentHash> = Vec::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    if alive.contains(root) && visited.insert(root.clone()) {
        order.push(root.clone());
        queue.push_back(root.clone());
    }

    while let Some(current) = queue.pop_front() {
        let Some(node) = by_hash.get(&current) else { continue };
        for import in &node.summary.imports {
            let Some(target) = path_to_hash.get(import.source.as_str()) else {
                continue;
            };
            if !alive.contains(target) {
                continue;
            }
            match import.kind {
                ImportKind::Static => {
                    if visited.insert(target.clone()) {
                        order.push(target.clone());
                        queue.push_back(target.clone());
                    }
                }
                ImportKind::Dynamic => {
                    if lazy_seen.insert(target.clone()) {
                        lazy_queue
                            .push_back((node.path.clone(), target.clone()));
                    }
                }
            }
        }
    }
    order
}
```

> **Note:** Delete the original `static_bfs_chunk` helper from Task 8 — `static_bfs_chunk_with_dynamic_capture` supersedes it. The Task 8 tests still pass because the new function falls back to pure static behavior when no dynamic imports exist.

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_lazy
cargo test -p wundler-graph --test chunks_initial
```

Expected: both test binaries report `test result: ok` with all tests passing.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: LAZY chunks at dynamic-import boundaries"
```

---

## Task 10 — `chunks.rs`: commons extraction

**Files:**
- Modify: `crates/wundler-graph/src/chunks.rs`
- Create: `crates/wundler-graph/tests/fixtures/commons.json`
- Test:   `crates/wundler-graph/tests/chunks_commons.rs`

### Step 1: Write the failing test and fixture

- [ ] Create `crates/wundler-graph/tests/fixtures/commons.json`:

```json
[
  {"id":"hash_entry_a","path":"entry_a","summary":{"exports":[{"name":"a","kind":"Default","from":null}],"imports":[{"source":"priv_a1","bindings":[],"kind":"Static"},{"source":"priv_a2","bindings":[],"kind":"Static"},{"source":"shared_util","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_entry_b","path":"entry_b","summary":{"exports":[{"name":"b","kind":"Default","from":null}],"imports":[{"source":"priv_b1","bindings":[],"kind":"Static"},{"source":"priv_b2","bindings":[],"kind":"Static"},{"source":"shared_util","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_entry_c","path":"entry_c","summary":{"exports":[{"name":"c","kind":"Default","from":null}],"imports":[{"source":"priv_c1","bindings":[],"kind":"Static"},{"source":"priv_c2","bindings":[],"kind":"Static"},{"source":"shared_util","bindings":[],"kind":"Static"}],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_a1","path":"priv_a1","summary":{"exports":[{"name":"a1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_a2","path":"priv_a2","summary":{"exports":[{"name":"a2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_b1","path":"priv_b1","summary":{"exports":[{"name":"b1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_b2","path":"priv_b2","summary":{"exports":[{"name":"b2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_c1","path":"priv_c1","summary":{"exports":[{"name":"c1","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_priv_c2","path":"priv_c2","summary":{"exports":[{"name":"c2","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null},
  {"id":"hash_shared_util","path":"shared_util","summary":{"exports":[{"name":"util","kind":"Named","from":null}],"imports":[],"side_effects":"None","call_edges":[],"ambient_refs":[]},"alive":true,"chunk_id":null}
]
```

- [ ] Create `crates/wundler-graph/tests/chunks_commons.rs`:

```rust
use std::collections::{HashMap, HashSet};
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::chunks::assign_chunks;
use wundler_graph::types::LoadCondition;

fn load_fixture(name: &str) -> Vec<BundleGraphNode> {
    let text = std::fs::read_to_string(format!("tests/fixtures/{name}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn h(s: &str) -> ContentHash {
    ContentHash(s.to_string())
}

#[test]
fn shared_util_extracted_to_commons_when_in_two_chunks() {
    let nodes = load_fixture("commons.json");
    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let mut entries = HashMap::new();
    entries.insert("/a".to_string(), h("hash_entry_a"));
    entries.insert("/b".to_string(), h("hash_entry_b"));
    entries.insert("/c".to_string(), h("hash_entry_c"));

    let (chunks, index) = assign_chunks(&nodes, &alive, &entries, 2);

    let commons: Vec<_> = chunks.iter().filter(|c| c.id == "commons").collect();
    assert_eq!(commons.len(), 1, "expected exactly one commons chunk");
    let commons = commons[0];
    assert_eq!(commons.load_condition, LoadCondition::Initial);
    assert_eq!(commons.modules, vec![h("hash_shared_util")]);
    assert_eq!(index.get(&h("hash_shared_util")), Some(&"commons".to_string()));

    // Each per-route chunk no longer contains shared_util.
    for c in chunks.iter().filter(|c| c.id != "commons") {
        assert!(
            !c.modules.contains(&h("hash_shared_util")),
            "chunk {} still contains shared_util",
            c.id
        );
    }
}

#[test]
fn private_modules_not_extracted_to_commons() {
    let nodes = load_fixture("commons.json");
    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let mut entries = HashMap::new();
    entries.insert("/a".to_string(), h("hash_entry_a"));
    entries.insert("/b".to_string(), h("hash_entry_b"));
    entries.insert("/c".to_string(), h("hash_entry_c"));

    let (chunks, _) = assign_chunks(&nodes, &alive, &entries, 2);

    let commons = chunks.iter().find(|c| c.id == "commons").unwrap();
    assert!(!commons.modules.contains(&h("hash_priv_a1")));
    assert!(!commons.modules.contains(&h("hash_priv_b2")));
}

#[test]
fn high_threshold_disables_commons_extraction() {
    let nodes = load_fixture("commons.json");
    let alive: HashSet<ContentHash> = nodes.iter().map(|n| n.id.clone()).collect();

    let mut entries = HashMap::new();
    entries.insert("/a".to_string(), h("hash_entry_a"));
    entries.insert("/b".to_string(), h("hash_entry_b"));
    entries.insert("/c".to_string(), h("hash_entry_c"));

    // shared_util appears in 3 chunks; threshold 4 → no extraction.
    let (chunks, _) = assign_chunks(&nodes, &alive, &entries, 4);
    assert!(chunks.iter().all(|c| c.id != "commons"));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_commons
```

Expected failure: no `commons` chunk is produced — the first assertion fails with `expected exactly one commons chunk` (the existing implementation duplicates `shared_util` across the three INITIAL chunks via `entry(...).or_insert_with(...)`, so the first chunk wins and the rest silently drop it — but the test asserts a dedicated `commons` chunk).

### Step 3: Write minimal implementation

- [ ] Refactor `assign_chunks` in `crates/wundler-graph/src/chunks.rs` to (a) record every chunk a module appears in across all routes, (b) build each route's INITIAL chunk *without* the `entry().or_insert_with` deduplication trick, (c) extract commons in a final pass. Replace the function body:

```rust
pub fn assign_chunks(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
    entry_hashes: &HashMap<String, ContentHash>,
    commons_threshold: usize,
) -> (Vec<Chunk>, HashMap<ContentHash, ChunkId>) {
    let by_hash: HashMap<&ContentHash, &BundleGraphNode> =
        nodes.iter().map(|n| (&n.id, n)).collect();
    let path_to_hash: HashMap<&str, ContentHash> =
        nodes.iter().map(|n| (n.path.as_str(), n.id.clone())).collect();

    // Phase 1: collect per-chunk membership without de-duping yet.
    // Each module may appear in many candidates here.
    struct Candidate {
        id: ChunkId,
        load_condition: LoadCondition,
        modules: Vec<ContentHash>,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut lazy_seen: HashSet<ContentHash> = HashSet::new();
    let mut lazy_queue: VecDeque<(String, ContentHash)> = VecDeque::new();
    let mut lazy_counter: usize = 0;

    let mut routes: Vec<&String> = entry_hashes.keys().collect();
    routes.sort();
    for route in routes {
        let entry_hash = &entry_hashes[route];
        if !alive.contains(entry_hash) {
            continue;
        }
        let id = format!("initial_{}", chunk_id_slug(route));
        let modules = static_bfs_chunk_with_dynamic_capture(
            entry_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );
        candidates.push(Candidate {
            id,
            load_condition: LoadCondition::Initial,
            modules,
        });
    }
    while let Some((_route, root)) = lazy_queue.pop_front() {
        lazy_counter += 1;
        let id = format!("lazy_{lazy_counter}");
        let modules = static_bfs_chunk_with_dynamic_capture(
            &root,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );
        if modules.is_empty() {
            continue;
        }
        candidates.push(Candidate {
            id,
            load_condition: LoadCondition::Lazy,
            modules,
        });
    }

    // Phase 2: count how many candidate chunks each module appears in.
    let mut appearance: HashMap<ContentHash, usize> = HashMap::new();
    for cand in &candidates {
        let mut seen_in_this: HashSet<ContentHash> = HashSet::new();
        for m in &cand.modules {
            if seen_in_this.insert(m.clone()) {
                *appearance.entry(m.clone()).or_insert(0) += 1;
            }
        }
    }

    // Phase 3: a module is "commons" iff it appears in >= threshold candidates.
    let commons_set: HashSet<ContentHash> = appearance
        .iter()
        .filter(|(_, &count)| count >= commons_threshold)
        .map(|(h, _)| h.clone())
        .collect();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut module_index: HashMap<ContentHash, ChunkId> = HashMap::new();

    // Emit the commons chunk first (if non-empty) so its id is stable.
    if !commons_set.is_empty() {
        let mut commons_members: Vec<ContentHash> = commons_set.iter().cloned().collect();
        commons_members.sort_by(|a, b| a.0.cmp(&b.0));
        for m in &commons_members {
            module_index.insert(m.clone(), "commons".to_string());
        }
        chunks.push(Chunk {
            id: "commons".to_string(),
            modules: commons_members,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    // Emit per-route chunks with commons filtered out and any leftover
    // duplicates resolved by first-seen wins.
    for cand in candidates {
        let mut modules: Vec<ContentHash> = Vec::with_capacity(cand.modules.len());
        for m in cand.modules {
            if commons_set.contains(&m) {
                continue;
            }
            if module_index.contains_key(&m) {
                continue;
            }
            module_index.insert(m.clone(), cand.id.clone());
            modules.push(m);
        }
        if modules.is_empty() {
            continue;
        }
        chunks.push(Chunk {
            id: cand.id,
            modules,
            hash: ContentHash("".to_string()),
            load_condition: cand.load_condition,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    (chunks, module_index)
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_commons
cargo test -p wundler-graph --test chunks_initial
cargo test -p wundler-graph --test chunks_lazy
```

Expected: each test binary reports `test result: ok` with all tests passing.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: commons extraction — modules in >= threshold chunks move to commons chunk"
```

---

## Task 11 — `chunks.rs`: chunk hashing

**Files:**
- Modify: `crates/wundler-graph/src/chunks.rs`
- Test:   `crates/wundler-graph/tests/chunks_hashing.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/chunks_hashing.rs`:

```rust
use wundler_core::types::ContentHash;
use wundler_graph::chunks::hash_chunk;

#[test]
fn hash_is_deterministic() {
    let members = vec![ContentHash("aaa".into()), ContentHash("bbb".into())];
    let h1 = hash_chunk(&members);
    let h2 = hash_chunk(&members);
    assert_eq!(h1, h2);
}

#[test]
fn hash_is_order_independent() {
    let a = vec![ContentHash("aaa".into()), ContentHash("bbb".into())];
    let b = vec![ContentHash("bbb".into()), ContentHash("aaa".into())];
    assert_eq!(hash_chunk(&a), hash_chunk(&b));
}

#[test]
fn hash_changes_when_member_changes() {
    let a = vec![ContentHash("aaa".into()), ContentHash("bbb".into())];
    let b = vec![ContentHash("aaa".into()), ContentHash("ccc".into())];
    assert_ne!(hash_chunk(&a), hash_chunk(&b));
}

#[test]
fn hash_for_empty_chunk_is_stable() {
    let h = hash_chunk(&[]);
    // SHA-256 of the empty input is well-known.
    assert_eq!(
        h.0,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hash_is_64_char_hex() {
    let h = hash_chunk(&[ContentHash("xyz".into())]);
    assert_eq!(h.0.len(), 64);
    assert!(h.0.chars().all(|c| c.is_ascii_hexdigit()));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_hashing
```

Expected failure: `error[E0425]: cannot find function hash_chunk in module wundler_graph::chunks`.

### Step 3: Write minimal implementation

- [ ] Append to `crates/wundler-graph/src/chunks.rs`:

```rust
use sha2::{Digest, Sha256};

/// SHA-256 over the lexicographically sorted member hashes, joined by '\n'.
/// Output is hex-encoded (64 ASCII characters).
pub fn hash_chunk(members: &[ContentHash]) -> ContentHash {
    let mut sorted: Vec<&str> = members.iter().map(|h| h.0.as_str()).collect();
    sorted.sort();
    let joined = sorted.join("\n");
    let digest = Sha256::digest(joined.as_bytes());
    ContentHash(hex::encode(digest))
}
```

- [ ] Wire `hash_chunk` into `assign_chunks`. Replace every `hash: ContentHash("".to_string()),` site in the `chunks.push(Chunk { ... })` blocks with the actual hash computed from the chunk's members. Concretely, build the `modules` vector first, then call `hash_chunk(&modules)` before pushing. Apply this to all three chunk-construction sites (commons, INITIAL per-route, LAZY).

For example, the commons-chunk site becomes:

```rust
let commons_hash = hash_chunk(&commons_members);
chunks.push(Chunk {
    id: "commons".to_string(),
    modules: commons_members,
    hash: commons_hash,
    load_condition: LoadCondition::Initial,
    co_request_score: None,
    median_load_order: None,
    suggested_merge: None,
});
```

And the per-route site becomes:

```rust
let chunk_hash = hash_chunk(&modules);
chunks.push(Chunk {
    id: cand.id,
    modules,
    hash: chunk_hash,
    load_condition: cand.load_condition,
    co_request_score: None,
    median_load_order: None,
    suggested_merge: None,
});
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test chunks_hashing
cargo test -p wundler-graph --test chunks_initial
cargo test -p wundler-graph --test chunks_lazy
cargo test -p wundler-graph --test chunks_commons
```

Expected: each test binary reports `test result: ok` with all tests passing.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: hash_chunk — SHA-256 over sorted member hashes; wire into assign_chunks"
```

---

## Task 12 — `manifest.rs`: `ChunkManifest` assembly

**Files:**
- Create: `crates/wundler-graph/src/manifest.rs`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/manifest_build.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/manifest_build.rs`:

```rust
use std::collections::HashMap;
use wundler_core::types::ContentHash;
use wundler_graph::manifest::build_manifest;
use wundler_graph::types::{Chunk, LoadCondition};

fn dummy_chunk(id: &str, members: Vec<ContentHash>) -> Chunk {
    Chunk {
        id: id.to_string(),
        modules: members,
        hash: ContentHash::of(id),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    }
}

#[test]
fn build_id_is_deterministic_for_same_entry_points() {
    let chunks = vec![dummy_chunk("c0", vec![ContentHash::of("m1")])];
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("m1"));
    let mut index = HashMap::new();
    index.insert(ContentHash::of("m1"), "c0".to_string());

    let m1 = build_manifest(chunks.clone(), &entries, index.clone());
    let m2 = build_manifest(chunks, &entries, index);
    assert_eq!(m1.build_id, m2.build_id);
    assert_eq!(m1.build_id.len(), 64);
}

#[test]
fn build_id_changes_when_entry_set_changes() {
    let chunks = vec![dummy_chunk("c0", vec![ContentHash::of("m1")])];
    let index: HashMap<ContentHash, String> = HashMap::new();

    let mut e1 = HashMap::new();
    e1.insert("/".to_string(), ContentHash::of("m1"));
    let mut e2 = HashMap::new();
    e2.insert("/admin".to_string(), ContentHash::of("m1"));

    let m1 = build_manifest(chunks.clone(), &e1, index.clone());
    let m2 = build_manifest(chunks, &e2, index);
    assert_ne!(m1.build_id, m2.build_id);
}

#[test]
fn entry_chunks_lists_route_to_owning_chunk() {
    let chunks = vec![
        dummy_chunk("c0", vec![ContentHash::of("entry_a"), ContentHash::of("m1")]),
        dummy_chunk("commons", vec![ContentHash::of("shared")]),
    ];
    let mut entries = HashMap::new();
    entries.insert("/a".to_string(), ContentHash::of("entry_a"));

    let mut index = HashMap::new();
    index.insert(ContentHash::of("entry_a"), "c0".to_string());
    index.insert(ContentHash::of("m1"), "c0".to_string());
    index.insert(ContentHash::of("shared"), "commons".to_string());

    let manifest = build_manifest(chunks, &entries, index);
    let chunks_for_a = manifest.entry_chunks.get("/a").expect("/a present");
    // Commons (if present) must be listed before the route-specific chunk.
    assert!(chunks_for_a.contains(&"c0".to_string()));
    assert!(chunks_for_a.contains(&"commons".to_string()));
    let commons_pos = chunks_for_a.iter().position(|c| c == "commons").unwrap();
    let c0_pos = chunks_for_a.iter().position(|c| c == "c0").unwrap();
    assert!(commons_pos < c0_pos);
}

#[test]
fn module_index_is_preserved() {
    let chunks = vec![dummy_chunk("c0", vec![ContentHash::of("m1")])];
    let mut entries = HashMap::new();
    entries.insert("/".to_string(), ContentHash::of("m1"));
    let mut index = HashMap::new();
    index.insert(ContentHash::of("m1"), "c0".to_string());

    let manifest = build_manifest(chunks, &entries, index.clone());
    assert_eq!(manifest.module_index, index);
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test manifest_build
```

Expected failure: `error[E0432]: unresolved import wundler_graph::manifest`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/manifest.rs`:

```rust
//! Assemble a `ChunkManifest` from chunks, entry points, and a module index.
//!
//! `build_id` is SHA-256 over the sorted entry-point paths joined with '\n'.
//! `entry_chunks[route]` is the ordered list of chunks the runtime must load
//! to satisfy the route. The owning chunk of the route's entry module is
//! emitted last; `commons` (when present) is emitted first.

use crate::types::{Chunk, ChunkId, ChunkManifest, EntryPoint};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use wundler_core::types::ContentHash;

pub fn build_manifest(
    chunks: Vec<Chunk>,
    entry_hashes: &HashMap<String, ContentHash>,
    module_index: HashMap<ContentHash, ChunkId>,
) -> ChunkManifest {
    let build_id = compute_build_id(entry_hashes);

    let known_chunk_ids: std::collections::HashSet<&str> =
        chunks.iter().map(|c| c.id.as_str()).collect();

    let mut entry_chunks: HashMap<EntryPoint, Vec<ChunkId>> = HashMap::new();
    for (route, entry_hash) in entry_hashes {
        let mut ordered: Vec<ChunkId> = Vec::new();
        if known_chunk_ids.contains("commons") {
            ordered.push("commons".to_string());
        }
        if let Some(owning) = module_index.get(entry_hash) {
            if owning != "commons" && !ordered.contains(owning) {
                ordered.push(owning.clone());
            }
        }
        entry_chunks.insert(route.clone(), ordered);
    }

    ChunkManifest {
        build_id,
        chunks,
        entry_chunks,
        module_index,
    }
}

fn compute_build_id(entry_hashes: &HashMap<String, ContentHash>) -> String {
    let mut sorted: Vec<&str> = entry_hashes.keys().map(|s| s.as_str()).collect();
    sorted.sort();
    let joined = sorted.join("\n");
    let digest = Sha256::digest(joined.as_bytes());
    hex::encode(digest)
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod chunks;
pub mod dce;
pub mod graph;
pub mod manifest;
pub mod reachability;
pub mod types;

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test manifest_build
```

Expected: `test result: ok. 4 passed; 0 failed`.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: build_manifest — build_id, entry_chunks, module_index assembly"
```

---

## Task 13 — `manifest.rs`: `ChunkManifest` JSON round-trip

**Files:**
- Test:   `crates/wundler-graph/tests/manifest_json.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/manifest_json.rs`:

```rust
use std::collections::HashMap;
use wundler_core::types::ContentHash;
use wundler_graph::manifest::build_manifest;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

fn fixture() -> ChunkManifest {
    let chunks = vec![
        Chunk {
            id: "commons".into(),
            modules: vec![ContentHash::of("shared")],
            hash: ContentHash::of("commons"),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        },
        Chunk {
            id: "initial__a".into(),
            modules: vec![ContentHash::of("entry_a"), ContentHash::of("m1")],
            hash: ContentHash::of("initial__a"),
            load_condition: LoadCondition::Initial,
            co_request_score: Some(0.42),
            median_load_order: Some(0.5),
            suggested_merge: Some("initial__b".into()),
        },
        Chunk {
            id: "lazy_1".into(),
            modules: vec![ContentHash::of("lz")],
            hash: ContentHash::of("lazy_1"),
            load_condition: LoadCondition::Lazy,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        },
    ];
    let mut entries = HashMap::new();
    entries.insert("/a".to_string(), ContentHash::of("entry_a"));
    let mut index = HashMap::new();
    index.insert(ContentHash::of("shared"), "commons".to_string());
    index.insert(ContentHash::of("entry_a"), "initial__a".to_string());
    index.insert(ContentHash::of("m1"), "initial__a".to_string());
    index.insert(ContentHash::of("lz"), "lazy_1".to_string());
    build_manifest(chunks, &entries, index)
}

#[test]
fn round_trip_preserves_all_fields() {
    let m = fixture();
    let json = m.to_json().unwrap();
    let back = ChunkManifest::from_json(&json).unwrap();

    assert_eq!(back.build_id, m.build_id);
    assert_eq!(back.chunks.len(), m.chunks.len());
    for (a, b) in back.chunks.iter().zip(m.chunks.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.modules, b.modules);
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.load_condition, b.load_condition);
        assert_eq!(a.co_request_score, b.co_request_score);
        assert_eq!(a.median_load_order, b.median_load_order);
        assert_eq!(a.suggested_merge, b.suggested_merge);
    }
    assert_eq!(back.entry_chunks, m.entry_chunks);
    assert_eq!(back.module_index, m.module_index);
}

#[test]
fn output_is_valid_pretty_printed_json() {
    let m = fixture();
    let json = m.to_json().unwrap();
    // Pretty-printed JSON contains at least one newline.
    assert!(json.contains('\n'));
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_object());
    assert!(parsed.get("build_id").is_some());
    assert!(parsed.get("chunks").is_some());
    assert!(parsed.get("entry_chunks").is_some());
    assert!(parsed.get("module_index").is_some());
}

#[test]
fn malformed_json_returns_error() {
    let bad = r#"{"build_id":"x","chunks":[{"id":"c","modules":["a"]"#;
    assert!(ChunkManifest::from_json(bad).is_err());
}
```

### Step 2: Run test, verify it FAILS

> The `to_json` / `from_json` methods already exist (Task 2), but this is the first time their behavior is exercised end-to-end through the assembled manifest. If Task 2's implementation is correct, this test should pass on first run. If it doesn't, the bug is in serialization.

- [ ] Run:

```bash
cargo test -p wundler-graph --test manifest_json
```

Expected: this test exercises only existing code — it should pass immediately. If it does **not**, that constitutes the "failing test" for the implementation step below: a real bug has been surfaced and the manifest/types module must be fixed before continuing.

### Step 3: Write minimal implementation

- [ ] If all `manifest_json` tests already pass, no code change is needed for this task — the previous implementations are correct. Confirm by re-running the whole crate's test suite:

```bash
cargo test -p wundler-graph
```

If any test fails, investigate and patch `crates/wundler-graph/src/types.rs` or `crates/wundler-graph/src/manifest.rs` until all pass. Most-likely failure modes and their fixes:
- `co_request_score` deserializes as `null` even when `Some` → make sure `#[serde(skip_serializing_if = "Option::is_none")]` is **not** applied (we want the field always present for downstream tooling).
- `LoadCondition` round-trip mismatch → confirm `#[derive(Serialize, Deserialize)]` is intact on the enum.

### Step 4: Run test, verify PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test manifest_json
cargo test -p wundler-graph
```

Expected: `manifest_json` reports `test result: ok. 3 passed; 0 failed`. Full crate run also clean.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: ChunkManifest JSON round-trip — full integration test for serde"
```

---

## Task 14 — `analyzer.rs`: `GraphAnalyzer::analyze()` end-to-end integration

**Files:**
- Create: `crates/wundler-graph/src/analyzer.rs`
- Modify: `crates/wundler-graph/src/lib.rs`
- Test:   `crates/wundler-graph/tests/analyze_integration.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-graph/tests/analyze_integration.rs`:

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use wundler_graph::types::LoadCondition;

fn load_fixture(name: &str) -> Vec<BundleGraphNode> {
    let text = std::fs::read_to_string(format!("tests/fixtures/{name}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn h(s: &str) -> ContentHash {
    ContentHash(s.to_string())
}

#[test]
fn analyze_multi_entry_produces_expected_chunk_layout() {
    let nodes = load_fixture("multi_entry.json");
    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("/a".into(), PathBuf::from("a"));
    entry_points.insert("/b".into(), PathBuf::from("b"));
    entry_points.insert("/c".into(), PathBuf::from("c"));

    let analyzer = GraphAnalyzer::new(entry_points);
    let AnalysisResult { nodes: out_nodes, manifest, stats } =
        analyzer.analyze(nodes).expect("analyze succeeds");

    // 15 modules total, all reachable from the union of {a, b, c} →
    // 13 reachable singletons + 1 cycle of 2 unreachable = 13 alive.
    // (cir1/cir2 are not imported by any entry.)
    assert_eq!(stats.total, 15);
    assert_eq!(stats.alive, 13);
    assert_eq!(stats.dead, 2);

    // shared appears in initial-a and initial-b → commons (threshold 2 default).
    let commons = manifest
        .chunks
        .iter()
        .find(|c| c.id == "commons")
        .expect("commons chunk exists");
    assert!(commons.modules.contains(&h("hash_shared")));
    assert_eq!(commons.load_condition, LoadCondition::Initial);

    // 3 entries → 3 INITIAL chunks (commons not counted) + 3 dynamic boundaries → 3 LAZY chunks.
    let initial_route_chunks: Vec<_> = manifest
        .chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Initial && c.id != "commons")
        .collect();
    assert_eq!(initial_route_chunks.len(), 3);
    let lazy_chunks: Vec<_> = manifest
        .chunks
        .iter()
        .filter(|c| c.load_condition == LoadCondition::Lazy)
        .collect();
    assert_eq!(lazy_chunks.len(), 3);

    // entry_chunks must list commons before each route chunk.
    for route in ["/a", "/b", "/c"] {
        let entries = manifest.entry_chunks.get(route).expect(route);
        assert!(entries.first().map(|s| s.as_str()) == Some("commons"));
    }

    // module_index is consistent: every alive module has exactly one owning chunk.
    for n in &out_nodes {
        if n.alive {
            assert!(
                manifest.module_index.contains_key(&n.id),
                "alive node {} missing from module_index",
                n.path
            );
        }
    }

    // build_id is 64-char hex.
    assert_eq!(manifest.build_id.len(), 64);
    assert!(manifest.build_id.chars().all(|c| c.is_ascii_hexdigit()));

    // Stats sanity.
    assert_eq!(stats.chunks, manifest.chunks.len());
}

#[test]
fn analyze_marks_nodes_alive_flag_correctly() {
    let nodes = load_fixture("multi_entry.json");
    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("/a".into(), PathBuf::from("a"));

    let analyzer = GraphAnalyzer::new(entry_points);
    let result = analyzer.analyze(nodes).unwrap();

    let alive_paths: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.alive)
        .map(|n| n.path.as_str())
        .collect();
    // From /a alone: a, x1, x2, shared, lazy_a1.
    assert!(alive_paths.contains(&"a"));
    assert!(alive_paths.contains(&"x1"));
    assert!(alive_paths.contains(&"x2"));
    assert!(alive_paths.contains(&"shared"));
    assert!(alive_paths.contains(&"lazy_a1"));
    assert!(!alive_paths.contains(&"b"));
    assert!(!alive_paths.contains(&"c"));
}

#[test]
fn analyze_fails_when_entry_point_path_unknown() {
    let nodes = load_fixture("multi_entry.json");
    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    entry_points.insert("/missing".into(), PathBuf::from("does_not_exist"));

    let analyzer = GraphAnalyzer::new(entry_points);
    let result = analyzer.analyze(nodes);
    assert!(result.is_err());
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-graph --test analyze_integration
```

Expected failure: `error[E0432]: unresolved import wundler_graph::analyzer`.

### Step 3: Write minimal implementation

- [ ] Create `crates/wundler-graph/src/analyzer.rs`:

```rust
//! Top-level orchestrator. `GraphAnalyzer::analyze()` wires together
//! reachability → DCE → chunk assignment → manifest assembly.

use crate::chunks::assign_chunks;
use crate::dce::compute_dead_exports;
use crate::manifest::build_manifest;
use crate::reachability::compute_reachability_with_sccs;
use crate::types::ChunkManifest;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use wundler_core::types::{BundleGraphNode, ContentHash};

#[derive(Debug, Clone)]
pub struct GraphAnalyzer {
    pub entry_points: HashMap<String, PathBuf>,
    /// Module must appear in >= this many candidate chunks to be extracted to
    /// `commons`. Default 2.
    pub commons_threshold: usize,
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub nodes: Vec<BundleGraphNode>,
    pub manifest: ChunkManifest,
    pub stats: AnalysisStats,
}

#[derive(Debug, Clone)]
pub struct AnalysisStats {
    pub total: usize,
    pub alive: usize,
    pub dead: usize,
    pub chunks: usize,
}

impl GraphAnalyzer {
    pub fn new(entry_points: HashMap<String, PathBuf>) -> Self {
        Self {
            entry_points,
            commons_threshold: 2,
        }
    }

    pub fn analyze(&self, mut nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        // 1. Resolve entry-point paths to ContentHashes.
        let path_to_hash: HashMap<&str, ContentHash> =
            nodes.iter().map(|n| (n.path.as_str(), n.id.clone())).collect();

        let mut entry_hashes: HashMap<String, ContentHash> = HashMap::new();
        let mut entry_hash_set: HashSet<ContentHash> = HashSet::new();
        for (route, path) in &self.entry_points {
            let path_str = path.to_string_lossy();
            let hash = path_to_hash.get(path_str.as_ref()).cloned().ok_or_else(|| {
                anyhow!(
                    "entry point {route} -> {} not found among summarized modules",
                    path.display()
                )
            })?;
            entry_hashes.insert(route.clone(), hash.clone());
            entry_hash_set.insert(hash);
        }

        // 2. Reachability (SCC-aware) and DCE.
        let alive = compute_reachability_with_sccs(&nodes, &entry_hash_set);
        let dead_exports = compute_dead_exports(&nodes, &alive);

        // 3. Stamp alive flags + dead-export annotations back onto nodes.
        for n in nodes.iter_mut() {
            n.alive = alive.contains(&n.id);
            if !n.alive {
                continue;
            }
            if let Some(dead) = dead_exports.get(&n.id) {
                // Remove dead-named exports from the node's summary so the
                // transform layer (Plan 3) doesn't have to recompute.
                n.summary.exports.retain(|e| !dead.contains(&e.name));
            }
        }

        // 4. Chunk assignment.
        let (mut chunks, module_index) =
            assign_chunks(&nodes, &alive, &entry_hashes, self.commons_threshold);

        // 5. Stamp owning chunk id onto each node.
        for n in nodes.iter_mut() {
            n.chunk_id = module_index.get(&n.id).cloned();
        }

        // Sort chunks deterministically by id for stable manifests.
        chunks.sort_by(|a, b| a.id.cmp(&b.id));

        // 6. Manifest assembly.
        let manifest = build_manifest(chunks, &entry_hashes, module_index);

        let total = nodes.len();
        let alive_count = nodes.iter().filter(|n| n.alive).count();
        let stats = AnalysisStats {
            total,
            alive: alive_count,
            dead: total - alive_count,
            chunks: manifest.chunks.len(),
        };

        Ok(AnalysisResult {
            nodes,
            manifest,
            stats,
        })
    }
}
```

- [ ] Modify `crates/wundler-graph/src/lib.rs`:

```rust
//! Wundler Graph Analyzer — consumes the Summarizer's `Vec<BundleGraphNode>`
//! and produces a chunked, reachability-pruned, dead-code-eliminated
//! `ChunkManifest` describing one bundle-graph view.

pub mod analyzer;
pub mod chunks;
pub mod dce;
pub mod graph;
pub mod manifest;
pub mod reachability;
pub mod types;

pub use analyzer::{AnalysisResult, AnalysisStats, GraphAnalyzer};
pub use types::{Chunk, ChunkId, ChunkManifest, EntryPoint, LoadCondition};

pub fn hello() -> &'static str {
    "wundler-graph"
}
```

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-graph --test analyze_integration
cargo test -p wundler-graph
```

Expected: `analyze_integration` reports `test result: ok. 3 passed; 0 failed`; full crate run reports all tests passing.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-graph
git commit -m "wundler-graph: GraphAnalyzer::analyze() — wire reachability + DCE + chunks + manifest"
```

---

## Task 15 — `wundler analyze` CLI subcommand

**Files:**
- Modify: `crates/wundler-cli/Cargo.toml`
- Modify: `crates/wundler-cli/src/main.rs`
- Test:   `crates/wundler-cli/tests/cli_analyze.rs`

### Step 1: Write the failing test

- [ ] Create `crates/wundler-cli/tests/cli_analyze.rs`:

```rust
use std::process::Command;
use tempfile::tempdir;

fn cargo_bin() -> String {
    env!("CARGO_BIN_EXE_wundler").to_string()
}

#[test]
fn analyze_emits_manifest_json_for_minimal_project() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/index.ts"),
        "import { x } from './util';\nexport default x;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/util.ts"),
        "export const x = 1;\n",
    )
    .unwrap();

    let output = Command::new(cargo_bin())
        .arg("analyze")
        .arg(dir.path().join("src"))
        .arg("--entry")
        .arg(format!("/={}", dir.path().join("src/index.ts").display()))
        .output()
        .expect("spawn wundler");

    assert!(
        output.status.success(),
        "wundler analyze failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("stdout must be valid JSON");
    assert!(parsed.get("build_id").is_some());
    assert!(parsed.get("chunks").is_some());
    assert!(parsed.get("entry_chunks").is_some());
    let chunks = parsed["chunks"].as_array().unwrap();
    assert!(!chunks.is_empty(), "expected at least one chunk");
}

#[test]
fn analyze_supports_multiple_entries() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(dir.path().join("src/b.ts"), "export const b = 2;\n").unwrap();

    let output = Command::new(cargo_bin())
        .arg("analyze")
        .arg(dir.path().join("src"))
        .arg("--entry")
        .arg(format!("/a={}", dir.path().join("src/a.ts").display()))
        .arg("--entry")
        .arg(format!("/b={}", dir.path().join("src/b.ts").display()))
        .output()
        .expect("spawn wundler");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = parsed["entry_chunks"].as_object().unwrap();
    assert!(entries.contains_key("/a"));
    assert!(entries.contains_key("/b"));
}

#[test]
fn analyze_rejects_malformed_entry_argument() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.ts"), "export const a = 1;\n").unwrap();

    let output = Command::new(cargo_bin())
        .arg("analyze")
        .arg(dir.path().join("src"))
        .arg("--entry")
        .arg("no_equals_sign_here")
        .output()
        .expect("spawn wundler");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_lowercase().contains("entry"));
}
```

### Step 2: Run test, verify it FAILS

- [ ] Run:

```bash
cargo test -p wundler-cli --test cli_analyze
```

Expected failure: either `wundler-cli` does not yet depend on `wundler-graph` (compile error), or the `analyze` subcommand is missing (`error: unrecognized subcommand 'analyze'`).

### Step 3: Write minimal implementation

- [ ] Modify `crates/wundler-cli/Cargo.toml` so it depends on `wundler-graph` and `tempfile` (for test data). The complete `[dependencies]` and `[dev-dependencies]` sections:

```toml
[dependencies]
wundler-core = { path = "../wundler-core" }
wundler-graph = { path = "../wundler-graph" }
clap = { version = "4", features = ["derive"] }
anyhow = "1"
serde_json = "1"
walkdir = "2"
indicatif = "0.17"

[dev-dependencies]
tempfile = "3"
```

- [ ] Modify `crates/wundler-cli/src/main.rs` to add the `Analyze` subcommand. The complete file (replacing any existing `main.rs`):

```rust
//! Wundler CLI: summarize / validate-scale / analyze.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;
use wundler_core::summarizer::ModuleSummarizer;
use wundler_graph::analyzer::GraphAnalyzer;

#[derive(Parser)]
#[command(name = "wundler", version, about = "Wundler — continuously-maintained module graph")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarize one module or a directory tree and print the BundleGraphNodes as JSON.
    Summarize {
        /// File or directory to summarize.
        path: PathBuf,
    },
    /// Walk a directory, summarize every module, and print only aggregate stats.
    ValidateScale {
        path: PathBuf,
    },
    /// Run the Graph Analyzer over a directory and print the ChunkManifest as JSON.
    Analyze {
        /// Source root to scan.
        path: PathBuf,
        /// One or more entry-point bindings of the form `route=path`.
        /// Example: --entry /=src/index.ts --entry /admin=src/admin.ts
        #[arg(long = "entry", value_name = "ROUTE=PATH", required = true)]
        entry: Vec<String>,
        /// Module-in-N-chunks threshold for commons extraction.
        #[arg(long = "commons-threshold", default_value_t = 2)]
        commons_threshold: usize,
    },
}

fn parse_entry_arg(s: &str) -> Result<(String, PathBuf)> {
    let (route, path) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("--entry must be of the form ROUTE=PATH (got {s:?})"))?;
    if route.is_empty() {
        return Err(anyhow!("--entry ROUTE side must not be empty (got {s:?})"));
    }
    if path.is_empty() {
        return Err(anyhow!("--entry PATH side must not be empty (got {s:?})"));
    }
    Ok((route.to_string(), PathBuf::from(path)))
}

fn run_summarize(path: &std::path::Path) -> Result<()> {
    let summarizer = ModuleSummarizer::default();
    let nodes = if path.is_dir() {
        summarizer.summarize_directory(path)?
    } else {
        vec![summarizer.summarize(path)?]
    };
    println!("{}", serde_json::to_string_pretty(&nodes)?);
    Ok(())
}

fn run_validate_scale(path: &std::path::Path) -> Result<()> {
    let summarizer = ModuleSummarizer::default();
    let nodes = summarizer.summarize_directory(path)?;
    println!(
        r#"{{"modules":{},"path":"{}"}}"#,
        nodes.len(),
        path.display()
    );
    Ok(())
}

fn run_analyze(
    path: &std::path::Path,
    entry_args: &[String],
    commons_threshold: usize,
) -> Result<()> {
    let mut entry_points: HashMap<String, PathBuf> = HashMap::new();
    for raw in entry_args {
        let (route, p) = parse_entry_arg(raw)?;
        entry_points.insert(route, p);
    }
    let summarizer = ModuleSummarizer::default();
    let nodes = summarizer
        .summarize_directory(path)
        .with_context(|| format!("summarizing {}", path.display()))?;
    let mut analyzer = GraphAnalyzer::new(entry_points);
    analyzer.commons_threshold = commons_threshold;
    let result = analyzer.analyze(nodes)?;
    println!("{}", result.manifest.to_json()?);
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Summarize { path } => run_summarize(&path),
        Command::ValidateScale { path } => run_validate_scale(&path),
        Command::Analyze {
            path,
            entry,
            commons_threshold,
        } => run_analyze(&path, &entry, commons_threshold),
    }
}
```

> **Note on `ModuleSummarizer::default()`:** Plan 1 exposes `ModuleSummarizer::new()`. If Plan 1's struct does not implement `Default`, replace `ModuleSummarizer::default()` with `ModuleSummarizer::new()` (and `wundler-core` must export `summarize_directory` as a method on the summarizer per Plan 1's published interface).

### Step 4: Run test, verify it PASSES

- [ ] Run:

```bash
cargo test -p wundler-cli --test cli_analyze
cargo build -p wundler-cli
cargo test -p wundler-graph
```

Expected: `cli_analyze` reports `test result: ok. 3 passed; 0 failed`; `wundler-cli` builds cleanly; `wundler-graph` regression suite still passes.

- [ ] Optional manual smoke test (outside the test runner):

```bash
mkdir -p /tmp/wundler-demo/src
echo 'import { x } from "./util"; export default x;' > /tmp/wundler-demo/src/index.ts
echo 'export const x = 1;' > /tmp/wundler-demo/src/util.ts
cargo run -p wundler-cli -- analyze /tmp/wundler-demo/src \
  --entry /=/tmp/wundler-demo/src/index.ts | jq .
```

Expected: a pretty-printed JSON ChunkManifest with one INITIAL chunk containing both modules and a `build_id` field.

### Step 5: Commit

- [ ] Run:

```bash
git add crates/wundler-cli
git commit -m "wundler-cli: add 'analyze' subcommand — summarize → GraphAnalyzer → ChunkManifest JSON"
```

---

## Post-Implementation Verification

After all 15 tasks are complete, run the full suite once more:

```bash
cd /Users/ken/workspace/ms/wundler
cargo build --workspace
cargo test --workspace
```

Both must succeed with zero failures. Confirm the public surface matches what Plan 3 expects:

```bash
grep -rn "pub fn analyze"              crates/wundler-graph/src
grep -rn "pub struct GraphAnalyzer"    crates/wundler-graph/src
grep -rn "pub struct AnalysisResult"   crates/wundler-graph/src
grep -rn "pub struct ChunkManifest"    crates/wundler-graph/src
grep -rn "pub struct Chunk"            crates/wundler-graph/src
grep -rn "pub enum LoadCondition"      crates/wundler-graph/src
grep -rn "pub fn to_json"              crates/wundler-graph/src
grep -rn "pub fn from_json"            crates/wundler-graph/src
```

Each must return at least one hit. With this in place, Plan 3 (Level 0 Build Pipeline) can begin.
