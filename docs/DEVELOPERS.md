# wundler developer reference

Audience: engineers who want to understand the internals, contribute, or build on top of wundler.

## 1. Repository layout

```
wundler/
├── Cargo.toml                  # workspace manifest
├── crates/
│   ├── wundler-core/           # summarizer, cache, CJS stub generator
│   ├── wundler-graph/          # reachability, SCC, ChunkManifest builder
│   ├── wundler-transform/      # TransformEngine trait + adapters
│   ├── wundler-pipeline/       # BuildPipeline, dev server
│   ├── wundler-abs/            # Adaptive Bundle Service
│   ├── wundler-pgo/            # PGO store, ingestor, C³, updater
│   ├── wundler-sw/             # Service Worker (TypeScript, bundled into wundler-abs)
│   └── wundler-cli/            # CLI subcommands
├── docs/
│   ├── wundler-architect-brief.md
│   ├── DEVELOPERS.md
│   └── superpowers/plans/      # design plans, current and future
└── test-app/                   # React 19 + TS validation app
    ├── src/                    # routes, components, dead exports, analytics side-effect
    ├── wundler-rolldown.toml
    └── wundler-swc.toml
```

The workspace is a single Cargo workspace. Each crate has its own `Cargo.toml` and `tests/` directory. `wundler-cli` depends on every other crate and is the only binary target.

## 2. ModuleSummary — the atomic unit

A `ModuleSummary` is the only thing the graph analyzer ever reads. It is sufficient for cross-module analysis: reachability, dead-export elimination, chunk assignment. The full AST is never required after the summary is produced.

```rust
pub struct ModuleSummary {
    pub source_hash: ContentHash,        // SHA-256 of source bytes
    pub path: PathBuf,
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
    pub call_edges: Vec<CallEdge>,
    pub side_effects: SideEffects,
    pub ambient_refs: Vec<String>,       // e.g. globalThis, window.X
}

pub struct Export {
    pub name: String,                    // "default" for default export
    pub kind: ExportKind,                // Function | Class | Const | Let | ReExport
    pub source: Option<String>,          // for re-exports: original module specifier
}

pub struct Import {
    pub specifier: String,               // "./foo", "react"
    pub bindings: Vec<ImportBinding>,    // named, default, namespace
    pub kind: ImportKind,                // Static | Dynamic
}

pub struct CallEdge {
    pub from_export: String,             // local export name
    pub to_specifier: String,            // imported module
    pub to_binding: String,              // imported binding name
}

pub enum SideEffects {
    None,
    Possible { reason: String },         // e.g. "top-level call to unknown function"
    Definite,                            // e.g. observable assignment to globalThis
}
```

The rule for `SideEffects`: conservative at module boundaries, aggressive at function boundaries. A top-level call whose target is not statically resolvable to a side-effect-free export is `Possible`. A side-effect-free function called only by a dead export is itself dead, regardless of its `SideEffects` marker, because call-edge reachability handles it.

`call_edges` is what makes export-level DCE work. Without it, dropping an export risks dropping its transitive helpers from other modules.

## 3. ChunkManifest

The output of `wundler-graph`. The handshake with everything downstream.

```json
{
  "build_id": "sha256:…",                      // SHA of entry points + summary set
  "chunks": [
    {
      "id": "chunk-0",
      "modules": ["sha256:…", "sha256:…"],     // ContentHash list, ordered
      "hash": "sha256:…",                      // hash of (id + module hashes)
      "load_condition": "Initial",             // Initial | Lazy | Prefetch
      "co_request_score": 0.87,                // PGO: P(this | initial), optional
      "median_load_order": 2,                  // PGO: derived from sessions, optional
      "suggested_merge": "chunk-7"             // PGO: merge candidate, optional
    }
  ],
  "entry_chunks": {
    "/": ["chunk-0", "chunk-1"],
    "/dashboard": ["chunk-0", "chunk-3"]
  },
  "module_index": {
    "sha256:…": "chunk-0",
    "sha256:…": "chunk-1"
  }
}
```

Every module is identified by `SHA-256(source_bytes)`. Chunk membership is content-addressed: two builds that produce the same module set in the same chunk produce the same `chunk.hash`. The `build_id` changes when the entry points or the summary set change. PGO fields (`co_request_score`, `median_load_order`, `suggested_merge`) are written in place by `wundler pgo apply` without modifying any other field.

## 4. The build pipeline

`wundler-pipeline::BuildPipeline` runs four stages, each producing input for the next.

