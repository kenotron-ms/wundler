# wundler

**The module graph is the software. The bundle is a query.**

Wundler is a JavaScript/TypeScript module bundler written in Rust, built on linker-theory principles. It treats your source tree as a single content-addressed graph and treats every output — a production bundle, a dev-server response, a manifest — as a materialization of a query against that graph.

This is an early but working bundler. Version 0.1.0 produces correct multi-chunk builds of a React 19 + TypeScript SPA today.

---

## Why another bundler?

Most JavaScript bundlers were architected before anyone seriously cared about graphs of 50,000+ modules. They scan, parse, transform, and emit in one pipeline pass per build. When your graph gets big — a real monorepo with internal packages, a design system, a generated GraphQL client — the per-build work scales linearly with source size, and incremental rebuilds end up redoing most of it.

Compilers solved this problem twenty years ago, and the solution has a name: **ThinLTO**.

ThinLTO's insight: don't do whole-program analysis on whole-program IR. Summarize each translation unit into a tiny side file, do whole-program analysis on the summaries, then emit per-unit codegen in parallel — only for units that changed.

Wundler applies the same three-stage decomposition to JavaScript:

1. **Summarize.** Parse each file once via SWC. Emit a ~2 KB `ModuleSummary` (exports, imports, side-effect status, call edges, ambient globals). Cache by `SHA-256(source)`. Unchanged files cost zero work on the next build.
2. **Analyze.** Load every summary. Build the dependency DAG. Run SCC-aware BFS reachability from entry points. Eliminate unreachable modules. Compute per-export dead-code via call-edge traversal. Assign survivors to chunks.
3. **Transform.** For each chunk, TS-strip the alive modules, flatten their ESM scopes, concatenate, and write a content-hash-named `.js` file. Emit `manifest.json` and `index.html`.

The dev server skips steps 2 and 3 entirely: it serves TypeScript on-demand with SWC stripping, plus SSE-based HMR.

The module graph isn't a thing the bundler builds. The module graph is the thing your codebase already is. Bundles are queries.

---

## Crate layout

```
wundler-cli
  └─ wundler-pipeline       (orchestration, config, dev-server)
       ├─ wundler-transform (SWC / Rolldown engines)
       └─ wundler-graph     (reachability, DCE, chunking, manifest)
            └─ wundler-core (parser, summarizer, types, cache)
```

| Crate | Responsibility |
|---|---|
| `wundler-core` | SWC parser, module summarizer, content-addressed local cache (`~/.wundler/cache/`), CJS detection, ESM stub generation. Produces `BundleGraphNode { id: SHA-256, path, summary, alive, chunk_id, source }`. |
| `wundler-graph` | Petgraph-based dep graph. Tarjan SCC, BFS reachability (SCC-atomic), export-level DCE via call-edge traversal, 3-phase commons-chunk assignment, manifest building. |
| `wundler-transform` | `TransformEngine` trait. SWC adapter (TS-strip → dead-export filtering → ESM scope-flattening → concatenation). Rolldown adapter (spawns a node process, falls back to SWC on failure). |
| `wundler-pipeline` | Tokio/Axum. `BuildPipeline` orchestrating all stages. Dev server with file watcher. Config parsing (`wundler.toml`). Output writers. |
| `wundler-cli` | Clap-based binary. Five subcommands. |

---

## Quick start

```bash
# Build from source
git clone <repo> wundler
cd wundler
cargo build --release

# Drop the binary somewhere on PATH
cp target/release/wundler /usr/local/bin/

# Build a project that has a wundler.toml at its root
cd my-app
wundler build

# Or run the dev server
wundler dev
```

Output lands in the directory named by `out_dir` in your `wundler.toml` (default `dist/`).

---

## Configuration: `wundler.toml`

```toml
[build]
root = "src"                 # source root, scanned recursively
out_dir = "dist"             # where chunks, manifest.json, index.html go
engine = "swc"               # "swc" | "rolldown" | "rspack" (rspack falls back to swc)
source_maps = false          # emit .js.map alongside chunks (line-coarse for now)
commons_threshold = 2        # a module shared by ≥ N initial chunks becomes commons

[entry]
"/"      = "src/main.tsx"           # the route → entrypoint map
"/admin" = "src/admin/index.tsx"    # one chunk per route, plus lazy chunks for dynamic import()
```

Routes in `[entry]` are arbitrary strings — wundler uses them to slugify chunk filenames and to key the `entry_chunks` table in the manifest. Use whatever shape your runtime router prefers.

---

## CLI

