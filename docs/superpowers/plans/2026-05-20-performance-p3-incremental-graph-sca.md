# Performance P3 SCA — Incremental Graph Analysis (SCC cache)

**Date:** 2026-05-20
**Owner:** cloudpack-graph / cloudpack-pipeline
**Scope:** P3 SCA only — Tarjan SCC cache with adjacency-based invalidation
**Status:** Ready to implement

---

## Goal

Cache `cloudpack_graph::graph::tarjan_sccs` output across consecutive analyses driven
by the dev loop and invalidate it only when the resolved import adjacency changes.

This is the **SCA wedge** of P3:
> "Cache `tarjan_sccs` output across analyses, invalidate only when an edge is added
> or removed. ~100 LoC, gives ~30 % of full win. Exercises the cache-invalidation
> discipline that full P3 demands."

The deliverable is a new `IncrementalAnalyzer` type that produces output
**bit-identical** to `GraphAnalyzer::analyze` for every reachable graph state.

### THE invariant (P0)

For every valid graph state `G` and every diff applied through the dev loop:

```
IncrementalAnalyzer::analyze(G).manifest.to_json()  ==
GraphAnalyzer::new(entries).analyze(G).manifest.to_json()
```

Enforced by the **V6 mandatory property test** (Task 3). Any divergence blocks
merge and is treated as a P0 regression.

### Out of scope

* Reachability delta (full P3, later milestone)
* Chunk-assignment delta (full P3, later milestone)
* DCE delta (full P3, later milestone)
* Wiring `run_analyze_incremental` into the live `DevServer` file-watcher
  callback. The dev server today does not re-run analysis on change — it only
  emits SSE `change` events and lets the browser reload. The plumbing point
  where `run_analyze_incremental` will be invoked is added in Task 4 and
  unit-tested in isolation; a future PR connects it to the watcher.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  BuildPipeline (cloudpack-pipeline)                                   │
│  ───────────────────────────────                                    │
│  config: BuildConfig                                                │
│  engine: Arc<dyn TransformEngine>                                   │
│  incremental: Option<IncrementalAnalyzer>   ← NEW (lazily init)     │
│                                                                     │
│  fn run_analyze(&self, nodes)         → uses GraphAnalyzer (build)  │
│  fn run_analyze_incremental(&mut, n)  → uses IncrementalAnalyzer    │
└──────────────────┬──────────────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  IncrementalAnalyzer (cloudpack-graph::incremental)        ← NEW      │
│  ─────────────────────────────────────────────────                  │
│  entry_points: HashMap<String, PathBuf>                             │
│  commons_threshold: usize  (default 2)                              │
│  cached_adj: Option<HashMap<ContentHash, Vec<ContentHash>>>         │
│  cached_sccs: Option<Vec<Vec<ContentHash>>>                         │
│  bailout_count: u64           ← exported metric                     │
│  tarjan_calls: u64            ← exported metric                     │
│                                                                     │
│  fn analyze(&mut self, nodes) -> Result<AnalysisResult>             │
│    1. resolve_entries(&entry_points, &nodes)                        │
│    2. new_adj = build_adjacency(&nodes)                             │
│    3. if cached_adj == Some(&new_adj):   HIT  → reuse cached_sccs   │
│       else:                              MISS → tarjan_sccs(...)    │
│                                                  bailout_count++    │
│                                                  if had prior cache │
│    4. compute_reachability_with_cached_sccs(&nodes, &set, &sccs)    │
│    5. compute_dead_exports / annotate / assign_chunks / manifest    │
└─────────────────────────────────────────────────────────────────────┘
```

`GraphAnalyzer::analyze` is **unchanged in behaviour** — it still drives the
one-shot `cloudpack build`. Internally, Task 1 introduces a shared
`pub(crate) resolve_entries(...)` helper that both analyzers call so the
entry-resolution logic does not diverge between the two pipelines.

### Cache semantics

| Event                                       | Result                                                |
|---------------------------------------------|-------------------------------------------------------|
| First call, cache empty                     | Cold miss; populate `cached_adj` + `cached_sccs`. No bailout increment. |
| Subsequent call, `cached_adj == new_adj`    | **HIT** — reuse cached SCCs. No tarjan call.          |
| Subsequent call, `cached_adj != new_adj`    | **MISS** — recompute SCCs, replace cache, **bailout_count += 1**. |

Adjacency equality is plain `==` on `HashMap<ContentHash, Vec<ContentHash>>`:
- Outer map: equality compares as a set of `(key, value)` pairs.
- Inner `Vec<ContentHash>`: order-sensitive equality. `build_adjacency`
  iterates `node.summary.imports` in source order, which is deterministic for
  unchanged source, so this is correct.

### Bailout policy

For SCA alone there is no fall-back to `GraphAnalyzer::analyze`: a cache miss
just recomputes SCCs and proceeds through the same pipeline. The
`bailout_count` field exists from day 1 so it can be repurposed for the full
P3 bailout policy (reachability/chunk delta failures) without an API change.

---

## Tech Stack

* `cloudpack-core` types: `BundleGraphNode`, `ContentHash`, `ModuleSummary`,
  `Import`, `ImportKind`.
* `cloudpack-graph`: `build_adjacency`, `tarjan_sccs`,
  `compute_reachability_with_sccs`, `compute_dead_exports`, `assign_chunks`,
  `build_manifest`, `AnalysisResult`, `AnalysisStats`, `GraphAnalyzer`,
  `ChunkManifest`.
* `cloudpack-pipeline`: `BuildPipeline`, `BuildConfig`.
* Std only: `std::collections::{HashMap, HashSet, VecDeque}`,
  `std::path::PathBuf`.
* `anyhow::{Result, Context, anyhow}`.
* Test deps already in `cloudpack-graph/Cargo.toml`: `serde_json`, `tempfile`.

No new crate dependencies.

---

## Scope Boundary

**In scope (this plan):**

1. `cloudpack_graph::reachability::compute_reachability_with_cached_sccs` —
   pure helper that takes pre-computed SCCs.
2. Refactor `compute_reachability_with_sccs` to a thin wrapper that calls
   `tarjan_sccs` then the new helper. **No semantic change.**
3. `pub(crate) fn cloudpack_graph::analyzer::resolve_entries(...)` — extracted
   from the existing `GraphAnalyzer::analyze` body. **No semantic change to
   the public method.**
4. `cloudpack_graph::incremental::IncrementalAnalyzer` — new public type, owns
   the SCC cache, exposes `bailout_count()` and `tarjan_calls()`.
5. `cloudpack_pipeline::pipeline::BuildPipeline::run_analyze_incremental` —
   new method, lazily constructs and reuses an `IncrementalAnalyzer`.
6. V6 mandatory property test in `crates/cloudpack-graph/tests/incremental_v6.rs`.

**Out of scope (this plan, called out so reviewers don't ask):**

* Modifying `GraphAnalyzer::analyze` semantics (refactor only).
* Hooking `run_analyze_incremental` into `DevServer`'s notify watcher.
* Reachability / chunk / DCE deltas.
* Metrics export to Prometheus / `build-stats.json` (the counter is
  programmatically readable; UI plumbing is a follow-on).

---

## File Structure

```
crates/cloudpack-graph/
├── src/
│   ├── lib.rs                       (modify: pub mod incremental + re-export)
│   ├── analyzer.rs                  (modify: extract resolve_entries)
│   ├── reachability.rs              (modify: add helper, refactor wrapper)
│   └── incremental.rs               (NEW: IncrementalAnalyzer + 5 unit tests)
└── tests/
    └── incremental_v6.rs            (NEW: V6 invariant property test)