**Summarize.** Walk the entry points and their transitive imports. For each source file, look up `SHA-256(source)` in the 2-tier cache (in-memory LRU → on-disk content-addressed). On miss, parse with SWC and emit a `ModuleSummary`; insert into both tiers. Driven by `rayon::par_iter` over the file set. No shared mutable state — the cache is the only shared structure and uses interior locking on the cold-tier writes only.

**Analyze.** `wundler-graph` reads the summary set. Three passes:
1. BFS reachability from the entry-point set. Modules not reached are dead and excluded from emission.
2. Tarjan SCC over module-level imports. Strongly connected components must stay in the same chunk; otherwise circular ESM evaluation breaks.
3. Route-based chunk splitting. Modules reachable from exactly one route become route-local chunks. Modules reachable from N ≥ `commons_threshold` routes become commons chunks. Lazy imports become `Lazy` chunks.

The output is a `ChunkManifest` with no PGO fields populated.

**Transform.** `TransformEngine::batch_transform(chunks, summaries) -> Vec<EmittedChunk>`. The engine receives the assignment and produces JavaScript output per chunk. See §5.

**Emit.** `OutputWriter::write_chunk` writes each `EmittedChunk` to `out_dir/<chunk_hash>.js`. `write_manifest` serializes the `ChunkManifest` to `out_dir/manifest.json` (optionally signed). `write_index_html` generates an `index.html` with `<script type="module">` tags for the entry chunks.

## 5. TransformEngine trait

```rust
pub trait TransformEngine: Send + Sync {
    fn name(&self) -> &str;

    fn batch_transform(
        &self,
        chunks: &[ChunkAssignment],
        summaries: &SummaryStore,
    ) -> Result<Vec<EmittedChunk>, TransformError>;
}
```

Two implementations.

