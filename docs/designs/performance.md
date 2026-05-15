# Performance Subsystem — System Design

| | |
|---|---|
| Status | Design (evidence-gated implementation) |
| Author | systems-design:systems-architect |
| Depends on | versioned-runtime-control.md (SCA), scale-benchmark-foundation.md (V5), observability.md (build-stats + telemetry) |
| Last reviewed | 2026-05-15 |

> **The COE constraint that frames this entire document:**
>
> *"Evidence-gated scale work. Incremental graph analysis and parallel PGO ingestion are triggered when measured workloads exceed current thresholds — not before. Do not run performance victory laps on a synthetic repo until graph-similarity passes."*

---

## ANALYZE — System Map

### Goals

| | Goal | Priority | Gate |
|---|---|---|---|
| G1 | `wundler dev` cold start does not re-bundle `node_modules` when `package.json` is unchanged | P1 | none — ships standalone |
| G2 | A single-module edit in dev mode updates the browser without losing component state | P2 | P1 stable |
| G3 | A single-file change triggers a graph re-analysis whose cost is proportional to the affected subgraph, not the whole graph | P3 | observability data + V5 |
| G4 | PGO ingestion of multi-GB telemetry completes in time proportional to (cores × file size / sequential rate) | P4 | observability data |
| G5 | `wundler build --analyze` produces a navigable view of chunks, dead modules, and size hotspots | P5 | build-stats.json (observability) |
| G6 | VS Code surfaces existing CLI diagnostics inline (no second model) | P6 | CLI diagnostics stable |
| G7 | Rspack adapter exists when, and only when, a named user requires it | P7 | deferred |

### Constraints

- **Rust everywhere on the server side.** No new runtimes for the dev server, the watcher, the dep pre-bundler, or the HMR coordinator.
- **JavaScript only where the browser demands it.** react-refresh runtime, HMR client glue, SW message handling.
- **Correctness over speed for graph work.** Incremental output must be bit-identical to full recompute output (manifest equality, chunk membership equality, dead-export set equality). This is testable and is a hard CI gate.
- **No performance claims without measurement from the Scale Benchmark Foundation.** V5 (graph-shape similarity) must pass on the synthetic corpus before any P3 work is presented as evidence of real-world wins.
- **PGO ingestion must be transactionally safe.** No lost events on crash, no duplicate inserts on restart. The current `INSERT OR IGNORE` idempotence (`store.rs:88`) is the floor any parallel design must preserve.

### Actors

| Actor | Calls into | Reads / writes |
|---|---|---|
| Developer | `wundler dev`, `wundler build --analyze`, VS Code | source files, `wundler.toml`, build-stats.json |
| Browser (dev) | SW + ABS | manifest.json, chunks, HMR updates |
| File watcher | `wundler dev` event bus | filesystem events (FSEvents/inotify) |
| PGO ingestor | SQLite store | JSONL telemetry log, `pgo.sqlite` |
| CI | `wundler bench`, build budget gate | build-stats.json |

### Interfaces (the boundaries this design lives at)

| Boundary | Today | After this design |
|---|---|---|
| File change → graph re-analysis | `notify` event → `GraphAnalyzer::analyze` (full) | `notify` event → `IncrementalAnalyzer::on_change(paths)` → either full recompute or scoped recompute, always producing the same `AnalysisResult` |
| Dep specifier → bundled chunk | resolve + re-bundle every `wundler dev` start | `DepPrebundler::ensure_fresh()` returns cached or re-bundles iff `package.json + lockfile` hash changed |
| Module update → browser | manifest write → SW reload → `location.reload()` | manifest write → SW posts `module-update` → page applies via react-refresh, falls back to reload on rejection |
| Telemetry JSONL → SQLite | `ingest_log` sequential `for line in reader.lines()` | bounded parallel parse → single writer thread with batched transactions (WAL) |
| build-stats.json → analyzer UI | (does not exist) | static HTML bundle reads `build-stats.json` via the same ABS route the dev server uses for the manifest |
| CLI diagnostics → VS Code | (does not exist) | LSP-style JSON-RPC over stdio; diagnostics emitted by `wundler check` are consumed verbatim |

### Wasteful paths (what runs today that doesn't need to)

| Path | Location | Cost growth | Cure |
|---|---|---|---|
| Full `GraphAnalyzer::analyze` on every file change | `crates/wundler-graph/src/analyzer.rs:89` | O(N + E) per keystroke-saved | P3 incremental |
| Full BFS over all entries every analysis | `crates/wundler-graph/src/reachability.rs:29,86` | O(N + E) | P3 scoped BFS from invalidated frontier |
| `tarjan_sccs` re-run every analysis | `crates/wundler-graph/src/graph.rs` (called from `reachability.rs:98`) | O(N + E) | P3 cache SCCs; recompute only when graph topology changes |
| `assign_chunks` 3-phase re-run on every analysis | `crates/wundler-graph/src/chunks.rs` | O(N · entries) | P3 invalidate per-entry chunk membership, recompute affected chunks |
| `compute_dead_exports` over the entire node set | `crates/wundler-graph/src/dce.rs` | O(N · exports) | P3 limit to alive set of affected subgraph |
| `for line in reader.lines()` + per-line SQLite transaction | `crates/wundler-pgo/src/ingestor.rs:60` + `store.rs:88` | O(L) sequential, lock per line | P4 parallel parse + batched single-writer commit |
| Full re-scan of `node_modules` on every `wundler dev` start | implicit in `run_dev` (`wundler-cli/src/main.rs:396`) | O(M) deps × parse | P1 hash + cache |
| `location.reload()` on every manifest swap | `crates/wundler-sw/src/sw.ts` | full document lifecycle, loses state | P2 selective `module-update` postMessage |

### Feedback loops

| Loop | Behavior |
|---|---|
| Edit → analyze → manifest → SW reload → page load | Today: ~full recompute every edit. P2+P3 turn this into a near-constant-cost loop for one-module edits. |
| Telemetry → PGO ingestion → hints → next build | Sequential ingestion gates how often hints can be recomputed. Larger logs = staler hints. P4 lets ingestion track log growth. |
| build-stats.json → CI budgets → developer | One-way today (built by observability). The analyzer viewer (P5) feeds this *back* to the developer on every build — must avoid becoming a second source of truth (see Risks). |

### Failure modes