Five subcommands. Each is useful on its own; they share the same internal stages.

### `wundler summarize <PATH>`

Parse a single file and print its `ModuleSummary` as JSON. Useful for inspecting what wundler sees in one source file without building anything.

```bash
$ wundler summarize src/components/Button.tsx
{
  "id": "f3a9c1...",
  "imports": [{ "specifier": "react", "kind": "static" }],
  "exports": [{ "name": "Button", "kind": "function" }],
  "side_effects": false,
  "call_edges": [...]
}
```

### `wundler validate-scale <DIR>`

Walk `<DIR>` in parallel, summarize every JS/TS file, print Phase 1 gate metrics (file count, cache hit rate, per-file timing percentiles). This is the throughput gate — if summarize is slow, everything else will be.

```bash
$ wundler validate-scale ./src
parsed 1,247 files in 312 ms (cold: 1,247  cached: 0)
p50 0.21 ms   p95 0.58 ms   p99 1.4 ms
```

### `wundler analyze <DIR> --entry ROUTE=PATH`

Run summarize → graph build → reachability → chunking, then print the resulting chunk manifest. No code is emitted. This is the planning view.

```bash
$ wundler analyze ./src --entry /=src/main.tsx --entry /admin=src/admin/index.tsx
chunks:
  commons       5 modules
  initial_root  6 modules
  initial_admin 4 modules
  lazy_1        1 module (src/Dashboard.tsx)
```

### `wundler build [--config wundler.toml]`

The full pipeline: summarize → analyze → transform → emit. Writes `dist/chunks/*.js`, `dist/manifest.json`, `dist/index.html`. Honors the cache; unchanged files round-trip through summarize at zero cost.

```bash
$ wundler build
✓ summarized 47 modules (cache hits: 41)
✓ reachability: 38 alive, 9 dropped
✓ chunked: commons + 2 initial + 2 lazy
✓ emitted dist/ (4 chunks, 1 manifest, 1 html)
```

### `wundler dev [--config wundler.toml]`

Axum dev server. Serves TypeScript on demand through the SWC transformer. Watches the source tree and pushes SSE updates on change. No chunking, no manifest — the graph is a query you make every request.

```bash
$ wundler dev
listening on http://localhost:5173
watching ./src
```

---

## Output

```
dist/
├── chunks/
│   ├── 9a1b2c3d…f0.js
│   ├── 9a1b2c3d…f0.js.map    # only if source_maps = true
│   └── …
├── index.html
└── manifest.json
```

Chunk filenames are SHA-256 content hashes of the emitted bytes — stable across builds when inputs don't change, immutable for cache-busting.

Chunk IDs in the manifest follow a fixed scheme:

| ID | Meaning |
|---|---|
| `commons` | Modules referenced by ≥ `commons_threshold` initial chunks |
| `initial_<slug>` | The chunk loaded for entry route `<slug>` (e.g. `initial_root`, `initial_admin`) |
| `lazy_N` | Chunks created from dynamic `import()` boundaries (1-indexed) |

### `manifest.json` — the build↔runtime handshake

```json
{
  "build_id": "2026-05-13T18:57:00Z-9a1b2c3d",
  "chunks": [
    {
      "id": "initial_root",
      "modules": ["src/main.tsx", "src/App.tsx", "..."],
      "hash": "9a1b2c3d…",
      "load_condition": "entry"
    },
    {
      "id": "lazy_1",
      "modules": ["src/Dashboard.tsx"],
      "hash": "7e8f4a…",
      "load_condition": "dynamic"
    }
  ],
  "entry_chunks": {
    "/":      ["commons", "initial_root"],
    "/admin": ["commons", "initial_admin"]
  },
  "module_index": {
    "src/App.tsx": "initial_root",
    "src/Button.tsx": "commons"
  }
}
```

The runtime loader only needs to know:

- Which chunks to inject for a given route (`entry_chunks`).
- Which chunk a module lives in, if it needs to resolve a dynamic import (`module_index`).

Everything else — load order, side-effect ordering, chunk shape — is precomputed.

---

## How it works

### Stage 1 — Summarize

For each `.js / .jsx / .ts / .tsx` under `root`:

1. Read source, hash with SHA-256.
2. Check `~/.wundler/cache/<hash>.summary.json`. If hit, deserialize and return.
3. Otherwise, parse via SWC, walk the AST, extract:
   - `imports`: specifier + kind (`static` or `dynamic`)
   - `exports`: name + kind (`function`, `const`, `class`, `default`, …)
   - `side_effects`: any top-level statement that isn't a declaration
   - `call_edges`: which exports call which imports
   - `ambient_globals`: bare references that aren't imported or declared
