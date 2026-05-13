# Wundler: A Teams-Scale Bundler

**Date:** 2026-05-13  
**Status:** Draft  
**Project:** wundler  

---

## Problem Statement

Modern JavaScript bundlers — Vite, webpack, esbuild, Rolldown — are architecturally unfit for Teams-scale applications (50k+ modules). The failure mode is identical across all of them: **eager, greedy, global analysis on every run**. Every module is parsed and analyzed on every build invocation, regardless of what changed.

At small scale this is acceptable. At Teams scale:

- Cold builds take minutes even with esbuild
- Incremental builds are limited by the build tool's coarse-grained cache
- Dev cold starts are sluggish; HMR degrades under large module graphs
- There is a structural dev/prod divergence: Vite's unbundled dev mode and bundled prod mode behave differently, causing subtle correctness bugs

The "linear build time" problem doesn't improve with a faster transform engine. It requires a different architecture — one that decouples "understand the graph" from "transform the code", so that only changed work is ever repeated.

**Bundle output quality (chunk optimization, delivery performance) is a tertiary concern.** You cannot optimize delivery if you cannot build the product in the first place.

---

## Design Goals

**Primary — the reason this exists:**

1. **Build speed** — cold builds scale sub-linearly with module count (O(n/cores)); incremental builds touch only what changed
2. **Dev speed** — zero bundling in development; sub-millisecond HMR for typical code changes

**Secondary — valuable, not blocking:**

3. **Delivery optimization** — better chunk groupings; adaptive per-client bundle serving that improves automatically over time

**Non-goals for v1:**