| FM | What breaks | Blast radius |
|---|---|---|
| FM-1 | HMR misses an invalidation boundary; browser shows stale state with no error | One session; silent. The most dangerous failure in the document. |
| FM-2 | Incremental graph produces a different `ChunkManifest` than full recompute | All clients; layout instability across the bundle; chunk identity drift |
| FM-3 | Parallel PGO ingestion loses events on crash, or double-inserts on restart | Hint quality silently degrades; idempotence guarantee broken |
| FM-4 | Dep pre-bundle cache grows without bound | Disk fills; dev server fails on cold start with cryptic error |
| FM-5 | Bundle analysis viewer reports different chunk sizes than `build-stats.json` | Developer reports a "bug" when the build is fine; two sources of truth |
| FM-6 | VS Code extension invents its own diagnostic model | Drift between CLI and editor; explicit COE prohibition violated |
| FM-7 | P3 lands without P3 gate evidence; "looks faster" is the only justification | Subtle correctness regressions sneak in under the banner of perf wins |

### The three categories (the cleanest split)

| Category | Items | Characteristic |
|---|---|---|
| **Dev-loop performance** | P1 (dep pre-bundling), P2 (HMR) | Felt at every keystroke. Wins are obvious to one developer with no benchmark. Ships standalone. |
| **Scale performance** | P3 (incremental graph), P4 (parallel PGO) | Wins only become visible above a threshold. Requires measurement. Risks correctness if rushed. |
| **DX tooling** | P5 (analyzer), P6 (VS Code) | Not in the hot path. Quality-of-life. Gated on stable contracts from other subsystems. |
| **Deferred** | P7 (Rspack) | Explicitly out of scope until a named user. |

### What "incremental" means in a graph with SCCs (the hard part)

The graph has three layers of state. Each can be made incremental in different ways, and they interact:

| Layer | Today | Incrementally |
|---|---|---|
| **Topology** (nodes + edges) | rebuilt from `BundleGraphNode` slice every analyze | invalidate per-file: a file change rewrites at most one node's outgoing edges. The node hash itself changes (content-hashing). |
| **SCC membership** | `tarjan_sccs` every analyze (`graph.rs`, called from `reachability.rs:98`) | invalidate when edges change. SCC membership can shift across non-adjacent nodes — see deep dive below. |
| **Reachability** | full BFS from entries every analyze (`reachability.rs:29,86`) | only the subgraph downstream-of-changes needs to be re-explored *from the affected frontier*; nothing else changes. |
| **Chunk assignment** | 3-phase recompute (`chunks.rs:assign_chunks`) | only chunks containing an invalidated module need to be reassembled — see "Can chunking be made incremental?" below. |

The key insight: **the four layers can be made incremental independently and with different difficulty.** Reachability is the easiest, chunking is harder, SCC handling is the trickiest, and DCE is in between. A correct incremental design treats them as a pipeline of cache levels, with explicit invariants between each level. Details in the deep dive section.

---

## DESIGN

### Priority 1 — Dependency pre-bundling

**Goal:** `wundler dev` cold start does not parse / re-bundle `node_modules` when `package.json + lockfile` hash has not changed.

**Where it lives:** New module `crates/wundler-dev/src/prebundle/`, called from `wundler-cli/src/main.rs::run_dev` (currently a stub) **before** the file watcher starts.

#### Candidates

| | C1 — current | **C2 — fingerprint cache (RECOMMENDED)** | C3 — fine-grained per-package cache |
|---|---|---|---|
| Trigger | always rebundle | rebundle iff `H(package.json ‖ lockfile)` differs | per-package metadata + content hash, rebundle only the changed packages |
| State on disk | none | `.wundler/cache/deps/{fingerprint}/` — one dir per fingerprint, contains pre-bundled chunks + `index.json` | `.wundler/cache/deps/{package}@{version}/{content-hash}/` — many small caches |
| Cold start (cached) | full re-bundle | filesystem read + hash check (~50ms) | filesystem walk + per-package hash check (~200ms) |
| Cold start (cache miss) | full re-bundle | full re-bundle, write to new fingerprint dir | only changed packages rebundled |
| GC policy | n/a | LRU prune on dirs older than `[dev.dep_cache_ttl_days]` (default 14) | LRU; harder because content hashes shared across versions |
| Code added | 0 | ~150 LoC | ~400 LoC + invalidation tracker |
| Failure mode | slow but obvious | stale cache survives a rename of `package.json` (we hash it — fine) | invalidation bugs hide (FM-4) |
| YAGNI | — | meets goal | over-engineered for a problem we don't have evidence of |

**8-dimension tradeoff (C2 vs current):**

| Dim | C2 |
|---|---|
| Simplicity | +1 (one hash, one cache dir, atomic swap) |
| Performance | +2 (skips re-bundle entirely on cached path) |
| Operability | +1 (`.wundler/cache/deps/` is human-inspectable) |
| Cost | ~0 (disk only; bounded by TTL) |
| Reliability | +1 (cache miss falls back to current behavior — no regression possible) |
| Security | 0 (cache contents are derived deterministically from `node_modules`; untrusted input was already there) |
| Evolution | +1 (clear upgrade path to C3 if measurement demands) |
| Blast radius | local to one developer's `.wundler` |

**Recommendation: C2.** It is the simplest design that meets the goal and offers no regression path. C3 is the right answer only if a measurement proves dep changes are frequent enough that per-package invalidation matters more than the cold-start ~50ms cache check.

#### Concrete shape — C2

```rust
// crates/wundler-dev/src/prebundle/mod.rs
pub struct DepPrebundler {
    cache_root: PathBuf,      // <project>/.wundler/cache/deps
    ttl_days: u32,
}

pub struct PrebundleResult {
    pub cache_dir: PathBuf,   // contains chunks + index.json
    pub from_cache: bool,
    pub fingerprint: String,  // hex of H(package.json || lockfile)
}

impl DepPrebundler {
    pub fn ensure_fresh(&self, project_root: &Path) -> Result<PrebundleResult> {
        let fp = compute_fingerprint(project_root)?;
        let target = self.cache_root.join(&fp);
        if target.join("index.json").exists() {
            self.touch(&target)?;  // for LRU
            return Ok(PrebundleResult { cache_dir: target, from_cache: true, fingerprint: fp });
        }
        // cache miss: bundle into a tempdir, atomic rename into place
        let tmp = self.cache_root.join(format!(".tmp-{fp}"));
        self.bundle_into(project_root, &tmp)?;
        std::fs::rename(&tmp, &target)?;
        self.gc()?;  // best-effort, never blocks
        Ok(PrebundleResult { cache_dir: target, from_cache: false, fingerprint: fp })
    }
}

fn compute_fingerprint(root: &Path) -> Result<String> {
    let mut h = blake3::Hasher::new();
    h.update(&std::fs::read(root.join("package.json"))?);
    if let Ok(lock) = std::fs::read(root.join("package-lock.json")) { h.update(&lock); }
    else if let Ok(lock) = std::fs::read(root.join("pnpm-lock.yaml")) { h.update(&lock); }
    // ... yarn.lock, bun.lockb
    Ok(h.finalize().to_hex().to_string())
}
```