crates/cloudpack-pipeline/
├── src/
│   ├── lib.rs                       (modify: re-export IncrementalAnalyzer)
│   └── pipeline.rs                  (modify: incremental field +
│                                             run_analyze_incremental)
└── tests/
    └── pipeline_incremental.rs      (NEW: BuildPipeline integration test)
```

**Existing tests that must still pass after every commit in this plan:**

* `cargo test -p cloudpack-graph` — currently 47 tests across 14 files.
* `cargo test -p cloudpack-pipeline` — full suite (the pipeline integration tests).
* `cargo test -p cloudpack-cli` — full suite.

---

## Tasks

> Order rationale: Task 1 (helper) is a dependency of Task 2
> (`IncrementalAnalyzer`). Task 3 (V6) is the merge gate. Task 4 (pipeline)
> consumes Task 2's public API. Each task is committed independently and the
> full workspace must build and test green at every commit boundary.

---

### Task 1 — `compute_reachability_with_cached_sccs` helper + `resolve_entries` extraction

**Files modified:** `crates/cloudpack-graph/src/reachability.rs`,
`crates/cloudpack-graph/src/analyzer.rs`.

**TDD step 1: write the failing test.**

Append to `crates/cloudpack-graph/tests/reachability_scc.rs` (or create a new
file if `reachability_scc.rs` is already large — check before editing):

```rust
// crates/cloudpack-graph/tests/reachability_scc.rs (additions)

use cloudpack_graph::graph::{build_adjacency, tarjan_sccs};
use cloudpack_graph::reachability::{
    compute_reachability_with_cached_sccs, compute_reachability_with_sccs,
};

#[test]
fn cached_sccs_helper_matches_wrapper_on_acyclic_graph() {
    // Build a small acyclic graph: a -> b -> c, with `a` as entry.
    let nodes = vec![
        make_node("a", &["b"]),
        make_node("b", &["c"]),
        make_node("c", &[]),
    ];
    let entry_hashes: HashSet<ContentHash> =
        std::iter::once(nodes[0].id.clone()).collect();

    let adj = build_adjacency(&nodes);
    let sccs = tarjan_sccs(&nodes, &adj);

    let from_wrapper = compute_reachability_with_sccs(&nodes, &entry_hashes);
    let from_helper =
        compute_reachability_with_cached_sccs(&nodes, &entry_hashes, &sccs);

    assert_eq!(
        from_wrapper, from_helper,
        "helper with pre-computed SCCs must match the wrapper"
    );
    assert_eq!(from_helper.len(), 3, "all three nodes are reachable");
}

#[test]
fn cached_sccs_helper_matches_wrapper_on_cyclic_graph() {
    // Cycle: a -> b -> c -> a, plus dead node d.
    let nodes = vec![
        make_node("a", &["b"]),
        make_node("b", &["c"]),
        make_node("c", &["a"]),
        make_node("d", &[]),
    ];
    let entry_hashes: HashSet<ContentHash> =
        std::iter::once(nodes[0].id.clone()).collect();

    let adj = build_adjacency(&nodes);
    let sccs = tarjan_sccs(&nodes, &adj);

    let from_wrapper = compute_reachability_with_sccs(&nodes, &entry_hashes);
    let from_helper =
        compute_reachability_with_cached_sccs(&nodes, &entry_hashes, &sccs);

    assert_eq!(from_wrapper, from_helper);
    assert_eq!(from_helper.len(), 3, "a,b,c are alive; d is dead");
    assert!(!from_helper.contains(&nodes[3].id));
}
```

`make_node` already exists in `reachability_scc.rs`. If it does not, define
it at the top of the test file as:

```rust
use std::collections::HashSet;
use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary,
    SideEffectMarker,
};