**`SwcTransformAdapter`.** Per-chunk transform. For each chunk, reads the source of every member module, applies SWC passes (TypeScript strip, JSX transform, dead-export strip using the summary's call-edge information), and concatenates the resulting modules into a single chunk file, flattening their scopes. Fast and self-contained. Known limitation: scope-flattening across modules with conflicting default-export aliasing or complex re-export chains can produce incorrect output. Used by `wundler dev` (one module per request, no flattening) and acceptable for many production cases.

**`RolldownAdapter`.** One subprocess invocation per build. Wundler writes a temporary entry stub per chunk, points rolldown at the entry stubs, and lets rolldown do scope merging. Slower than `SwcTransformAdapter` per invocation, but inherits rolldown's correctness for the edge cases SWC's intra-chunk flattening misses. Preferred for production builds.

The choice is `[build] engine = "rolldown" | "swc"` in `wundler.toml`.

## 6. Adaptive Bundle Service

The ABS is a manifest server. The CDN serves the static chunks. They are separate.

### Delta protocol

```
POST /manifest
{
  "entry_point": "/dashboard",
  "cached_hashes": ["sha256:…", "sha256:…"],   // module-level hashes, NOT chunks
  "build_id": "sha256:…"                        // optional, last known build_id
}

200 OK
{
  "build_id": "sha256:…",
  "fetch_urls": ["/c/abcd1234.js", "/c/ef567890.js"],
  "prefetch_urls": ["/c/fa11ce.js"],
  "ttl": 300
}
```

`cached_hashes` are **module-level** SHA-256 values. This is what matters: a chunk whose modules are all already cached on the client (perhaps from a previous build, in a different chunk grouping) does not need to be fetched. Chunk-level hashes would force a fetch every time the manifest reshuffles modules across chunks, which is exactly what PGO does.

The server:
1. Looks up `entry_chunks[entry_point]` in the current manifest.
2. Walks the chunk graph (commons dependencies, route chunks, lazy chunks that the entry depends on).
3. For each candidate chunk, checks whether every module in `chunk.modules` appears in `cached_hashes`. If yes, skip. If no, emit its URL.
4. Returns the resulting URL list.

If `build_id` is present and matches the current manifest, the server can shortcut by comparing chunk membership directly. If it does not match, the server computes the full delta against `cached_hashes`.

The Service Worker has a 100ms p99 timeout on the ABS request. On timeout, it falls back to the static `manifest.json` served from the CDN — the same one the build emitted. The ABS is never on the hard critical path.

### Signing

`wundler abs keygen` generates an ed25519 key pair. `wundler build --sign --key signing.pem` signs the manifest. The Service Worker verifies the signature with the embedded public key before trusting `fetch_urls`. Compromise of the ABS without the signing key cannot inject new content — only redirect to existing content-hashed chunks.

## 7. PGO store

`wundler-pgo` is a SQLite database, a JSONL ingestor, and a C³ clustering pass.

### Co-request matrix schema

```sql
CREATE TABLE sessions (
  session_id TEXT PRIMARY KEY,
  build_id TEXT NOT NULL,
  entry_point TEXT NOT NULL,
  started_at INTEGER NOT NULL
);

CREATE TABLE chunk_loads (
  session_id TEXT NOT NULL,
  chunk_id TEXT NOT NULL,
  load_order INTEGER NOT NULL,
  PRIMARY KEY (session_id, chunk_id)
);

CREATE INDEX chunk_loads_by_chunk ON chunk_loads(chunk_id);
```

`insert_session` is idempotent on `session_id` to tolerate retried browser uploads.

### C³ algorithm

For every pair of chunks `(A, B)` co-loaded in at least `min_sessions` sessions, compute:

```
P(A | B) = sessions(A ∩ B) / sessions(B)
P(B | A) = sessions(A ∩ B) / sessions(A)
```

If `min(P(A|B), P(B|A)) > threshold` (default 0.8), the pair is a merge candidate. Candidates are sorted by `min(P(A|B), P(B|A))` descending and processed greedily: each chunk participates in at most one merge per pass. The sort is stable on `(chunk_id_a, chunk_id_b)` so the output is deterministic given the same input matrix.

`wundler pgo analyze` prints the candidate list. `wundler pgo apply` writes `suggested_merge` and `co_request_score` fields back into the manifest.

### Two-stage update

`wundler pgo apply` never mutates `chunks[].modules`, `chunks[].hash`, `build_id`, `entry_chunks`, or `module_index`. It writes PGO fields only. The write is staged: serialize the new manifest to `manifest.json.tmp`, `fsync`, then `rename` to `manifest.json`. The rename is atomic on POSIX filesystems. Concurrent readers see the old or new manifest, never a partial one.

## 8. Service Worker

`wundler-sw` is bundled into the `wundler-abs` binary and served alongside the CDN. The SW is the runtime that makes adaptive delivery possible.

**Install.** Fetch the static `manifest.json` from the CDN. Store it in IndexedDB keyed by `build_id`. This is the fallback for ABS timeouts.

**Activate.** `clients.claim()`. Take control of all clients in scope.

**Fetch intercept.** Only for navigation requests (HTML documents). On a navigation:
1. Enumerate `caches.match` keys to collect the set of module-level hashes currently in Cache Storage.
2. POST to ABS with `entry_point`, `cached_hashes`, and `build_id`.
3. On 200, fetch each URL in `fetch_urls` in parallel and put them in Cache Storage. Pass through the original navigation request.
4. On timeout (100ms p99) or 5xx, fall back to the cached static manifest and compute the fetch set locally.
5. Schedule `prefetch_urls` on `requestIdleCallback`.

**Build rotation.** If the ABS response contains a `build_id` different from the client's current one, the SW `postMessage`s every controlled client with `{ type: "build-rotated", build_id }`. Clients reload on next navigation. The old `build_id`'s chunks remain in Cache Storage until eviction; module-level content-addressing means the next build can reuse them.

## 9. Running tests

```bash
cargo test -p wundler-core      # 65 tests: extractors, cache, CJS stubs
cargo test -p wundler-graph     # BFS, SCC, chunking, manifest
cargo test -p wundler-transform # SWC adapter, JSX transform, intra-chunk stripping
cargo test -p wundler-pipeline  # BuildPipeline, output writer, level0 ESM build
cargo test -p wundler-abs       # delta computation, signing, telemetry
cargo test -p wundler-pgo       # store queries, ingestor, C³, updater
cargo test                      # all
```

Integration tests against `test-app/` live in `crates/wundler-pipeline/tests/`. They run a full build and compare the chunk set and module assignment against fixtures.

## 10. Known issues and future work

- **Dev server is live reload, not HMR.** A page reload happens on every change. True module-replacement HMR is planned. See `docs/superpowers/plans/future/true-hmr.md`.
- **`SwcTransformAdapter` scope flattening.** Complex default-export patterns and re-export chains can produce incorrect output. Prefer `RolldownAdapter` for production builds. The dev server, which serves one module per request without flattening, is unaffected.
- **ABS signature verification is incomplete.** `signing_key_pem` is honored at build time but `server::run()` does not yet require verification on inbound requests. Manifests are accepted unsigned if no key is configured. Tracked for the next cycle.
- **Service Worker `session_id`.** Currently derived from `build_id`, which means multiple navigations on the same build collapse into a single PGO session. Should be a per-navigation UUID for accurate co-request matrices.