**GC: bounded.** On every cache miss, walk `cache_root`, delete dirs whose `mtime > ttl_days` ago. Best-effort; never blocks the dev server. This is the answer to FM-4.

#### Simplest Credible Alternative (P1)

A 30-line variant of C2 that hashes only `package.json` (skip the lockfile), uses a single `.wundler/cache/deps/{hash}/` directory, no GC. The lack of lockfile hashing means a `npm install` of a different version of the same range slips through. The lack of GC means disk grows monotonically. Acceptable for a POC.

---

### Priority 2 — True HMR

**Goal:** A single-module edit in `wundler dev` updates the browser in place; component state is preserved across the update.

**Where it lives:**
- Rust: new module `crates/wundler-dev/src/hmr/`, plus ABS route `POST /reload` (already designed in VRC).
- TypeScript: HMR client glue in `crates/wundler-sw/src/hmr.ts`; react-refresh runtime injected by the transform pipeline.

#### Candidates

| | C1 — current | **C2 — react-refresh + module-update msg (RECOMMENDED)** | C3 — CRDT-preserved state across complex updates |
|---|---|---|---|
| Update mechanism | `location.reload()` | SW posts `{type:"module-update", build_id, modules:[...]}`; page-side HMR runtime applies via react-refresh | C2 + CRDT-merged props/state across non-refreshable boundaries |
| State preservation | none | component state preserved for react-refresh-compatible boundaries; falls back to reload if rejected | aspires to preserve state across more boundaries |
| Invalidation boundary detection | n/a | module-level: any module that exports a non-component or has side effects is a "reload boundary" | same as C2 |
| SCC handling | n/a | a change inside an SCC invalidates the whole SCC; if any SCC member is a reload boundary → full reload | same |
| New dependencies | 0 | `react-refresh` (npm); no new Rust deps | `yjs` or similar; large new surface |
| Code added | 0 | ~300 LoC Rust + ~400 LoC TS | C2 + likely 2000+ LoC |
| Failure mode | slow but obvious | FM-1: stale state if invalidation misses a boundary | FM-1 amplified — CRDT bugs are silent and weird |
| YAGNI | — | meets goal | clearly overkill for the user we have today |

**Recommendation: C2.** State preservation is the user-visible value of HMR; react-refresh handles it for the dominant case (React components). For everything else, fall back to reload — which is the current behavior, so it cannot regress.

#### Invalidation algorithm (C2)

```
on file change F:
    new_node = scan_file(F)              // re-derive content hash, imports, exports
    if new_node.id == old_node.id:
        return                            // no observable change

    affected = {new_node}
    if changed_set_in_scc(new_node):
        affected |= scc_members(new_node) // SCC invalidates as a unit

    // boundary test (run on every affected module):
    let mode = if all(is_refresh_boundary(m) for m in affected) { Update }
               else                                              { Reload };

    write_manifest()                      // VRC: atomic, build_id bumped
    abs.broadcast(BuildEvent {
        build_id,
        mode,                              // Update | Reload
        modules: affected.iter().map(|m| m.url).collect(),
    });
```

`is_refresh_boundary(m)` is true iff every export of `m` is a React component (heuristic: exported identifier starts with uppercase, body contains JSX) OR `m` declares `module.hot.accept()`. Conservative: any doubt → reload boundary.

#### What HMR does NOT solve (write these down before they get asked)

- **Non-React frameworks** — out of scope for v1; reload only.
- **Side-effect modules** (CSS imports, polyfills) — always reload boundary in v1.
- **Cross-realm state** (IndexedDB, Service Worker globals) — never touched by HMR; survives reload too.
- **Type-only changes** — still trigger HMR because we cannot distinguish them without TS-aware analysis (out of scope for v1).

#### Concrete shape — C2

```rust
// crates/wundler-dev/src/hmr/mod.rs
pub struct HmrCoordinator {
    abs_tx: tokio::sync::broadcast::Sender<BuildEvent>,
}

#[derive(serde::Serialize, Clone)]
pub struct BuildEvent {
    pub build_id: String,
    pub mode: UpdateMode,            // Update | Reload
    pub modules: Vec<String>,        // chunk URLs to refetch
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode { Update, Reload }
```

ABS gains `GET /hmr/events` (SSE) — the SW subscribes once per page session, relays to the page via `postMessage`. SSE is chosen over WebSocket because the dev server only sends one direction; SSE has half the moving parts.

#### Simplest Credible Alternative (P2)

Keep `location.reload()`, but make it a *targeted* reload: SW receives `{ build_id, mode: "reload" }` and uses `clients.matchAll()` → `client.navigate(client.url)` instead of relying on the page polling. Removes the polling cost and unifies the reload mechanism with the manifest swap pipeline. No react-refresh, no state preservation, but the dev-loop latency improves on its own. About 80 LoC.

---

### Priority 3 — Incremental graph analysis (EVIDENCE-GATED)

**Goal:** `IncrementalAnalyzer::on_change(paths)` produces an `AnalysisResult` whose cost is proportional to the affected subgraph, not to the whole graph. The produced result MUST be bit-identical to a fresh `GraphAnalyzer::analyze(...)` of the same inputs.

**Gate (mandatory — see GATE CONDITIONS section):** must not start until observability shows real-corpus analyze times above the threshold AND Scale Benchmark Foundation V5 (graph-shape similarity) passes.

#### Candidates