fn make_node(path: &str, imports: &[&str]) -> BundleGraphNode {
    BundleGraphNode {
        id: ContentHash::from_source(path),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: Vec::new(),
            imports: imports
                .iter()
                .map(|s| Import {
                    specifier: (*s).to_string(),
                    kind: ImportKind::Named,
                    bindings: Vec::new(),
                    is_dynamic: false,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: Vec::new(),
            ambient_refs: Vec::new(),
        },
        alive: false,
        chunk_id: None,
        source: None,
    }
}
```

Run:

```bash
cargo test -p cloudpack-graph --test reachability_scc \
    -- cached_sccs_helper_matches_wrapper_on_acyclic_graph \
       cached_sccs_helper_matches_wrapper_on_cyclic_graph
```

**Expected output (red):**

```
error[E0432]: unresolved import `cloudpack_graph::reachability::compute_reachability_with_cached_sccs`
```

**TDD step 2: implement.**

Edit `crates/cloudpack-graph/src/reachability.rs`. Replace the existing
`compute_reachability_with_sccs` body with a wrapper, and add the new helper
plus a thin documentation update. The full diff is:

```rust
// crates/cloudpack-graph/src/reachability.rs

pub fn compute_reachability_with_sccs(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
) -> HashSet<ContentHash> {
    if entry_hashes.is_empty() {
        return HashSet::new();
    }
    let adj = build_adjacency(nodes);
    let sccs = tarjan_sccs(nodes, &adj);
    compute_reachability_with_cached_sccs(nodes, entry_hashes, &sccs)
}

/// Like [`compute_reachability_with_sccs`] but consumes pre-computed SCCs.
///
/// Used by [`crate::incremental::IncrementalAnalyzer`] to skip the
/// `tarjan_sccs` call when the dependency adjacency has not changed since
/// the previous analysis.
///
/// Behaviour is **identical** to `compute_reachability_with_sccs` when
/// `sccs == tarjan_sccs(nodes, &build_adjacency(nodes))`.  Passing stale SCCs
/// (computed against a different node slice) yields undefined results — the
/// caller is responsible for invalidating the cache on adjacency change.
pub fn compute_reachability_with_cached_sccs(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
    sccs: &[Vec<ContentHash>],
) -> HashSet<ContentHash> {
    if entry_hashes.is_empty() {
        return HashSet::new();
    }

    let adj = build_adjacency(nodes);

    let mut scc_of: HashMap<ContentHash, usize> = HashMap::new();
    for (idx, component) in sccs.iter().enumerate() {
        for hash in component {
            scc_of.insert(hash.clone(), idx);
        }
    }

    let mut alive: HashSet<ContentHash> = HashSet::new();
    let mut alive_scc: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    let known: HashSet<&ContentHash> = nodes.iter().map(|n| &n.id).collect();
    for entry in entry_hashes {
        if known.contains(entry) {
            if let Some(&scc_idx) = scc_of.get(entry) {
                activate_scc(scc_idx, sccs, &mut alive, &mut queue, &mut alive_scc);
            }
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(targets) = adj.get(&current) {
            for target in targets {
                if let Some(&scc_idx) = scc_of.get(target) {
                    activate_scc(scc_idx, sccs, &mut alive, &mut queue, &mut alive_scc);
                }
            }
        }
    }

    alive
}
```

The existing private `activate_scc` is unchanged.

**TDD step 3: extract `resolve_entries`.**

Edit `crates/cloudpack-graph/src/analyzer.rs`. Replace the entry-resolution
block inside `GraphAnalyzer::analyze` (currently lines 94–139) with a single
call to a new `pub(crate) fn`. Add the function at module level:

```rust
// crates/cloudpack-graph/src/analyzer.rs (additions, near the top of impl block)

/// Resolve every entry-point path to a `ContentHash`.
///
/// Returns:
/// * `entry_hashes` — `route → hash` map preserving the original keys.
/// * `entry_hash_set` — the same hashes as a `HashSet` for membership tests.
///
/// Tries three resolution strategies in order:
/// 1. Exact match against `node.path`.
/// 2. `"./"`-prefixed match (handles `WalkDir` from scan root `"."`).
/// 3. Suffix match `"/<path>"` (handles absolute scan roots).
pub(crate) fn resolve_entries(
    entry_points: &HashMap<String, PathBuf>,
    nodes: &[BundleGraphNode],
) -> Result<(HashMap<String, ContentHash>, HashSet<ContentHash>)> {
    let path_to_hash: HashMap<&str, ContentHash> = nodes
        .iter()
        .map(|n| (n.path.as_str(), n.id.clone()))
        .collect();

    let mut entry_hashes: HashMap<String, ContentHash> = HashMap::new();
    let mut entry_hash_set: HashSet<ContentHash> = HashSet::new();

    for (route, path) in entry_points {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("route {route:?}: path is not valid UTF-8"))?;

        let hash = path_to_hash
            .get(path_str)
            .or_else(|| path_to_hash.get(format!("./{path_str}").as_str()))
            .or_else(|| {
                let suffix = format!("/{path_str}");
                nodes
                    .iter()
                    .find(|n| n.path.ends_with(&suffix))
                    .and_then(|n| path_to_hash.get(n.path.as_str()))
            })
            .ok_or_else(|| {
                anyhow!("route {route:?}: path {path_str:?} not found in graph")
            })?;

        entry_hashes.insert(route.clone(), hash.clone());
        entry_hash_set.insert(hash.clone());
    }

    Ok((entry_hashes, entry_hash_set))
}
```

Then in `GraphAnalyzer::analyze`, replace the resolution block with:

```rust
let (entry_hashes, entry_hash_set) =
    resolve_entries(&self.entry_points, &nodes)?;
```

This is a pure refactor: identical behaviour, identical return values,
identical error messages.

**TDD step 4: run all cloudpack-graph tests.**

```bash
cargo test -p cloudpack-graph
```

**Expected output (green):**

```
test result: ok. 49 passed; 0 failed; 0 ignored
```

(47 pre-existing tests + 2 new tests added in this task.)

**Commit message:**

```
feat(graph): extract reachability/entry helpers for incremental analysis

* Add compute_reachability_with_cached_sccs(nodes, entries, &sccs) —
  pure helper that consumes pre-computed SCCs.
* Refactor compute_reachability_with_sccs into a thin wrapper that
  calls tarjan_sccs then the helper. Behaviour preserved.
* Extract pub(crate) resolve_entries(...) from GraphAnalyzer::analyze
  so it can be shared with IncrementalAnalyzer. Behaviour preserved.
* +2 unit tests asserting wrapper == helper on acyclic and cyclic
  graphs.

No public API changes. Prep work for P3 SCA (Tarjan SCC cache).
```

---

### Task 2 — `IncrementalAnalyzer` with SCC cache

**Files:** new `crates/cloudpack-graph/src/incremental.rs`, modify
`crates/cloudpack-graph/src/lib.rs`.

**TDD step 1: write the failing tests** (in `incremental.rs`).

```rust
// crates/cloudpack-graph/src/incremental.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use cloudpack_core::types::{
        BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary,
        SideEffectMarker,
    };

    fn make_node(path: &str, imports: &[&str]) -> BundleGraphNode {
        BundleGraphNode {
            id: ContentHash::from_source(&format!(
                "{}|{}",
                path,
                imports.join(",")
            )),
            path: path.to_string(),
            summary: ModuleSummary {
                exports: Vec::new(),
                imports: imports
                    .iter()
                    .map(|s| Import {
                        specifier: (*s).to_string(),
                        kind: ImportKind::Named,
                        bindings: Vec::new(),
                        is_dynamic: false,
                    })
                    .collect(),
                side_effects: SideEffectMarker::None,
                call_edges: Vec::new(),
                ambient_refs: Vec::new(),
            },
            alive: false,
            chunk_id: None,
            source: None,
        }
    }

    fn entries() -> HashMap<String, PathBuf> {
        [("/main".to_string(), PathBuf::from("a"))].into_iter().collect()
    }

    fn three_node_graph() -> Vec<BundleGraphNode> {
        vec![
            make_node("a", &["b"]),
            make_node("b", &["c"]),
            make_node("c", &[]),
        ]
    }

    #[test]
    fn scc_cache_is_reused_when_adjacency_unchanged() {
        let mut analyzer = IncrementalAnalyzer::new(entries());

        let _ = analyzer.analyze(three_node_graph()).expect("first analyze");
        assert_eq!(analyzer.tarjan_calls(), 1, "cold start runs tarjan once");
        assert_eq!(analyzer.bailout_count(), 0, "no prior cache to invalidate");

        let _ = analyzer.analyze(three_node_graph()).expect("second analyze");
        assert_eq!(
            analyzer.tarjan_calls(),
            1,
            "second analyze with unchanged adjacency must reuse cached SCCs"
        );
        assert_eq!(analyzer.bailout_count(), 0, "still no bailouts");
    }

    #[test]
    fn scc_cache_is_invalidated_when_edge_added() {
        let mut analyzer = IncrementalAnalyzer::new(entries());

        let _ = analyzer.analyze(three_node_graph()).expect("first");
        assert_eq!(analyzer.tarjan_calls(), 1);

        // Add an edge: a -> c (previously a -> b only).
        let edited = vec![
            make_node("a", &["b", "c"]),
            make_node("b", &["c"]),
            make_node("c", &[]),
        ];
        let _ = analyzer.analyze(edited).expect("second");

        assert_eq!(
            analyzer.tarjan_calls(),
            2,
            "added edge must invalidate cache and re-run tarjan"
        );
        assert_eq!(analyzer.bailout_count(), 1, "one bailout recorded");
    }

    #[test]
    fn scc_cache_is_invalidated_when_edge_removed() {
        let mut analyzer = IncrementalAnalyzer::new(entries());

        // Start with a -> b, a -> c, b -> c.
        let initial = vec![
            make_node("a", &["b", "c"]),
            make_node("b", &["c"]),
            make_node("c", &[]),
        ];
        let _ = analyzer.analyze(initial).expect("first");
        assert_eq!(analyzer.tarjan_calls(), 1);

        // Remove edge a -> c.
        let edited = vec![
            make_node("a", &["b"]),
            make_node("b", &["c"]),
            make_node("c", &[]),
        ];
        let _ = analyzer.analyze(edited).expect("second");

        assert_eq!(analyzer.tarjan_calls(), 2);
        assert_eq!(analyzer.bailout_count(), 1);
    }

    #[test]
    fn incremental_output_matches_fresh_on_first_run() {
        use crate::analyzer::GraphAnalyzer;

        let nodes = three_node_graph();

        let mut incremental = IncrementalAnalyzer::new(entries());
        let inc = incremental.analyze(nodes.clone()).expect("incremental");

        let full = GraphAnalyzer::new(entries())
            .analyze(nodes)
            .expect("full");

        assert_eq!(
            inc.manifest.to_json().expect("inc json"),
            full.manifest.to_json().expect("full json"),
            "cold-start incremental manifest must equal full-recompute manifest"
        );
        assert_eq!(inc.stats.total, full.stats.total);
        assert_eq!(inc.stats.alive, full.stats.alive);
        assert_eq!(inc.stats.dead, full.stats.dead);
        assert_eq!(inc.stats.chunks, full.stats.chunks);
    }

    #[test]
    fn incremental_output_matches_fresh_after_edit() {
        use crate::analyzer::GraphAnalyzer;

        let mut incremental = IncrementalAnalyzer::new(entries());

        // Warm cache with one graph.
        let _ = incremental.analyze(three_node_graph()).expect("warm");

        // Now edit the graph: add edge a -> c.
        let edited = vec![
            make_node("a", &["b", "c"]),
            make_node("b", &["c"]),
            make_node("c", &[]),
        ];
        let inc = incremental.analyze(edited.clone()).expect("inc");
        let full = GraphAnalyzer::new(entries())
            .analyze(edited)
            .expect("full");

        assert_eq!(
            inc.manifest.to_json().expect("inc json"),
            full.manifest.to_json().expect("full json"),
            "post-edit incremental manifest must equal full-recompute manifest"
        );
    }
}
```

Run:

```bash
cargo test -p cloudpack-graph --lib incremental
```

**Expected output (red):**

```
error[E0433]: failed to resolve: could not find `incremental` in `cloudpack_graph`
```

**TDD step 2: implement.**

Create `crates/cloudpack-graph/src/incremental.rs`:

```rust
//! Incremental dependency-graph analyzer with a Tarjan-SCC cache.
//!
//! `IncrementalAnalyzer` is the "SCA" wedge of the P3 incremental graph plan:
//! it caches the output of [`tarjan_sccs`] across consecutive `analyze` calls
//! and reuses it whenever the resolved import adjacency is unchanged.
//!
//! # Correctness invariant (P0)
//!
//! For every valid graph state `G`:
//!
//! ```text
//! IncrementalAnalyzer::analyze(G).manifest.to_json() ==
//! GraphAnalyzer::new(entries).analyze(G).manifest.to_json()
//! ```
//!
//! Enforced by `crates/cloudpack-graph/tests/incremental_v6.rs`. Any divergence
//! is treated as a P0 regression.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use cloudpack_core::types::{BundleGraphNode, ContentHash};

