# wundler

A continuously-maintained module graph and build system for large-scale JavaScript applications.

The module graph is the primary artifact. Bundles, dev servers, adaptive delivery, and PGO are derived views of that graph.

## Design

Three ideas from the linker world:

1. **ThinLTO decomposition.** Parse each module once into a compact summary (exports, imports, call edges, side effects). Cache content-addressed. Analyze summaries globally. Never reparse unchanged code. A `ModuleSummary` is ~2KB; the source file might be 50KB.

2. **mold parallelism.** The summarizer is a rayon parallel iterator over files. No shared mutable state. Graph analysis is fast because it operates on summaries, not IR.

3. **BOLT/Propeller profile-guided layout.** A co-request matrix collected from real browser sessions drives a C³ clustering pass that suggests chunk merges. The PGO store ingests ABS telemetry and rewrites the manifest in place. No rebuild.

The output is a `ChunkManifest`: a JSON document listing which modules belong in which chunk, their content hashes, load conditions (`Initial`, `Lazy`, `Prefetch`), and, after PGO, co-request scores and merge suggestions. Everything downstream — the production bundle, the dev server, the Adaptive Bundle Service, the Service Worker — is a consumer of the ChunkManifest.

## Stack

| Crate | Job |
|---|---|
| `wundler-core` | SWC-based module summarizer, 2-tier content-addressed cache, CJS→ESM stub generator |
| `wundler-graph` | BFS reachability, Tarjan SCC, route-based chunk splitting, ChunkManifest builder |
| `wundler-transform` | `TransformEngine` trait; `SwcTransformAdapter` (type strip + JSX + dead-export strip); `RolldownAdapter` (subprocess) |
| `wundler-pipeline` | `BuildPipeline` (summarize → analyze → transform → emit); dev server (Axum, on-demand SWC, SSE live reload) |
| `wundler-abs` | Adaptive Bundle Service: Axum server returning delta manifests, ed25519 signing, `TelemetryLogger`, bundled Service Worker |
| `wundler-pgo` | SQLite co-request matrix, JSONL ingestor, C³ clustering, `PgoHints` computation, atomic manifest updater |
| `wundler-cli` | All CLI subcommands |

## Quick start

```bash
# Build
wundler build --config wundler.toml

# Dev (live reload, on-demand SWC transform)
wundler dev --port 3000

# Sign the manifest for ABS verification
wundler abs keygen --signing-out signing.pem --verifying-out verifying.pem
wundler build --config wundler.toml --sign --key signing.pem

# Start the Adaptive Bundle Service
wundler abs serve --config abs.toml

# Ingest browser telemetry and apply PGO hints
wundler pgo ingest telemetry.jsonl --db pgo.sqlite
wundler pgo apply --db pgo.sqlite --manifest dist/manifest.json
```

## wundler.toml

```toml
[build]
root = "."
out_dir = "dist"
engine = "rolldown"      # rolldown | swc
source_maps = true
commons_threshold = 2    # modules in N+ chunks → commons

[entry]
"/" = "src/main.tsx"
"/dashboard" = "src/pages/Dashboard.tsx"
```

## CLI reference

```
wundler summarize <file>             Emit ModuleSummary as JSON
wundler analyze <dir>                Emit ChunkManifest as JSON
  --entry route=path
wundler build                        Summarize + analyze + transform + emit
  --config wundler.toml
  --engine rolldown|swc
  --sign --key <pem>                 Sign the manifest with ed25519
wundler dev                          Dev server (on-demand SWC, live reload)
  --port 3000

wundler abs keygen                   Generate ed25519 key pair
  --signing-out signing.pem
  --verifying-out verifying.pem
wundler abs serve                    Start ABS delta manifest server
  --config abs.toml

wundler pgo ingest <log.jsonl>       Load browser telemetry into SQLite
  --db pgo.sqlite
wundler pgo analyze                  Print C³ merge suggestions (no writes)
  --db pgo.sqlite
  --manifest dist/manifest.json
wundler pgo apply                    Apply PGO hints to manifest (atomic write)
  --db pgo.sqlite
  --manifest dist/manifest.json
wundler pgo status                   Show session count, readiness
  --db pgo.sqlite
```

## Why not Vite, esbuild, Rolldown

Those tools rebuild from source on every invocation. Wundler summarizes once per file change, caches content-addressed, and analyzes summaries. Graph analysis is O(changed files), not O(all files). At 50k modules the difference is seconds vs minutes.

The ABS is the other piece. It serves the minimum fetch set for a specific browser client. Not a static bundle, not the full manifest, but a delta computed from what the client already has in Cache Storage. Service Workers intercept navigations and call the ABS. The static CDN never changes. Only the manifest changes when PGO hints are applied.

## Test app

`test-app/` is a React 19 + TypeScript SPA with multiple lazy-loaded routes, dead exports (`CloseIcon`, `formatPhone`), and a side-effect module (`analytics.ts`). It was used to validate the full pipeline.

```bash
# Rolldown baseline
cd test-app && npm run build

# Wundler + rolldown backend
wundler build --config test-app/wundler-rolldown.toml
```

Both produce a working application in the browser.

## Documentation

See `docs/DEVELOPERS.md` for internals, schemas, the delta protocol, and contribution notes.