| | C1 — current | **C2 — scoped recompute (RECOMMENDED, EVIDENCE-GATED)** | C3 — C2 + persistent cache across restarts |
|---|---|---|---|
| On file change | full `GraphAnalyzer::analyze` | invalidate affected node + its SCC; scoped reachability from frontier; scoped DCE; reuse all unaffected chunks | C2 + serialize `GraphCache` to `.wundler/cache/graph/{build_id}.bin` |
| First-build cost | O(N+E) | O(N+E) (same) | O(N+E) (same) |
| Edit-build cost | O(N+E) | O(K+E_K) where K = invalidated set | O(K+E_K) |
| Cold dev-server restart | O(N+E) | O(N+E) | O(N+E) for first edit only; deserialize cost ≈ 5% of analyze |
| Code added | 0 | ~600 LoC across `wundler-graph/` | C2 + ~250 LoC serde + version-pinned cache |
| Failure mode | slow but obvious | FM-2: divergence from full recompute | FM-2 + cache corruption on bincode version skew |
| YAGNI | — | gated on measurement | gated harder — need to also prove restart latency matters |

**Recommendation: C2.** C3 is correct in spirit but the cache invalidation rules (when does `GraphCache` become invalid? on toolchain bump? on transform-rule change?) introduce a second invalidation problem on top of the first. Land C2 first, measure restart latency, decide.

#### Architecture — C2

A new struct `IncrementalAnalyzer` in `crates/wundler-graph/src/incremental.rs` holds four caches mirroring the four layers in the ANALYZE section:

```rust
pub struct IncrementalAnalyzer {
    // Layer 1 — topology
    nodes_by_path: HashMap<PathBuf, ContentHash>,
    nodes: HashMap<ContentHash, BundleGraphNode>,
    adj: HashMap<ContentHash, Vec<ContentHash>>,
    rev_adj: HashMap<ContentHash, Vec<ContentHash>>,  // NEW: for upstream invalidation

    // Layer 2 — SCC membership
    sccs: Vec<Vec<ContentHash>>,
    scc_of: HashMap<ContentHash, usize>,
    scc_dag_adj: HashMap<usize, Vec<usize>>,         // condensation graph

    // Layer 3 — reachability
    alive: HashSet<ContentHash>,
    alive_scc: HashSet<usize>,

    // Layer 4 — chunking
    last_manifest: ChunkManifest,
    module_to_chunk: HashMap<ContentHash, String>,
}
```

`on_change(paths: &[PathBuf]) -> AnalysisResult` runs the following pipeline. Each stage caches outputs and shrinks the input to the next stage:

```
1. Rescan changed files → derive new BundleGraphNodes.
   For each path P:
     old_id = nodes_by_path[P]
     new_node = scan(P)
     if new_node.id == old_id: skip (no-op)
     dirty_nodes += { old_id, new_node.id }
     update nodes_by_path[P] = new_node.id
     update adj for new_node; rev_adj likewise

2. Topology delta → SCC invalidation.
   Affected SCCs = { scc_of[h] for h in dirty_nodes if h was in graph before }
   If any edge crossed an SCC boundary (added or removed):
     Recompute SCCs locally — see "SCC INVALIDATION SCOPE" deep dive below
     // Worst-case fallback: re-run tarjan_sccs() on the full graph and bail out
     // of the incremental path for this analyze. Bailout MUST be logged and counted.

3. Reachability delta.
   For each newly-dead-candidate SCC s in affected_sccs:
     If no path from any entry_scc to s in scc_dag_adj: s drops out of alive_scc
   For each newly-reachable SCC s (a new edge created the only path):
     BFS from s downstream, adding any not-yet-alive SCCs

4. DCE delta.
   For each export changed in dirty_nodes:
     Recompute dead-set ONLY for that module (compute_dead_exports already
     operates per-module, just feed it the dirty set)

5. Chunk delta.
   For each chunk c in last_manifest.chunks:
     If c.members ∩ dirty_nodes == empty AND c.entry not affected: REUSE
     Else: recompute c via the existing assign_chunks logic, restricted to c's seed set
   Commons chunk recomputed iff appearance counts changed for any module in commons_set.

6. Manifest assembly.
   build_manifest(reused + recomputed chunks). build_id derived deterministically
   (VRC's compute_build_id) — if no observable change, build_id is byte-identical to
   the previous one and ABS short-circuits broadcast.
```

#### Correctness invariant (THE invariant)

```
∀ valid graph states G:
    IncrementalAnalyzer::on_change(diff(G_prev, G)).serialize()
    ==
    GraphAnalyzer::analyze(G).serialize()
```

This is **mechanically checked** by Scale Benchmark Foundation's V4 (validity) and a new V6-incremental:

> **V6:** for a generated repo and a generated edit sequence E of length K, the cumulative `IncrementalAnalyzer` state after replaying E must equal `GraphAnalyzer::analyze(G_K)` for the final graph G_K. Tested with K ∈ {1, 10, 100, 1000}.

Any divergence is a P0 bug. **No measurement claim about P3 is admissible until V6 passes on at least one real corpus.**

#### Bailout policy

When the incremental code path detects a state it cannot handle (e.g., a topological change that affects too many SCCs to be cheaper than full recompute), it falls back to `GraphAnalyzer::analyze` and increments a `incremental_bailout_total` counter (Prometheus, via observability). A bailout is not a failure; an *uncounted* bailout is. Threshold-based alerts on bailout rate are a Phase 2 follow-up.

#### Simplest Credible Alternative (P3)

The simplest non-trivial improvement: cache `tarjan_sccs` output across analyses, invalidate only when an edge is added or removed. Reachability and chunking remain full recompute. This is ~100 LoC and gives roughly 30% of the win (SCC computation dominates on dense graphs). It also exercises the cache-invalidation discipline that full P3 will demand. **This is what we should ship first under the P3 banner**, then layer on reachability and chunking deltas iff measurement still demands it.

---

### Priority 4 — Parallel PGO ingestion (EVIDENCE-GATED)

**Goal:** Ingest multi-GB JSONL telemetry logs in time proportional to (cores × size / sequential rate), with exactly the same idempotence guarantees as today.

**Gate:** observability data must show ingestion time exceeding a threshold (see GATE CONDITIONS).

#### Candidates