use crate::analyzer::{resolve_entries, AnalysisResult, AnalysisStats};
use crate::chunks::assign_chunks;
use crate::dce::compute_dead_exports;
use crate::graph::{build_adjacency, tarjan_sccs};
use crate::manifest::build_manifest;
use crate::reachability::compute_reachability_with_cached_sccs;

/// Incremental dependency-graph analyzer with a Tarjan-SCC cache.
pub struct IncrementalAnalyzer {
    /// Route name → entry-point path (same shape as [`crate::analyzer::GraphAnalyzer`]).
    pub entry_points: HashMap<String, PathBuf>,
    /// Commons-chunk inclusion threshold. Defaults to `2`.
    pub commons_threshold: usize,

    cached_adj: Option<HashMap<ContentHash, Vec<ContentHash>>>,
    cached_sccs: Option<Vec<Vec<ContentHash>>>,
    bailout_count: u64,
    tarjan_calls: u64,
}

impl IncrementalAnalyzer {
    /// Create a new analyzer with an empty cache and `commons_threshold = 2`.
    pub fn new(entry_points: HashMap<String, PathBuf>) -> Self {
        Self {
            entry_points,
            commons_threshold: 2,
            cached_adj: None,
            cached_sccs: None,
            bailout_count: 0,
            tarjan_calls: 0,
        }
    }