4. Write the cache entry.

Output is `~2 KB` per module. Summaries are the **only** thing stage 2 looks at; source bytes are not loaded again until transform.

### Stage 2 — Analyze

1. Load every summary. Build a petgraph `DiGraph` keyed by module ID.
2. Run Tarjan to identify strongly connected components (cycles). SCCs are treated atomically: a module is alive iff its SCC is alive.
3. BFS from each entry point. Mark reachable. Drop the rest.
4. For each alive export, walk the call-edge graph. Mark per-export liveness. *(This is scaffolded; see Status.)*
5. Chunk assignment, in three phases:
   - **Initial pass:** every entry's transitive static-import closure → `initial_<slug>`.
   - **Lazy pass:** every dynamic-import target and its static closure (minus anything already in an initial chunk) → `lazy_N`.
   - **Commons pass:** any module appearing in ≥ `commons_threshold` initial chunks gets promoted to `commons` and removed from its original chunks.

### Stage 3 — Transform & Emit

For each chunk:

1. TS-strip each alive module with SWC (types out, JSX → `React.createElement`).
2. Filter dead exports from the stripped output.
3. Flatten ESM scopes: rewrite local identifiers to chunk-unique names, drop intra-chunk imports, keep cross-chunk imports as ESM `import` statements.
4. Concatenate in topological order.
5. SHA-256 the bytes, name the file accordingly.

Write `chunks/`, then build `manifest.json` from the chunk metadata, then template out `index.html` from `entry_chunks`.

---

## Example: React 19 + TS SPA

Running `wundler build` against the `test-app/` fixture produces:

```
commons        (5 modules)  Button, validators, Icon, store, format
initial_root   (6 modules)  main, App, Navbar, Home, router, analytics
lazy_1         (1 module)   Dashboard.tsx
lazy_2         (1 module)   Settings.tsx
```

Why a commons chunk? `Button`, `validators`, `Icon`, `store`, and `format` are statically imported by `initial_root` *and* by both lazy chunks. With `commons_threshold = 2`, they get hoisted into `commons`, which loads once and serves all routes. The lazy chunks now contain only their unique work — `Dashboard.tsx` is one module, `Settings.tsx` is one module — and the initial paint pulls `commons + initial_root` (11 modules total) instead of 16.

---

## Status

This is a real bundler that produces correct output, and it is also early. Here's the honest scorecard.

### Working

- ✅ Full build pipeline end-to-end.
- ✅ TypeScript and TSX parsing + type-stripping via SWC.
- ✅ Static import graph with SCC-aware reachability.
- ✅ Dynamic `import()` detection → lazy chunks.
- ✅ Module-level dead-code elimination.
- ✅ Commons chunk extraction (configurable threshold).
- ✅ Content-hash-named output files with stable hashes.
- ✅ `manifest.json` + `index.html` generation.
- ✅ Dev server with on-demand TS→JS over Axum.
- ✅ SSE-based HMR with file watcher.
- ✅ Content-addressed local cache (`~/.wundler/cache/`).
- ✅ CJS detection and ESM stub generation.
- ✅ Rolldown adapter with SWC fallback.
- ✅ `validate-scale` Phase 1 throughput gate.
- ✅ ~60 unit and integration tests.

### Known gaps

- ⚠️ **Export-level DCE is scaffolded but not yet wired.** The call-edge data is collected and the traversal runs, but `Export` has no `alive` field yet, so the result isn't consumed by the emitter. Output today is module-granularity DCE only.
- ⚠️ **`export default function Foo()` scope-flattening bug.** An order-of-checks issue in `strip_imports_from_transpiled_js` mishandles default-exported function declarations. Workaround: assign to a named const first.
- ⚠️ **CJS interop is limited** to static `module.exports = { ... }` patterns. Dynamic CJS — conditional exports, runtime mutation of `module.exports` — does not round-trip.
- ⚠️ **Source maps are line-coarse.** The `sources` list is correct, but there are no VLQ column mappings yet, so stack traces resolve to the right file and roughly the right line.
- ⚠️ **Rspack engine is a stub.** `engine = "rspack"` is accepted by the config parser but currently falls back to the SWC engine.

### Not yet started

- CSS / asset graph (imports of `.css`, `.svg`, etc. are passed through but not bundled).
- Tree-shaking across re-exports beyond simple cases.
- Plugin API.
- A distributed cache layer for CI.

---

## License

TBD.