| | C1 — current | **C2 — parallel parse, single writer (RECOMMENDED)** | C3 — parallel parse, sharded writers |
|---|---|---|---|
| Parse | sequential | `rayon` over a memory-mapped file split on `\n` boundaries | same |
| Writer | per-line transaction, single connection (`store.rs:88`) | one writer thread, batched transactions of size B=1000, WAL mode | N writer threads, each owning a SQLite shard |
| Idempotence | `INSERT OR IGNORE` per row | same `INSERT OR IGNORE`, just in batches | per-shard idempotence; cross-shard correctness requires careful key partitioning |
| Crash safety | WAL-less; partial line on crash possible | WAL + `synchronous=NORMAL`; truncates last incomplete batch | per-shard WAL; recovery is N × the work |
| Locking risk | none (single thread) | none (single writer) | high (SQLite write lock contention is famously worse than serial under WAL) |
| Code added | 0 | ~250 LoC | ~600 LoC + shard-routing logic |
| Failure mode | slow but obvious | a parser panic loses the in-flight batch (≤ B rows); recoverable on next run via `INSERT OR IGNORE` | FM-3 amplified; cross-shard partial commit creates non-deterministic state |
| YAGNI | — | gated | clearly the wrong answer for SQLite (see SQLite docs §How to corrupt) |

**Recommendation: C2.** SQLite under WAL with a single writer is the canonical high-throughput pattern. Parallel readers/parsers fill the writer's queue; the writer commits batches in a tight loop. This is what `litestream`, `rqlite`, and friends do.

#### Architecture — C2

```rust
// crates/wundler-pgo/src/ingestor.rs (replacement)
pub fn ingest_log(store: &PgoStore, log_path: &Path) -> Result<IngestStats> {
    let mmap = unsafe { memmap2::Mmap::map(&std::fs::File::open(log_path)?)? };
    let (tx, rx) = crossbeam_channel::bounded::<SessionRecord>(BATCH_SIZE * 4);

    // Parser pool — rayon
    let stats = std::thread::scope(|s| {
        let writer = s.spawn(|| writer_loop(store, rx));
        rayon_parse(&mmap, tx);   // drops tx on completion
        writer.join().unwrap()
    })?;
    Ok(stats)
}

fn writer_loop(store: &PgoStore, rx: Receiver<SessionRecord>) -> Result<IngestStats> {
    let mut conn = store.conn.lock().unwrap();
    let mut stats = IngestStats::default();
    let mut batch: Vec<SessionRecord> = Vec::with_capacity(BATCH_SIZE);
    while let Ok(rec) = rx.recv() {
        batch.push(rec);
        if batch.len() >= BATCH_SIZE { flush(&mut conn, &mut batch, &mut stats)?; }
    }
    if !batch.is_empty() { flush(&mut conn, &mut batch, &mut stats)?; }
    Ok(stats)
}
```

#### WAL mode (new, required)

In `store.rs::init`:

```rust
conn.execute_batch("
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA temp_store = MEMORY;
    PRAGMA cache_size = -64000;    -- 64 MB
")?;
```

This is the boring, correct configuration. `synchronous=NORMAL` under WAL is durable-on-checkpoint; we accept losing at most the last checkpoint's worth of rows on power loss — same idempotence-on-restart guarantee re-runs the file and recovers via `INSERT OR IGNORE`.

#### Correctness contract

| Property | Today | After C2 |
|---|---|---|
| Idempotence (re-running same file) | ✅ `INSERT OR IGNORE` per row | ✅ same — batches contain the same rows |
| Crash safety (no partial sessions) | ⚠ partial line on power loss possible | ✅ WAL truncates incomplete batches |
| Determinism (output = same DB state) | ✅ | ✅ — order within a session is preserved (`load_order` field is set by the parser, not by insertion order) |
| No duplicate inserts on parallel restart | ✅ | ✅ via `INSERT OR IGNORE` |

**No lost events, no duplicate inserts.** The COE constraint is preserved.

#### Simplest Credible Alternative (P4)

Keep the sequential parser, but: (a) enable WAL mode in `store.rs::init` (5 lines), (b) replace per-line transactions with one transaction per 1000 lines (15 lines). This typically gives a 10–100× speedup on its own — most of the parallel-ingestion win is actually batched-commit win, not parallelism win. Measure this first; the rayon parser may turn out to be unnecessary entirely. **This is also a strong candidate for what to ship under the P4 banner if measurement doesn't justify the full design.**

---

### Priority 5 — Bundle analysis viewer

**Goal:** `wundler build --analyze` opens a treemap of modules, chunks, and dead code.

**Where it lives:**
- Input: `build-stats.json` (from observability.md, P1).
- Renderer: a static HTML/JS bundle in `crates/wundler-cli/assets/analyzer/` served by an ephemeral one-shot HTTP server on `127.0.0.1:0`. Same shape as `cargo flamegraph` / `webpack-bundle-analyzer`.

**Why one design, not three:** the data already exists and the rendering is a solved problem. Reuse `d3-treemap` or `react-window-treemap`; do not invent a visualization layer.

**Surface:**
- Treemap of chunks → modules → bytes (raw, gzip, brotli)
- "Dead modules" panel reading `build-stats.json::dead_modules`
- "Largest 20 modules" sorted view
- Per-chunk metadata: `id`, `load_condition`, `route` (initial chunks)
- Search filter (by path substring)

**What it must NOT do (FM-5):**
- Recompute any sizes — render only what `build-stats.json` reports
- Run any transforms or rebundle
- Become writable in any way

The viewer is a read-only projection of build-stats.json. If a developer reports "the analyzer says X but the build is Y", the answer is "the analyzer is wrong; build-stats.json is authoritative" — and the fix is in the viewer, never anywhere else.

#### Simplest Credible Alternative (P5)

`wundler build --analyze` prints a markdown table to stdout: top-N chunks by size, top-N modules by size, count of dead modules. ~50 LoC. Surfaces the same insights without any frontend infrastructure. The treemap is a refinement.

---

### Priority 6 — VS Code extension

**Goal:** Surface existing `wundler` diagnostics inline in the editor.

**Where it lives:** A new repo / npm package `wundler-vscode`, talking to a `wundler lsp` subcommand over stdio.

**Gate (mandatory):** CLI diagnostics must be **stable and structured** first. "Stable" means:
- JSON-RPC schema versioned and committed in `crates/wundler-cli/src/diagnostics/schema.json`
- Each diagnostic has: `code` (e.g. `WUN-001`), `severity`, `range`, `source`, `message`
- Schema follows LSP `Diagnostic` shape verbatim so VS Code consumes it with `vscode.languages.createDiagnosticCollection`

**What it must NOT do (FM-6, COE prohibition):**
- Invent a second diagnostic model
- Surface diagnostics the CLI does not also surface
- Re-implement reachability or DCE analysis in TypeScript