    /// Number of times the SCC cache had to be invalidated and recomputed
    /// because the resolved adjacency changed since the previous call.
    ///
    /// **Not** incremented for the cold-start cache fill on the very first
    /// call (there is no prior state to invalidate).
    pub fn bailout_count(&self) -> u64 {
        self.bailout_count
    }

    /// Number of times [`tarjan_sccs`] was actually executed. Useful in tests
    /// to assert cache hits.
    pub fn tarjan_calls(&self) -> u64 {
        self.tarjan_calls
    }

    /// Run the full analysis pipeline. Output is bit-identical to
    /// [`crate::analyzer::GraphAnalyzer::analyze`] on the same input.
    pub fn analyze(&mut self, mut nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        // ── Step 1: Resolve entries (shared with GraphAnalyzer) ──────────
        let (entry_hashes, entry_hash_set) =
            resolve_entries(&self.entry_points, &nodes)?;

        // ── Step 2: Build adjacency and check the cache ──────────────────
        let new_adj = build_adjacency(&nodes);
        let cache_hit = matches!(
            &self.cached_adj,
            Some(prev_adj) if *prev_adj == new_adj
        );

        if !cache_hit {
            if self.cached_adj.is_some() {
                // Prior cache existed but is now stale → bailout.
                self.bailout_count += 1;
            }
            let sccs = tarjan_sccs(&nodes, &new_adj);
            self.tarjan_calls += 1;
            self.cached_adj = Some(new_adj);
            self.cached_sccs = Some(sccs);
        }

        let sccs_ref: &[Vec<ContentHash>] = self
            .cached_sccs
            .as_deref()
            .expect("cached_sccs is Some after the branch above");

        // ── Step 3: Reachability + DCE ──────────────────────────────────
        let alive =
            compute_reachability_with_cached_sccs(&nodes, &entry_hash_set, sccs_ref);
        let dead_exports = compute_dead_exports(&nodes, &alive);

        // ── Step 4: Annotate `alive` + trim dead Named exports ──────────
        for n in &mut nodes {
            n.alive = alive.contains(&n.id);
            if n.alive {
                if let Some(dead_set) = dead_exports.get(&n.id) {
                    if !dead_set.is_empty() {
                        n.summary.exports.retain(|e| !dead_set.contains(&e.name));
                    }
                }
            }
        }

        // ── Step 5: Chunk assignment ────────────────────────────────────
        let (mut chunks, module_index) =
            assign_chunks(&nodes, &alive, &entry_hashes, self.commons_threshold);

        for n in &mut nodes {
            n.chunk_id = module_index.get(&n.id).cloned();
        }

        chunks.sort_by(|a, b| a.id.cmp(&b.id));

        // ── Step 6: Manifest + stats ────────────────────────────────────
        let manifest = build_manifest(chunks, &entry_hashes, module_index);
        let total = nodes.len();
        let alive_count = nodes.iter().filter(|n| n.alive).count();
        let dead_count = total - alive_count;
        let chunks_count = manifest.chunks.len();

        let stats = AnalysisStats {
            total,
            alive: alive_count,
            dead: dead_count,
            chunks: chunks_count,
        };

        Ok(AnalysisResult {
            nodes,
            manifest,
            stats,
        })
    }
}
```

Edit `crates/cloudpack-graph/src/lib.rs` — add module and re-export:

```rust
pub mod analyzer;
pub mod chunks;
pub mod dce;
pub mod graph;
pub mod incremental;          // NEW
pub mod manifest;
pub mod reachability;
pub mod types;

pub use analyzer::{AnalysisResult, AnalysisStats, GraphAnalyzer};
pub use incremental::IncrementalAnalyzer;   // NEW
pub use types::{Chunk, ChunkId, ChunkManifest, EntryPoint, LoadCondition};
```

`resolve_entries` is `pub(crate)`, so `incremental.rs` (same crate) can call it
directly via `use crate::analyzer::resolve_entries`.

**TDD step 3: run tests.**

```bash
cargo test -p cloudpack-graph --lib incremental
```

**Expected output (green):**

```
running 5 tests
test incremental::tests::scc_cache_is_reused_when_adjacency_unchanged ... ok
test incremental::tests::scc_cache_is_invalidated_when_edge_added ... ok
test incremental::tests::scc_cache_is_invalidated_when_edge_removed ... ok
test incremental::tests::incremental_output_matches_fresh_on_first_run ... ok
test incremental::tests::incremental_output_matches_fresh_after_edit ... ok

test result: ok. 5 passed; 0 failed; 0 ignored
```

Then verify nothing regressed in the broader graph suite:

```bash
cargo test -p cloudpack-graph
```

**Expected:**

```
test result: ok. 54 passed; 0 failed; 0 ignored
```

(49 from Task 1 + 5 from Task 2.)

**Commit message:**

```
feat(graph): add IncrementalAnalyzer with Tarjan-SCC cache (P3 SCA)

Introduces cloudpack_graph::incremental::IncrementalAnalyzer, an
incremental dependency-graph analyzer that caches tarjan_sccs output
across analyze() calls and invalidates it only when the resolved
import adjacency changes.

* Same pipeline as GraphAnalyzer::analyze — output is bit-identical.
* Cache hit ⇒ skips Tarjan, reuses cached SCC vector.
* Cache miss ⇒ recomputes SCCs, increments bailout_count
  (cold-start fills don't count as bailouts).
* Exposes bailout_count() and tarjan_calls() metrics.

+5 unit tests for cache hit/miss behaviour and correctness vs.
GraphAnalyzer on small graphs. The mandatory V6 property test
(landing in the next commit) is what actually enforces the
correctness invariant.
```

---

### Task 3 — V6 mandatory property test

**File:** new `crates/cloudpack-graph/tests/incremental_v6.rs`.

**This test is the merge gate.** It must pass before any P3 code (including
this plan's Tasks 1 and 2) lands on `main`. The test asserts THE invariant
across a series of synthetic graph edits.

**TDD step 1: write the failing test.**

```rust
// crates/cloudpack-graph/tests/incremental_v6.rs
//
// V6 mandatory property test for P3 SCA.
//
// Acceptance criterion:
//   cargo test -p cloudpack-graph --test incremental_v6
// reports
//   test result: ok. 1 passed; 0 failed
//
// Asserts: for every K ∈ {1, 5, 20} and every step s ∈ [0, K),
//   IncrementalAnalyzer::analyze(G_s).manifest.to_json()
//   == GraphAnalyzer::new(entries).analyze(G_s).manifest.to_json()
// where G_s is the graph after applying s deterministic edits.

use std::collections::HashMap;
use std::path::PathBuf;

use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary,
    SideEffectMarker,
};
use cloudpack_graph::analyzer::GraphAnalyzer;
use cloudpack_graph::incremental::IncrementalAnalyzer;