- Scope-hoisting parity with webpack (good enough output from Rolldown is sufficient)
- CSS module processing, SVG/image handling (delegated to Rolldown's existing plugin system)

---

## Core Concept: The Bundle Graph

The central paradigm shift: **the bundler no longer emits a file. It builds and maintains a Bundle Graph.**

Bundles, chunks, ABS responses, and dev-mode native ESM are all *materializations* of the same graph — computed on demand. The graph is the truth. Files are derived views.

Mental model: **database, not compiler.** The module graph is the schema. Bundle requests are queries. Materializations are computed views.

```
Source Files  →  Bundle Graph  →  materialize()  →  bundle.js
                              →  materialize()  →  chunks/ + manifest.json
                              →  materialize()  →  ABS delta manifest
                              →  materialize()  →  native ESM (dev)
```

No rebuild between environments. Same graph, different materialization strategy.

---

## Architecture Overview

Three phases populate the Bundle Graph. The Adaptive Bundle Service consumes it for delivery.

```
SOURCE FILES
     │
     ▼  Phase 1  ──  parallel, incremental
MODULE SUMMARIES  (cached by content hash)
     │
     ▼  Phase 2  ──  serial, fast (operates on summaries only, never source)
BUNDLE GRAPH  +  CHUNK MANIFEST
     │
     ├──▶  CDN  ──  content-hashed chunk files  [Level 0]
     │           ──  manifest.json (static fallback)
     │
     └──▶  ABS  ──  delta manifest server  [Level 1]
                         │
                         ▼  co-request logs
                    PGO STORE  →  suggestedMerge hints  →  feeds Phase 2  [Level 2]
```

Dev mode bypasses Phases 2 and 3. Modules are served as native ESM directly from Phase 1.

---

## Phase 1: Module Summarizer

### Overview

Phase 1 is the hottest code path in the system. It runs on every changed module, in parallel, and its output is cached by content hash and persists across builds indefinitely.

**Input:** one source file  
**Output:** `BundleGraphNode.summary`  
**Cache key:** `SHA-256(source)` — if hash unchanged, skip entirely

### BundleGraphNode

```typescript
interface BundleGraphNode {
  id:   ContentHash   // SHA-256(source) — stable identity, survives renames
  path: string

  // Phase 1 output — always present, ~2KB avg, never stale
  summary: {
    exports:     Export[]            // named, default, re-exports, star-exports
    imports:     Import[]            // static + dynamic import(), with exact bindings used
    sideEffects: SideEffectMarker    // see: Side Effects Decision below
    callEdges:   CallEdge[]          // export A internally calls export B — enables cross-module DCE
    ambientRefs: string[]            // globals read/written: window.X, document, globalThis.Y
  }

  // Phase 3 output — lazy, only produced for alive + reachable nodes
  transform?: {
    code:    string       // post-TS, post-JSX, minification-ready
    map:     SourceMap
    exports: string[]     // surviving exports after Phase 2 tree-shake (subset of summary.exports)
  }

  // Phase 2 output — thin-link decisions
  alive:    boolean       // reachable from any active entry point?
  chunkId?: ChunkId       // which chunk this node belongs to
}
```

### Side Effects Decision

**Module-level code** (any code that executes at import time): conservatively assumed to have side effects. A module is not eliminated from the graph unless it has zero live imports anywhere.

**Function-level code** (named exports, default export, helper functions): aggressively DCE'd using call-edge reachability from the summary. If no live entry point transitively calls a function, it is marked dead and stripped in Phase 3.

This is the ThinLTO model applied to JavaScript: conservative about module-level initialization, aggressive about function-level DCE via call-graph reachability.

```typescript
type SideEffectMarker =
  | { kind: 'NONE' }                     // proven: no module-level side effects
  | { kind: 'POSSIBLE'; reason: string } // heuristic: ambient refs detected
  | { kind: 'DEFINITE' }                 // explicit: @sideEffect annotation or sideEffects field in package.json
```

**Correctness audit mode** (mandatory, non-negotiable): A build mode that runs both Conservative-only and the heuristic side by side, diffs observable outputs, and alerts on any divergence. This must be implemented and pass before any tree-shaking ships to production users. Getting this wrong silently breaks production for 300M users. There is no acceptable tradeoff here.

### Validation Experiment (Month 1 Gate)

Run the summarizer across Teams' full module graph. Measure total summary cache size.

**Target:** 50k modules × ~2KB/summary ≈ 100MB

If summaries are 10× that (~1GB), the format is too fat and Phase 2's serial pass loses its performance advantage. This single measurement validates or invalidates the entire architecture before Phase 2 is written. Do not proceed to Phase 2 until this passes.

---

## Phase 2: Thin-Link Analysis

### Overview

Phase 2 is the only serial step in the pipeline. It operates exclusively on summaries — never on source.

At Teams scale: 50k modules × ~2KB/summary ≈ 100MB total. Single-threaded scan at memory bandwidth: ~50ms. The serial step is not a bottleneck.

**Input:** all `BundleGraphNode.summary` objects  
**Output:** per-node decisions (alive flag, chunk assignment) + `ChunkManifest`

Phase 2 re-runs only when a module's summary changes (new imports added, exports changed, side-effect status changed). Pure transform changes (implementation edits that don't affect the module's public surface) do not trigger Phase 2.

### Three Operations

**1. Reachability Analysis**  
BFS/DFS from all entry points. Mark every transitively reachable module `alive: true`. Dead modules skip Phase 3 entirely — they are never transformed, never written to disk. At Teams scale, a significant fraction of the codebase is unreachable from any active route on any given build. This is the largest single build speedup.

**2. Function-level DCE**  
Per live module: walk call-edges from exported symbols. Any export with zero inbound calls from any live module is marked dead. Phase 3 uses this to strip dead exports from the transform output, producing smaller chunks without requiring full cross-module IR analysis.

**3. Chunk Assignment (Day 1: Route-Based)**  
Split at every dynamic `import()` boundary. Modules referenced by N+ chunks (configurable threshold, default: 2) are extracted into a commons chunk.

This is not novel — it is what webpack's `splitChunks` has done since 2018. It is correct, predictable, and debuggable. The PGO-based chunk optimization (Level 2) replaces or refines this once the ABS has collected real usage data. Do not design day-1 chunk assignment for a day-90 optimization.

### ChunkManifest

The `ChunkManifest` is the primary output of Phase 2. It is the handshake between the build system and the Adaptive Bundle Service.

```typescript
interface ChunkManifest {
  buildId:      string                                 // SHA of all entry points — CDN cache-bust key
  chunks:       Chunk[]
  entryChunks:  Record<EntryPoint, ChunkId[]>         // which chunks to load for each route
  moduleIndex:  Record<ContentHash, ChunkId>          // fast lookup: module → chunk
}

interface Chunk {
  id:            ChunkId
  modules:       ContentHash[]
  hash:          ContentHash         // chunk-level cache key — changes only when a member module changes
  loadCondition: 'INITIAL' | 'LAZY' | 'PREFETCH'

  // PGO fields — null on day 1, populated by ABS after data collection
  coRequestScore?:   number    // P(this chunk | initial load) from real sessions
  medianLoadOrder?:  number    // when in the session timeline this chunk is typically needed
  suggestedMerge?:   ChunkId  // ABS recommendation: merge this chunk with another
}
```

---

## Phase 3: Transform Engine

### Overview

Phase 3 operates through a **pluggable `TransformEngine` interface**. It receives a set of module nodes and chunk assignments from Phase 2 and produces content-hashed chunk files. Phase 3 is the most isolated part of the system — its contract is narrow enough to support multiple concrete backends.

**Input:** `BundleGraphNode[]` (per chunk) + Phase 2 decisions  
**Output:** `ChunkOutput[]` (code + source map + content hash, per chunk)  
**Scope:** only `alive: true` nodes whose content hash changed since the last build

Phase 3's wall-clock time is proportional to `(changed_alive_modules / CPU_cores)` — not total module count.

### TransformEngine Interface

```typescript
interface TransformEngine {
  // Transform a set of module nodes into one chunk
  transformChunk(
    modules: BundleGraphNode[],
    chunkId: ChunkId,
    decisions: PhaseDecisions
  ): Promise<ChunkOutput>

  // Finalize: write all chunk files to disk
  emit(outputs: ChunkOutput[], outDir: string): Promise<void>
}

interface ChunkOutput {
  chunkId:  ChunkId
  hash:     ContentHash
  code:     string
  map:      SourceMap
}
```

### Adapters

**`RolldownAdapter`** (default): Drives Rolldown (Rust, Rollup-compatible). No FFI boundary — same runtime as Phases 1 and 2. Best long-term performance. Rollup plugin ecosystem available.

**`RspackAdapter`**: Drives Rspack (Rust, webpack-compatible). Enables Teams to adopt ThinBundle's Phase 1 + Phase 2 incrementally without replacing an existing Rspack setup. webpack plugin ecosystem available. Introduces a thin API boundary but Phase 3 is not on the hot path, so this is acceptable.

### Trade-off

Two adapters means two code paths to maintain. Any new Phase 3 capability (CSS module changes, new asset handling) must be implemented in both. **Ship one adapter on day 1** — whichever engine is closer to Teams' current toolchain. Define the interface now. Add the second adapter when there is a concrete need for it.

---

## Adaptive Bundle Service (ABS)

### Core Insight

The ABS is **not a CDN replacement**. It is a lightweight coordinator in front of a static CDN. It answers one question per request:

> *"Given the module hashes you already have in your cache, which CDN URLs do you need to fetch?"*

**ABS never serves code. CDN serves code.** This distinction is what makes the security model workable in an enterprise context.

### Request Protocol

**Client → ABS:**
```json
{
  "entryPoint": "teams.channel",
  "cachedHashes": ["a3f2...", "9d1c..."],
  "buildId": "b8f3..."
}
```

**ABS → Client:**
```json
{
  "buildId":      "c2a1...",
  "fetchUrls":    ["/chunks/4f1a.js"],
  "prefetchUrls": ["/chunks/7e3b.js"],
  "ttl":          300
}
```

ABS logs which chunks were served together per session. This co-request log is the raw data for Level 2 PGO chunk optimization.

### Security Model

ABS compromise lets an attacker redirect clients to different *existing* content-hashed chunks — not inject arbitrary code. To ship malicious code, an attacker must also compromise the CDN build pipeline and produce a valid content hash. This is a substantially harder attack surface than serving arbitrary content from a compromised CDN.

For Teams enterprise deployment: chunk hashes in the ChunkManifest can be **build-time signed** (ed25519 key held by the build system) and verified by the service worker on receipt. ABS manifest tampering becomes detectable, not just difficult.

### Service Worker

Every sufficiently large web application already has a service worker managing assets. For Teams, the service worker is existing infrastructure — not new operational overhead. The delta-manifest protocol is a configuration update to the SW, not a new service.

The SW is responsible for:
- Maintaining the local cache of content-hashed chunks
- Sending `cachedHashes` to ABS on each navigation
- Fetching missing chunks from CDN URLs returned by ABS
- Detecting `buildId` changes and triggering background updates
- Falling back to `manifest.json` on ABS failure

### Failure Model

| Scenario | Behavior |
|---|---|
| ABS unreachable | SW falls back to `manifest.json` on CDN, silently. No user impact beyond missing delta optimization. |
| New build deployed | ABS returns updated `buildId`. SW detects change, background-fetches new chunks. User sees new code on next navigation — not mid-session. No forced reload. |
| Cold-start user (empty cache) | `cachedHashes: []` → ABS returns the full initial chunk set, identical to the static manifest. No regression from current behavior. |
| Service worker bug | `?nosw=1` query parameter bypasses SW entirely. Falls back to classic CDN loading. Standard escape hatch for Teams' enterprise IT debugging. |
| ABS p99 latency spike | SW timeout (recommended: 100ms) falls back to static manifest. ABS is never on the hard critical path for page load. |

---

## Dev Mode

Phases 2 and 3 are bypassed entirely. Modules are served as native ESM from Phase 1 results (TypeScript/JSX transformed on demand per-module).

**HMR on source change:**
1. Phase 1 re-summarizes the changed module — milliseconds
2. If the module's summary is unchanged (pure implementation edit): Phase 2 skips entirely
3. SW receives notification: "module X has a new content hash"
4. SW fetches just that module from the module server
5. Browser hot-swaps the module

Result: sub-millisecond HMR for typical implementation changes. Phase 2 only re-runs when the module's public surface changes (new imports, changed exports). No full graph rebuild on routine edits.

---

## Deployment Levels

### Level 0 — Static Only

ThinBundle (Phases 1–3 + Rolldown) replaces the current build pipeline. ChunkManifest and content-hashed chunk files are pushed to CDN. `manifest.json` is published as a static fallback.

**No new infra. No service worker changes.**

Already better than today: faster incremental builds, dead-code elimination at Teams scale, predictable chunk boundaries. Validates the ThinBundle pipeline end-to-end before any dynamic serving is introduced.

**Ship this first. Do not build Level 1 until Level 0 is stable in production.**

### Level 1 — ABS as Manifest Server

ABS deployed. SW updated to send `cachedHashes` on navigation and receive delta manifests. PGO data collection begins.

Repeat visitors get faster loads proportional to their cached state. New data pipeline for Level 2. Transparent fallback to Level 0 at all times.

### Level 2 — PGO-Driven Chunk Reorganization

Co-request data from Level 1 feeds back into Phase 2's chunk assignment via `suggestedMerge` hints. Chunk groupings reorganize based on real usage — without triggering a full rebuild.

Requires several weeks of Level 1 traffic to produce statistically meaningful signal. Do not implement before the data exists. Target: ~6 months post Level 0 deployment.

---

## Implementation Roadmap

### Month 1 — Phase 1: The Summarizer

Build the module summarizer in isolation: one source file in, `BundleGraphNode.summary` out. Implement the content-addressed summary cache.

**Validation gate:** Run the summarizer across Teams' full module graph. Target: ~100MB total summary cache size. If this fails (summaries are too fat or too slow), stop and redesign the summary format before proceeding. This is the cheapest possible proof point for the entire architecture.

### Month 2 — Phase 2: Thin-Link

Implement reachability, function-level DCE via call-edges, and route-based chunk assignment. Output: `ChunkManifest`.

**Validation gate:** Feed Phase 2 output into Rolldown. Diff bundle output against the current build (Vite/webpack baseline). Confirm correctness before declaring Phase 2 complete. The correctness audit mode (side-effect diffing) must be implemented and passing before this milestone closes.

### Month 3 — Level 0: Replace the Build

Deploy ThinBundle (Phases 1–3 + Rolldown) replacing the current build pipeline in CI. Push chunks + manifest to CDN.

**Metrics to capture at launch:**
- Cold build time (full from scratch)
- Incremental build time (single file change)
- Percentage of modules eliminated as dead code
- Bundle size delta vs. current baseline

### Month 4+ — Level 1: ABS

Deploy the manifest server. Update the SW. Start collecting co-request data.

Level 2 follows when PGO data stabilizes — approximately Month 6+.

---

## Implementation Language

**Rust.** The relevant ecosystem has converged: SWC (transforms), Rolldown (bundler core), Turbopack (incremental graph) are all Rust. Implementing Phase 1 and Phase 2 in Rust means native integration with Rolldown for Phase 3, with no FFI boundary on the hottest code path. Writing in Go or Node would introduce a serialization boundary at exactly the wrong point.

---

## Key Design Decisions

| Decision | Chosen | What Was Considered | Reason |
|---|---|---|---|
| Primary artifact | Bundle Graph | Bundle file | Enables multiple materializations from one build; eliminates dev/prod divergence |
| Side effects — module level | Conservative | Aggressive static analysis | Aggressive failure mode is silent P0 production bugs at 300M users scale |
| Side effects — function level | Aggressive (call-edge DCE) | Conservative | Functions with zero callers from live entry points are provably dead |
| Correctness audit mode | Mandatory | Optional / deferred | Required before any tree-shaking ships to production. Non-negotiable. |
| Day-1 chunk strategy | Route-based splitting | Package-based, PGO | Correct, predictable, compatible with PGO upgrade path in Level 2 |
| Phase 3 engine | Pluggable `TransformEngine` interface | Build from scratch | Narrow contract enables Rolldown (default) and Rspack adapters; only Phase 3 is behind the interface — Phases 1+2 have no FFI concern |
| ABS serving model | Manifests only, never code | Serving code directly | ABS compromise ≠ code injection; substantially better security model for enterprise |
| Service worker dependency | Assumed present | New infrastructure | Standard in every large-scale web application; not new operational overhead |
| Implementation language | Rust | Go, Node.js | Ecosystem convergence; native Rolldown integration; no FFI on hot path |

---

## Open Questions

1. **Summary format versioning** — when `BundleGraphNode.summary` schema changes, cached summaries become invalid. Need a version prefix in the cache key (e.g., `v2:<SHA-256(source)>`) and a migration path for the cache store.

2. **Dynamic `import()` with variable specifiers** — `import(\`./\${name}\`)` is unresolvable statically. Conservative handling: treat as a dependency on all modules matching the glob pattern. Expensive but correct. Requires a well-defined escape hatch for intentional dynamic resolution.

3. **Circular imports** — the module graph has cycles. Phase 2's reachability traversal must handle SCCs (strongly connected components via Tarjan or Kosaraju). Chunk assignment for a cycle: all nodes in the SCC are assigned to the same chunk.

4. **Monorepo boundary** — does one Bundle Graph span the entire Teams monorepo, or is there one per deployable app with shared summary caches? Single graph enables better dead-code analysis across package boundaries; per-app graphs are simpler to reason about. Likely answer: one graph per deployable app, summary caches shared across the monorepo via the content-addressed store.

5. **ABS availability SLO** — what is the acceptable p99 latency for an ABS manifest request? Recommendation: <50ms p99. The SW should timeout at 100ms and fall back to `manifest.json` — ABS must never be on the hard critical path for page load. This SLO must be defined before Level 1 ships.

6. **Non-JS assets in Phase 1** — CSS modules, SVGs, JSON imports. Summary for non-JS assets: `{ exports: ['default'], imports: [], sideEffects: { kind: 'DEFINITE' }, callEdges: [], ambientRefs: [] }`. All non-JS assets are treated as having definite side effects (CSS injection is always a side effect). Actual processing delegated to Rolldown plugins in Phase 3.

---

*Design finalized in brainstorming session on 2026-05-13. Implementation begins with the Month 1 validation experiment: run the summarizer against Teams' full module graph and confirm summary cache size is within target (~100MB). That single measurement is the gate before any further investment.*