The extension is a transport, not a logic component. If a diagnostic appears in VS Code, the same diagnostic must appear when running `wundler check` on the command line.

#### Architecture

```
VS Code ─── LSP/JSON-RPC ───▶ `wundler lsp` (stdio child process)
                                    │
                                    ▼
                               IncrementalAnalyzer + DCE
                                    │
                                    ▼
                              diagnostics → push to VS Code
```

**Notice:** `wundler lsp` reuses `IncrementalAnalyzer` (P3). The editor experience benefits from P3 directly — keystroke → diagnostic update — and P3 thereby acquires a second consumer beyond the dev server. This is a strong argument that P3 and P6 should be designed together even if implemented separately.

#### Simplest Credible Alternative (P6)

A "problems matcher" registered in `tasks.json` that parses `wundler check`'s stdout (one diagnostic per line in a regex-friendly format). No LSP server, no incremental analysis, no extension. Surfaces diagnostics in the Problems panel. ~50 lines of JSON. **Recommended starting point** — full LSP only when this proves inadequate.

---

### Priority 7 — Rspack adapter (DEFERRED)

**Goal:** none until a named user requires it.

**Design output:** explicitly none. This entry exists so that future work knows to point at this section, read the deferral, and stop.

If a named user emerges, the design must answer:
1. What can Rspack do that `RolldownAdapter` cannot?
2. Does the requirement justify a second adapter, or does it justify extending the abstraction at the adapter layer (`crates/wundler-pipeline/src/adapter.rs`)?
3. What is the maintenance cost projection (security patches, version pinning, transform-rule drift)?

Until those questions have concrete answers tied to a real user, no work happens. The `RolldownAdapter` already validates that the adapter abstraction is sufficient; a second adapter is only valuable as a proof.

---

## GATE CONDITIONS — precise, measurable

These are the thresholds at which evidence-gated work is permitted to begin. They are written in terms of metrics emitted by the Observability design.

### P3 — incremental graph analysis

**Permitted to begin when ALL of:**

| Condition | Threshold | Metric source |
|---|---|---|
| 1. Real-corpus analyze p95 | `analyze_duration_seconds{phase="analyze"}` p95 > **5 s** on a corpus ≥ 5k modules | observability P3 (Prometheus) |
| 2. Dev-loop edit frequency | At least one team reports ≥ 50 edits/hour on the gated corpus | manual via observability dashboard |
| 3. V5 graph-similarity | Scale Benchmark Foundation V5 passes on at least one preset (uniform.v1 OR a real corpus) | `wundler bench validate-scale` exit code |
| 4. Bailout instrumentation | `incremental_bailout_total` Prometheus counter exists (Phase 1 instrumentation) | observability extension |

Why these numbers: under 5s, the optimization is invisible to a human; over 5s, it dominates the edit loop. 50 edits/hour is the rate at which the total daily cost crosses ~5 minutes. V5 protects against optimizing for the wrong workload.

### P4 — parallel PGO ingestion

**Permitted to begin when ALL of:**

| Condition | Threshold | Metric source |
|---|---|---|
| 1. Real ingestion duration | A telemetry log ≥ **1 GB** has been ingested AND wall time > **30 s** | `wundler pgo ingest` self-report |
| 2. Simplest-credible-alt landed first | WAL mode + batched-commit alternative is in `main`, and a follow-up measurement still exceeds the 30s threshold | git log + measurement |
| 3. Idempotence test in CI | A "re-ingest same file produces identical row count" test passes | `cargo test -p wundler-pgo` |

The simplest-credible-alt clause is deliberate: most of the win is in batching, not parallelism. We measure, then decide.

### P6 — VS Code extension

**Permitted to begin when ALL of:**

| Condition | Threshold | Metric source |
|---|---|---|
| 1. Diagnostic schema versioned | `crates/wundler-cli/src/diagnostics/schema.json` exists; `version` field ≥ 1.0.0 | repo artifact |
| 2. Diagnostic schema stable | No breaking schema changes for ≥ 30 days | git log |
| 3. CLI diagnostics count | `wundler check` emits ≥ 5 distinct diagnostic codes against the dogfood corpus | manual |

---

## INCREMENTAL GRAPH ANALYSIS — DEEP DIVE

### Why SCCs are the hard part

In a DAG, a node change has a clean downstream-only blast radius. In a graph with cycles, a node belongs to an SCC; the SCC behaves as a single unit for liveness ("if any member is alive, all members are alive" — already documented in `reachability.rs:80`).

**SCC membership is not monotone under edits.** Adding a single edge can:

1. **Merge SCCs.** Two previously-separate SCCs A and B, where A → B existed but B → A did not. Add a new edge `b → a` for `a ∈ A, b ∈ B`: A and B collapse into one larger SCC. All members of the new SCC are now mutually-reachable, which has implications for liveness (no change — both were already alive) but **major** implications for chunking: the commons heuristic may now place modules in different chunks.

2. **Split SCCs.** Removing an edge can split an SCC into two smaller SCCs. Detecting this without re-running tarjan from scratch is non-trivial; the literature has incremental SCC algorithms (Bender-Fineman-Gilbert-Tarjan 2015), but their constants are not always favorable for small edit sets.

The honest answer: **do not invent an incremental SCC algorithm.** Use the following decision tree:

```
edits affect ≤ K nodes, edges added/removed all inside a single existing SCC?
  → SCC membership unchanged; reuse cache. Reachability unchanged.
edges added that cross between two SCCs?
  → Recompute SCCs on the closure(affected_sccs ∪ direct_predecessors ∪ direct_successors)
  → If the result is a single merged SCC: update scc_of for those members; mark chunks
    containing the merged set dirty.
edges removed from inside an existing SCC?
  → BAILOUT: re-run tarjan_sccs on the full graph. Log incremental_bailout_total{reason="scc_split"}.
```

Bailout is acceptable. Bailout that doesn't get counted is not.

### Can chunking be made incremental?

The chunking algorithm (`crates/wundler-graph/src/chunks.rs`) is a 3-phase pipeline:

1. **Phase 1 — Collect candidates** (one BFS per entry, dynamic imports captured)
2. **Phase 2 — Count appearances** (membership counting across candidates)
3. **Phase 3 — Emit chunks** (commons threshold applied, commons emitted first)