const N_NODES: usize = 30;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn module_path(i: usize) -> String {
    format!("m{i}")
}

/// Compute a content-addressed id from path + imports. The id MUST change
/// whenever the import list changes (mirrors what the real summarizer does)
/// so that the SCC cache key (`build_adjacency` output) reflects real edits.
fn node_id(path: &str, imports: &[String]) -> ContentHash {
    let mut buf = String::with_capacity(path.len() + 64);
    buf.push_str(path);
    buf.push('|');
    for imp in imports {
        buf.push_str(imp);
        buf.push(',');
    }
    ContentHash::from_source(&buf)
}

fn make_node(path: &str, imports: Vec<String>) -> BundleGraphNode {
    BundleGraphNode {
        id: node_id(path, &imports),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: Vec::new(),
            imports: imports
                .into_iter()
                .map(|specifier| Import {
                    specifier,
                    kind: ImportKind::Named,
                    bindings: Vec::new(),
                    is_dynamic: false,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: Vec::new(),
            ambient_refs: Vec::new(),
        },
        alive: false,
        chunk_id: None,
        source: None,
    }
}

/// Tiny deterministic 64-bit LCG. Test-local; no extra crate dependency.
fn lcg_next(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state
}

/// Build a synthetic N-node dependency graph with one weak cycle.
///
/// Layout:
/// * m0 is the entry. m0 → m1, m0 → m2.
/// * m{i} → m{i+1}  for i in 1..N-1  (a long chain)
/// * m{N-1} → m1                     (one back-edge — creates a cycle)
/// * m2 → m{N-1}                      (cross edge to exercise SCC handling)
fn build_synth_graph(n: usize) -> Vec<BundleGraphNode> {
    assert!(n >= 5, "synthesizer needs at least 5 nodes");
    let paths: Vec<String> = (0..n).map(module_path).collect();

    let mut nodes = Vec::with_capacity(n);
    for i in 0..n {
        let imports: Vec<String> = match i {
            0 => vec![paths[1].clone(), paths[2].clone()],
            i if i == n - 1 => vec![paths[1].clone()],
            2 => vec![paths[3].clone(), paths[n - 1].clone()],
            i => vec![paths[i + 1].clone()],
        };
        nodes.push(make_node(&paths[i], imports));
    }
    nodes
}

/// Apply one deterministic edit indexed by `step`.
///
/// Edits cycle through four shapes so we cover add-edge, remove-edge,
/// rename-target, and add-back-edge across all K ranges:
/// * step % 4 == 0 → m{a} adds an import of m{b}     (add edge)
/// * step % 4 == 1 → m{a} removes its first import   (remove edge)
/// * step % 4 == 2 → m{a} replaces its first import  (rename target)
/// * step % 4 == 3 → m{a} adds back-edge to m1        (extend SCC)
///
/// `a` and `b` are chosen deterministically from `step` so that running
/// the same step on a freshly-built graph is reproducible.
fn apply_edit(nodes: &mut [BundleGraphNode], step: usize) {
    let n = nodes.len();
    let mut rng = (step as u64).wrapping_add(0xDEAD_BEEF);

    let a = (lcg_next(&mut rng) as usize % (n - 1)) + 1; // avoid index 0 (entry)
    let b = (lcg_next(&mut rng) as usize % (n - 1)) + 1;

    match step % 4 {
        0 => {
            let target = module_path(b);
            if !nodes[a].summary.imports.iter().any(|i| i.specifier == target) {
                nodes[a].summary.imports.push(Import {
                    specifier: target,
                    kind: ImportKind::Named,
                    bindings: Vec::new(),
                    is_dynamic: false,
                });
            }
        }
        1 => {
            if !nodes[a].summary.imports.is_empty() {
                nodes[a].summary.imports.remove(0);
            }
        }
        2 => {
            if let Some(first) = nodes[a].summary.imports.first_mut() {
                first.specifier = module_path(b);
            }
        }
        _ => {
            let target = module_path(1);
            if !nodes[a].summary.imports.iter().any(|i| i.specifier == target) {
                nodes[a].summary.imports.push(Import {
                    specifier: target,
                    kind: ImportKind::Named,
                    bindings: Vec::new(),
                    is_dynamic: false,
                });
            }
        }
    }

    // Re-derive the node's id so build_adjacency / cache keying see the change.
    let imports: Vec<String> = nodes[a]
        .summary
        .imports
        .iter()
        .map(|i| i.specifier.clone())
        .collect();
    nodes[a].id = node_id(&nodes[a].path, &imports);
}

fn entries() -> HashMap<String, PathBuf> {
    [("/main".to_string(), PathBuf::from("m0"))]
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// THE test
// ---------------------------------------------------------------------------

#[test]
fn v6_incremental_equals_full_recompute_for_k_in_1_5_20() {
    for &k in &[1usize, 5, 20] {
        let mut nodes = build_synth_graph(N_NODES);

        let mut incremental = IncrementalAnalyzer::new(entries());

        // Warm the cache with the unedited graph; assert the cold-start
        // result already matches a full recompute.
        {
            let inc = incremental
                .analyze(nodes.clone())
                .unwrap_or_else(|e| panic!("V6 warm-up incremental failed: {e}"));
            let full = GraphAnalyzer::new(entries())
                .analyze(nodes.clone())
                .unwrap_or_else(|e| panic!("V6 warm-up full failed: {e}"));
            assert_eq!(
                inc.manifest.to_json().expect("inc json"),
                full.manifest.to_json().expect("full json"),
                "V6: cold-start divergence (k={k})"
            );
        }

        // Apply K deterministic edits, asserting after each.
        for step in 0..k {
            apply_edit(&mut nodes, step);

            let inc = incremental
                .analyze(nodes.clone())
                .unwrap_or_else(|e| panic!("V6 incremental analyze failed at k={k} step={step}: {e}"));
            let full = GraphAnalyzer::new(entries())
                .analyze(nodes.clone())
                .unwrap_or_else(|e| panic!("V6 full analyze failed at k={k} step={step}: {e}"));

            assert_eq!(
                inc.manifest.to_json().expect("inc json"),
                full.manifest.to_json().expect("full json"),
                "V6: incremental output must be bit-identical to full recompute \
                 (k={k}, step={step}, bailout_count={})",
                incremental.bailout_count()
            );
        }
    }
}
```

Run:

```bash
cargo test -p cloudpack-graph --test incremental_v6
```

**Expected output (green) — after Tasks 1 and 2 are in place:**

```
running 1 test
test v6_incremental_equals_full_recompute_for_k_in_1_5_20 ... ok

test result: ok. 1 passed; 0 failed; 0 ignored
```

If this test fails at **any** step, **do not merge**. The failure means
incremental and full pipelines diverged. Likely causes (debug checklist):

1. `cached_adj == new_adj` returned `true` when adjacency actually differed
   (Vec-order non-determinism in `build_adjacency` — should be impossible
   given current implementation; verify).
2. `compute_reachability_with_cached_sccs` consumed stale SCCs (cache
   invalidation logic in `IncrementalAnalyzer::analyze` is wrong).
3. `node.id` was not recomputed after an import edit (test-side bug —
   `apply_edit` must re-hash the node).

**Commit message:**

```
test(graph): V6 mandatory property test for P3 SCA incremental analyzer

Adds crates/cloudpack-graph/tests/incremental_v6.rs — the merge-gate
test for P3.

For K ∈ {1, 5, 20}, builds a 30-node synthetic graph with one cycle,
applies K deterministic edits (add/remove/rename/back-edge), and
asserts after every step that
  IncrementalAnalyzer.analyze(G).manifest.to_json()
  == GraphAnalyzer::analyze(G).manifest.to_json().

Any divergence is treated as a P0 regression: this test gates every
future P3 change.

The test is self-contained: no external fixtures, deterministic LCG
seeded by step index, no extra crate dependencies.
```

---

### Task 4 — Wire `run_analyze_incremental` into `BuildPipeline`

**Files modified:** `crates/cloudpack-pipeline/src/pipeline.rs`,
`crates/cloudpack-pipeline/src/lib.rs`. New file:
`crates/cloudpack-pipeline/tests/pipeline_incremental.rs`.

**TDD step 1: write the failing integration test.**

Create `crates/cloudpack-pipeline/tests/pipeline_incremental.rs`:

```rust
//! Integration test: BuildPipeline::run_analyze_incremental.
//!
//! Asserts:
//! 1. First call populates the analyzer.
//! 2. Second call with identical nodes reuses the cached SCCs
//!    (analyzer.bailout_count() stays at 0).
//! 3. Second call with edited nodes invalidates the cache
//!    (analyzer.bailout_count() == 1).
//! 4. Output manifest matches `run_analyze` (the GraphAnalyzer path).

use std::collections::HashMap;
use std::path::PathBuf;

use cloudpack_core::types::{
    BundleGraphNode, ContentHash, Import, ImportKind, ModuleSummary,
    SideEffectMarker,
};
use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
use cloudpack_pipeline::pipeline::BuildPipeline;

fn make_node(path: &str, imports: &[&str]) -> BundleGraphNode {
    let id_src = format!("{path}|{}", imports.join(","));
    BundleGraphNode {
        id: ContentHash::from_source(&id_src),
        path: path.to_string(),
        summary: ModuleSummary {
            exports: Vec::new(),
            imports: imports
                .iter()
                .map(|s| Import {
                    specifier: (*s).to_string(),
                    kind: ImportKind::Named,
                    bindings: Vec::new(),
                    is_dynamic: false,
                })
                .collect(),
            side_effects: SideEffectMarker::None,
            call_edges: Vec::new(),
            ambient_refs: Vec::new(),
        },
        alive: false,
        chunk_id: None,
        source: None,
    }
}

fn make_config(tmp: &std::path::Path) -> BuildConfig {
    // BuildConfig has no Default impl; every field must be supplied.
    BuildConfig {
        root: tmp.to_path_buf(),
        out_dir: tmp.join("out"),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: [("/main".to_string(), PathBuf::from("a"))]
            .into_iter()
            .collect(),
        budget: None,
        dev: None,
    }
}

#[test]
fn run_analyze_incremental_caches_and_invalidates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut pipeline = BuildPipeline::new(make_config(tmp.path()));

    let nodes = vec![
        make_node("a", &["b"]),
        make_node("b", &["c"]),
        make_node("c", &[]),
    ];

    // First call: cold start.
    let r1 = pipeline
        .run_analyze_incremental(nodes.clone())
        .expect("first incremental analyze");
    assert_eq!(pipeline.incremental_bailout_count(), 0);
    assert_eq!(pipeline.incremental_tarjan_calls(), 1);

    // Second call, identical nodes: cache hit.
    let r2 = pipeline
        .run_analyze_incremental(nodes.clone())
        .expect("second incremental analyze");
    assert_eq!(pipeline.incremental_bailout_count(), 0);
    assert_eq!(pipeline.incremental_tarjan_calls(), 1, "cache hit expected");
    assert_eq!(
        r1.manifest.to_json().unwrap(),
        r2.manifest.to_json().unwrap()
    );

    // Third call, edited graph (add a -> c): cache miss.
    let edited = vec![
        make_node("a", &["b", "c"]),
        make_node("b", &["c"]),
        make_node("c", &[]),
    ];
    let r3 = pipeline
        .run_analyze_incremental(edited.clone())
        .expect("third incremental analyze");
    assert_eq!(pipeline.incremental_bailout_count(), 1, "edit triggers bailout");
    assert_eq!(pipeline.incremental_tarjan_calls(), 2);

    // r3 must match what `run_analyze` (the non-incremental path) produces
    // for the same edited graph.
    let full = pipeline
        .run_analyze(edited)
        .expect("non-incremental analyze");
    assert_eq!(
        r3.manifest.to_json().unwrap(),
        full.manifest.to_json().unwrap(),
        "incremental output must match GraphAnalyzer output (post-edit)"
    );
}
```

Run:

```bash
cargo test -p cloudpack-pipeline --test pipeline_incremental
```

**Expected output (red):**

```
error[E0599]: no method named `run_analyze_incremental` found for struct `BuildPipeline`
```

**TDD step 2: implement.**

Edit `crates/cloudpack-pipeline/src/pipeline.rs`. Add the import, field, and
methods. The complete diff to the file:

```rust
// crates/cloudpack-pipeline/src/pipeline.rs

use cloudpack_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use cloudpack_graph::incremental::IncrementalAnalyzer;          // NEW
use cloudpack_graph::types::ChunkManifest;
// (existing imports unchanged)

pub struct BuildPipeline {
    pub config: BuildConfig,
    pub engine: Arc<dyn TransformEngine>,
    /// Lazily-constructed incremental analyzer used by the dev/watch loop.
    /// `None` until `run_analyze_incremental` is first called.
    incremental: Option<IncrementalAnalyzer>,                  // NEW
}

impl BuildPipeline {
    pub fn new(config: BuildConfig) -> Self {
        let engine: Arc<dyn TransformEngine> = match config.engine {
            // (unchanged engine selection)
            EngineChoice::Swc => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
            EngineChoice::Rolldown => {
                Arc::new(RolldownAdapter::with_config(RolldownAdapterConfig::default()))
            }
            EngineChoice::Rspack => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
        };
        Self {
            config,
            engine,
            incremental: None,                                 // NEW
        }
    }

    // ... run_summarize, run_analyze, run_transform, build unchanged ...

    /// Step 2 (incremental variant): analyze with the Tarjan-SCC cache.
    ///
    /// Lazily constructs an [`IncrementalAnalyzer`] on first call, then
    /// reuses it across calls so its cache survives between invocations.
    /// Output is bit-identical to [`Self::run_analyze`] for the same input
    /// (enforced by `crates/cloudpack-graph/tests/incremental_v6.rs`).
    pub fn run_analyze_incremental(
        &mut self,
        nodes: Vec<BundleGraphNode>,
    ) -> Result<AnalysisResult> {
        let analyzer = self.incremental.get_or_insert_with(|| {
            let mut a = IncrementalAnalyzer::new(self.config.entry_points.clone());
            a.commons_threshold = self.config.commons_threshold;
            a
        });
        analyzer
            .analyze(nodes)
            .with_context(|| "incremental graph analysis failed")
    }

    /// Inspect the underlying incremental analyzer's bailout counter (i.e.,
    /// the number of times the SCC cache had to be invalidated). Returns `0`
    /// when [`Self::run_analyze_incremental`] has never been called.
    pub fn incremental_bailout_count(&self) -> u64 {
        self.incremental
            .as_ref()
            .map(|a| a.bailout_count())
            .unwrap_or(0)
    }

    /// Inspect the number of [`tarjan_sccs`] calls executed by the
    /// underlying incremental analyzer. Returns `0` when
    /// [`Self::run_analyze_incremental`] has never been called.
    pub fn incremental_tarjan_calls(&self) -> u64 {
        self.incremental
            .as_ref()
            .map(|a| a.tarjan_calls())
            .unwrap_or(0)
    }
}
```

Edit `crates/cloudpack-pipeline/src/lib.rs` — re-export `IncrementalAnalyzer`
so dev-loop integrators can refer to it from the `cloudpack_pipeline`
namespace without taking a direct dep on `cloudpack-graph`:

```rust
pub use config::{BuildConfig, DevConfig, EngineChoice};
pub use dev_server::DevServer;
pub use pipeline::{BuildOutput, BuildPipeline};
pub use cloudpack_graph::IncrementalAnalyzer;        // NEW (re-export)
```

**TDD step 3: run all the things.**

```bash
cargo test -p cloudpack-pipeline --test pipeline_incremental
```

**Expected output (green):**

```
running 1 test
test run_analyze_incremental_caches_and_invalidates ... ok

test result: ok. 1 passed; 0 failed; 0 ignored
```

Then the full pipeline suite (to confirm no regression in `build()` or any
other existing path):

```bash
cargo test -p cloudpack-pipeline
```

**Expected:** all existing pipeline tests + the new one pass, `0 failed`.

Then the whole workspace:

```bash
cargo build --workspace --all-targets
cargo test  --workspace
```

**Expected (final):** clean build, every crate's tests `ok`, `0 failed`,
including the V6 gate test:

```
     Running tests/incremental_v6.rs (target/debug/deps/incremental_v6-…)
running 1 test
test v6_incremental_equals_full_recompute_for_k_in_1_5_20 ... ok
```

**Commit message:**

```
feat(pipeline): expose run_analyze_incremental on BuildPipeline

Adds BuildPipeline::run_analyze_incremental(&mut self, nodes), which
lazily constructs and then reuses an IncrementalAnalyzer so the
Tarjan-SCC cache survives across calls. Output is bit-identical to
run_analyze (the GraphAnalyzer path).

Also adds:
* incremental_bailout_count() — read-only metric accessor.
* incremental_tarjan_calls()  — read-only metric accessor.

Note: this commit does NOT wire the new method into DevServer's
notify watcher. The dev server currently does not re-run analysis on
file change (it only emits SSE reload events); connecting the watcher
to run_analyze_incremental is the next milestone and lives behind a
small cloudpack-pipeline PR rather than this graph-layer change.

Integration test:
  cargo test -p cloudpack-pipeline --test pipeline_incremental
```

---

## Final verification checklist

Before opening the PR, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test  --workspace
```

All three must exit `0`. In particular confirm:

* `cargo test -p cloudpack-graph --test incremental_v6` — **the merge gate** — is `ok`.
* `cargo test -p cloudpack-graph` total count is `54 passed` (47 baseline + 2 helper + 5 incremental).
* `cargo test -p cloudpack-pipeline --test pipeline_incremental` is `ok`.
* `cargo test -p cloudpack-cli` is unchanged (no failures introduced).

LoC budget audit (informational — keep us honest against the design doc's
"~100 LoC" target):

| File                                          | New / Changed LoC |
|-----------------------------------------------|-------------------|
| `cloudpack-graph/src/reachability.rs`           | ~35 (helper + refactor) |
| `cloudpack-graph/src/analyzer.rs`               | ~40 (extract, no net new logic) |
| `cloudpack-graph/src/incremental.rs`            | ~105 (struct + analyze + tests) |
| `cloudpack-graph/src/lib.rs`                    | +2 |
| `cloudpack-pipeline/src/pipeline.rs`            | ~30 |
| `cloudpack-pipeline/src/lib.rs`                 | +1 |
| **Implementation total (excl. tests)**        | **~130** |
| `cloudpack-graph/tests/incremental_v6.rs`       | ~180 (test infrastructure) |
| `cloudpack-pipeline/tests/pipeline_incremental.rs` | ~85 |

Implementation is within the design doc's ballpark ("~100 LoC, ~30 % win").
The bulk of the diff is test infrastructure — exactly as P3 SCA's
discipline-building goal predicts.