**Phase 1 is incrementalizable per-entry.** Each entry-route's BFS is independent. If module M with chunk_id C is invalidated, only the candidates that contained M need to be recomputed — and only if M's reachability or imports actually changed.

**Phase 2 is the hard part.** The appearance count of an unmodified module N changes whenever a candidate that contained N is added or removed from the candidate set. This means a change in entry E's BFS can shift the commons-set classification of modules that have no other relationship to E.

**Phase 3 follows Phase 2.** If commons-set membership changed for any module, all chunks must be re-emitted with the new commons filter applied.

**Practical implication:** Phase 1 yields most of the benefit when edits are localized within a single entry's reachable set. Phase 2/3 incrementality is bounded by how often commons-set membership shifts — likely rare for non-trivial corpora, common for synthetic uniform graphs (which is one more reason V5 graph-similarity matters).

**Recommendation for P3 v1:**
- Phase 1: per-entry BFS cache; invalidate per-entry on changed reachability
- Phase 2: full re-count (cheap given the cached Phase 1 outputs)
- Phase 3: full re-emit (cheap)

The full Phase 2/3 incrementality is a Phase 2 follow-up if measurement justifies it.

### The correctness invariant — again, in stronger form

```
∀ G, ∀ valid edit sequence E of length K, ∀ starting cache state C0:
    let C_K = fold(E, C0, IncrementalAnalyzer::on_change)
    C_K.last_manifest.canonical_bytes() == GraphAnalyzer::analyze(G_K).manifest.canonical_bytes()
```

Where `canonical_bytes()` is the deterministic serialization defined by VRC. This is a property test; it must be in CI before any P3 code is merged. The test catches FM-2 and FM-7 simultaneously.

---

## RISKS

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-1 (FM-1) | HMR misses an invalidation boundary; browser shows stale state silently | Medium | High (user trust) | Conservative `is_refresh_boundary` (default: not a boundary unless proven); fall back to reload on any doubt; instrumented mismatch counter |
| R-2 (FM-2) | Incremental graph produces different `ChunkManifest` than full recompute | Medium | High (layout instability, user trust) | V6 property test in CI (mandatory before merge); explicit bailout path with counter; `--no-incremental` escape hatch |
| R-3 (FM-3) | Parallel PGO ingestion duplicates events on crash-restart | Low | Medium | `INSERT OR IGNORE` invariant preserved by C2; idempotence test in CI; WAL mode tested under simulated crash |
| R-4 (FM-4) | Dep pre-bundle cache grows unbounded | Medium | Low | LRU GC on every cache miss; `wundler dev` warns when cache > 1 GB |
| R-5 (FM-5) | Bundle viewer becomes second source of truth | Medium | Medium | Viewer is strictly read-only over `build-stats.json`; no recomputation; if mismatch reported, fix is always in the viewer |
| R-6 (FM-6) | VS Code extension invents a second diagnostic model | Medium | High (architectural drift, COE explicit prohibition) | Schema-first contract; CI test that `wundler lsp` and `wundler check` produce equivalent diagnostics on a corpus |
| R-7 (FM-7) | P3 lands without gate evidence | Medium | High (correctness regressions, false perf claims) | Gate conditions are encoded as PR-blocking checks; "P3 work" PRs require a link to a passing V5 + observability report |
| R-8 | P2 reload-on-any-doubt is so aggressive it's no better than today | Medium | Low | Track `hmr_update_total` vs `hmr_reload_total` ratio; if ratio < 50% updates after 30 days of dogfood, revisit boundary detection |
| R-9 | LSP from VS Code becomes the primary consumer of P3, accidentally forcing P3 to ship before its gate | Low | High | P6 SCA is the markdown-matcher alternative; full LSP gates on P3 gates |
| R-10 | Performance work distracts from production-grade gaps (security, scale, observability) | Low | High | This document explicitly cites the prior four designs as prerequisites; P3/P4 cannot begin until those land |

---

## SEQUENCING

```
                                 ┌─────────────────────────────────┐
                                 │ Prerequisites (already designed)│
                                 │  - VRC (SCA = compute_build_id) │
                                 │  - Security Baseline P1         │
                                 │  - Scale Benchmark V1-V4 + V5   │
                                 │  - Observability P1 + P2 + P3   │
                                 └─────────────────┬───────────────┘
                                                   │
            ┌──────────────────────────────────────┼──────────────────────────────────────┐
            │                                      │                                      │
            ▼                                      ▼                                      ▼
   ┌──────────────────┐                   ┌──────────────────┐                   ┌──────────────────┐
   │  P1 dep pre-bun  │                   │  P5 analyzer SCA │                   │  P6 SCA matcher  │
   │  (standalone)    │                   │  (markdown table │                   │  (problemsMatcher│
   │                  │                   │   from stats)    │                   │   tasks.json)    │
   └────────┬─────────┘                   └────────┬─────────┘                   └────────┬─────────┘
            │                                      │                                      │
            ▼                                      ▼                                      ▼
   ┌──────────────────┐                   ┌──────────────────┐                   ┌──────────────────┐
   │  P2 HMR          │                   │  P5 full viewer  │                   │  P6 full LSP     │
   │  (gates on P1)   │                   │  (frontend)      │                   │  (gates on P3 +  │
   │                  │                   │                  │                   │   stable schema) │
   └────────┬─────────┘                   └──────────────────┘                   └──────────────────┘
            │
            │  ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ EVIDENCE GATE ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
            │  (observability p95 + V5 pass + bailout instrumentation)
            ▼
   ┌──────────────────┐                   ┌──────────────────┐
   │  P3 SCA (cache   │                   │  P4 SCA (WAL +   │
   │   SCCs only)     │                   │   batched commit)│
   └────────┬─────────┘                   └────────┬─────────┘
            │                                      │
            │  ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ MEASURE ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
            │
            ▼                                      ▼
   ┌──────────────────┐                   ┌──────────────────┐
   │  P3 full         │                   │  P4 full (rayon  │
   │  (reachability + │                   │   parser)        │
   │   chunk deltas)  │                   │                  │
   └──────────────────┘                   └──────────────────┘

                                  P7 Rspack: deferred indefinitely
```

### What can land without dependencies

- **P1** (dep pre-bundling) — standalone. Ships any time.
- **P5 SCA** (markdown table) — depends only on observability P1 (build-stats.json), which is already designed.
- **P6 SCA** (problems matcher) — depends only on `wundler check` having stable output, which already exists.

### What is gated

- **P2** — gated on P1 stable. Slow dev server with HMR is worse than fast dev server without HMR.
- **P3** — gated on observability + V5. Hard gate.
- **P4** — gated on observability + SCA-shipped. Soft gate (SCA-first is the policy).
- **P5 full viewer** — gated on P5 SCA having been used and found inadequate.
- **P6 full LSP** — gated on diagnostic schema stability AND P3 (incremental analyzer is the LSP backend).

### What is deferred

- **P7 Rspack** — deferred until a named user.

---

## SIMPLEST CREDIBLE ALTERNATIVE PER TIER

| Priority | SCA | LoC | What it gives up |
|---|---|---|---|
| P1 | Hash `package.json` only, single cache dir, no GC | ~30 | misses lockfile-only changes; cache grows monotonically |
| P2 | Targeted reload via SW `client.navigate()`, no react-refresh | ~80 | no state preservation; still a full reload, just driven from SW |
| P3 | Cache `tarjan_sccs` output across analyses only | ~100 | only ~30% of full win; no reachability or chunk deltas |
| P4 | WAL mode + batched commits, no parallel parser | ~20 | parallelism win unrealized — but typically 10–100× is here anyway |
| P5 | Markdown table on stdout | ~50 | no interactive view; no treemap |
| P6 | `tasks.json` problems matcher | ~50 lines JSON | no incremental updates; runs `wundler check` on save |
| P7 | (none — defer) | 0 | n/a |

**Strong claim, written down so we can argue with it:** these SCAs together cover ≥ 70% of the user-visible win of the full design, at ≤ 15% of the code. They are the right starting point for every tier. The full designs are admissible only with measurement showing the SCA is insufficient.

---

## ACCEPTANCE — what "done" looks like per priority

| Priority | Done means |
|---|---|
| P1 | `wundler dev` cold start with cached deps completes in time ≤ T_baseline; with cache miss completes in time = T_baseline. Cache fingerprint changes correctly under `npm install`, `pnpm install`, `yarn`, `bun install`. |
| P2 | A React component edit updates in the browser without losing local state on at least one dogfood app. Mismatch / boundary-miss counter exists and is ≤ 1% of updates over 30 days. |
| P3 | V6 property test passes for K ∈ {1, 10, 100, 1000} on uniform.v1 AND on at least one real corpus. Bailout counter exists. Analyze p95 on the gated corpus drops below the gate threshold. |
| P4 | Idempotence test passes (re-ingest produces identical row count). Crash-during-ingest test passes (kill -9 during ingest; restart; final row count is exactly the same as a clean run). Ingest wall time on the gated corpus drops below 30s. |
| P5 | `wundler build --analyze` opens a treemap whose totals equal `build-stats.json::total_bytes` byte-for-byte. |
| P6 | `wundler lsp` emits diagnostics whose `code`, `severity`, and `range` match `wundler check` output verbatim on a 50-diagnostic corpus. |
| P7 | (n/a — deferred) |

---

## OPEN QUESTIONS (for the user)

1. **Dep pre-bundle scope.** Do we want to pre-bundle only bare specifiers (`import "react"`) or also slow source files in `node_modules` that ship raw ESM? The latter approaches Vite parity; the former is simpler. P1's SCA targets the former.
2. **HMR fallback granularity.** When `is_refresh_boundary` rejects, should we reload only the affected route (using SW `client.navigate()`) or the whole page? Route-level is friendlier; page-level is more predictable. Both are <100 LoC apart.
3. **Gate measurement source.** P3 gate cites "observability p95 > 5s". Is that the right number for our user base, or is the right threshold "more than one development team has filed a slowness ticket"? The numeric threshold is concrete and dispassionate; the ticket-based one is grounded in real human pain.
4. **P5 SCA vs full.** Are we comfortable shipping just the markdown table for the first version? It's adequate for CI, not adequate for "explain this bundle to a junior dev".
5. **VS Code priority.** The COE is clear that this is low priority. Should it leave the document entirely until someone asks for it, or is its presence here (gated, deferred) the correct treatment?

---

## CROSS-CUTTING — what this design depends on from other designs

| Dependency | Used by | What we need |
|---|---|---|
| VRC `compute_build_id` | P2, P3, P5 | Deterministic `build_id` for HMR event correlation, incremental analyze short-circuit, build-stats join key |
| VRC atomic manifest swap | P2 | HMR fires *after* the swap completes; never on a torn manifest |
| Security Baseline `[security]` token | P5 (analyzer's one-shot server) | The viewer's localhost server must use the same bearer token mechanism so it cannot be reached from a malicious browser tab |
| Scale Bench V5 (graph-shape similarity) | P3 | Proves the synthetic corpus the gate measures against is shaped like real corpora |
| Observability `build-stats.json` | P5 | The viewer's sole input |
| Observability Prometheus `/metrics` | P3 gate, P4 gate | Numeric thresholds for opening the gate |
| Observability `/telemetry/chunk-error` | (none directly, but informs which boundaries are HMR-critical) | future evolution of `is_refresh_boundary` |

---

## SUMMARY (for the writeup)

This document is **deliberately small in surface**. The recommendations:

- **P1 → C2** (fingerprint cache + atomic swap + LRU GC). Ships standalone.
- **P2 → C2** (react-refresh + `BuildEvent::Update`/`Reload`; fall-back-to-reload is the default). Gates on P1.
- **P3 → C2 (full)**, with **SCA (tarjan cache only) as the first ship**. Gates on observability + V5 + bailout instrumentation.
- **P4 → C2 (full)**, with **SCA (WAL + batched commit) as the first ship**. Gates on observability + SCA-first measurement.
- **P5 → full viewer**, with **markdown SCA as the first ship**. Read-only over build-stats.json, no second source of truth.
- **P6 → full LSP**, with **`tasks.json` SCA as the first ship**. Gates on stable diagnostic schema + P3.
- **P7 → deferred**.

The crucial discipline: **every "full" item has an SCA that lands first.** The SCA exercises the invalidation discipline, surfaces the missing instrumentation, and forces a measurement before the full design is admissible. This is how we honor the COE's evidence gate without ossifying.

**The most important sentence in the document, repeated:** *Incremental output must be bit-identical to full recompute output.* Encoded as V6 property test. Any divergence is P0. Speed without that invariant is not a feature — it is a regression with a stopwatch.
