# cloudpack configuration reference

Complete reference for every configuration file and tunable in cloudpack. Fields are documented with their type, default, and behavior. Examples are runnable as written.

## cloudpack.toml

The build configuration. Passed to every cloudpack subcommand via `--config cloudpack.toml`. All fields under `[build]` are read by `BuildConfig`; the `[entry]` table maps route names to entry module paths.

```toml
[build]
# Required: source root — directory cloudpack walks for .ts/.tsx/.js/.jsx
root = "src"

# Required: output directory for built chunks, manifest.json, index.html
out_dir = "dist"

# Transform engine. Options: "rolldown" (recommended) | "swc"
# rolldown: invokes rolldown as a subprocess; handles all JS edge cases correctly
# swc: pure-Rust scope-flattening; faster cold start, limitations with complex defaults
engine = "rolldown"

# Number of chunks a module must appear in before being extracted to a shared commons chunk
# Increase if you have many fine-grained lazy routes sharing few modules
commons_threshold = 2

# Emit .js.map source maps alongside each chunk
source_maps = false

[entry]
# Map of route names to entry module paths (relative to workspace root)
# Each entry creates an initial chunk + any lazy splits from dynamic import()
"/" = "src/main.tsx"
"/dashboard" = "src/pages/Dashboard.tsx"
```

### Field details

**`[build] root`** (required, string). The directory walked by the summarizer. All entry-point paths are resolved relative to the workspace root, but `root` constrains which files are considered candidates for inclusion in the module graph. Files outside `root` are reachable only via explicit import.

**`[build] out_dir`** (required, string). Where `manifest.json`, `index.html`, and the `chunks/` directory are written. Created if it does not exist. The directory is not cleaned between builds — content-hashed chunk filenames make stale files harmless, but a periodic `rm -rf dist/chunks` is reasonable hygiene.

**`[build] engine`** (string, default `"rolldown"`). The `TransformEngine` to use. `"rolldown"` invokes rolldown as a subprocess and produces correct output for every JavaScript pattern cloudpack has been tested with. `"swc"` uses `SwcTransformAdapter`, which is faster on cold start but performs per-chunk scope-flattening in pure Rust; it produces incorrect output for some complex re-export and default-aliasing patterns. Choose `swc` only when you have validated it against your codebase.

**`[build] commons_threshold`** (integer, default `2`). The module-sharing threshold for commons extraction. A module reachable from N or more route entries is hoisted to a shared chunk. Raising this to 3 or higher reduces commons-chunk size at the cost of per-route duplication. Lowering to 1 disables commons extraction (every shared module is duplicated).

**`[build] source_maps`** (boolean, default `false`). Emit `.js.map` alongside each chunk. Increases build time and `out_dir` size. Recommended `true` for production builds where errors are reported with stack traces.

**`[entry]`** (required, table). Maps route paths to entry module file paths. Each entry produces one initial chunk per route plus any lazy chunks reached via `import()`. The route names are arbitrary strings; the ABS uses them as the keys of `entry_chunks` in the manifest. Use the URL path of the route for clarity.

## abs.toml

Configuration for `cloudpack abs serve`. Mapped 1:1 to the `AbsConfig` struct in `crates/cloudpack-abs/src/server.rs`.

```toml
# Path to the ChunkManifest JSON produced by cloudpack build
manifest_path = "dist/manifest.json"

# Base URL the Service Worker uses to construct fetch_urls
# In production: your CDN origin (https://cdn.example.com)
# In development: local file server (http://localhost:4006)
cdn_base_url = "https://cdn.example.com"

# Path to write telemetry JSONL (one line per /manifest request)
# Omit to disable telemetry
telemetry_log = "/var/log/cloudpack/telemetry.jsonl"

# Port for the ABS HTTP server
port = 4500

# Cache-Control max-age for delta manifest responses (seconds)
# Clients re-request after TTL expires
ttl_seconds = 300

# Optional: path to ed25519 signing key PEM for manifest verification
# If set, the server refuses to start if manifest.json has no valid .sig sidecar
# signing_key_pem = "signing.pem"
```

### Field details

**`manifest_path`** (string, default `"dist/manifest.json"`). The `ChunkManifest` the ABS serves deltas against. Loaded once at startup. If the file changes on disk, the ABS does not currently hot-reload; restart the process. The path must be readable by the ABS process user.

**`cdn_base_url`** (string, default `"https://cdn.example.com"`). Prefix used to construct every URL returned in `fetch_urls` and `prefetch_urls`. The ABS does not serve chunk content — only delta manifests. The CDN must mirror the `dist/chunks/` directory layout.

**`telemetry_log`** (string, default `"/tmp/cloudpack-telemetry.jsonl"`). Append-only JSONL log. One line per `POST /manifest` request. Consumed by `cloudpack pgo ingest`. To disable telemetry entirely, point this at `/dev/null`. The directory must exist and be writable.

**`port`** (integer, default `8080` in `AbsConfig::default()`, frequently overridden to `4500` in deployment). TCP port. Bind address is `0.0.0.0` — front the ABS with a reverse proxy if you need TLS, rate limiting, or authentication.

**`ttl_seconds`** (integer, default `300`). The `Cache-Control: max-age` value the ABS attaches to delta manifest responses. The Service Worker re-requests the manifest after this TTL. Lower values make PGO updates visible faster at the cost of more requests; higher values reduce ABS load. 300 is a reasonable starting point.

**`signing_key_pem`** (optional string, default unset). If set, the ABS expects `manifest.sig` next to `manifest.json` and verifies the ed25519 signature at startup using the corresponding verifying key. The signing key PEM is read to derive the public key; it is never used to sign anything at the ABS. Unset means the ABS accepts any manifest on disk.

## C³ cluster configuration

The Conditional Co-request Clustering pass that drives PGO merge suggestions. Tuned by `C3Config` in `crates/cloudpack-pgo/src/clustering.rs`. There is currently no `[pgo]` section in any TOML file; tuning is done by passing arguments to `compute_clusters`. A future release will expose these as CLI flags on `cloudpack pgo apply`.

| Field | Type | Default | Meaning |
|---|---|---|---|
| `merge_threshold` | float | `0.70` | `P(b\|a)` AND `P(a\|b)` must both exceed this for a merge candidate |
| `max_merge_modules` | int | `500` | Skip merge if combined module count would exceed this |
| `min_sessions` | int | `100` | `cloudpack pgo status` shows `NOT READY` below this |

### Field details

**`merge_threshold`** (float, default `0.70`). The two-way conditional probability that two chunks are loaded in the same session. `P(b|a)` is the probability of loading chunk `b` given chunk `a` was loaded, and vice versa. The pass requires *both* to exceed the threshold so it does not merge an always-loaded commons chunk into every chunk that imports it. Higher values produce fewer, more confident merges. `0.70` is calibrated against real traffic. `0.90` or higher is appropriate for synthetic sessions, where co-request probabilities saturate near 1.0 and a lower threshold would over-merge.

**`max_merge_modules`** (int, default `500`). A safety floor. The greedy assignment pass refuses to combine two chunks whose total module count would exceed this. Without it, transitively-chained merges can produce a single chunk that contains most of the application, defeating the purpose of code splitting. `500` is conservative for most React applications; raise it if you have evidence the cap is preventing useful merges.

**`min_sessions`** (int, default `100`). Statistical floor below which `compute_clusters` returns no suggestions. Co-request probabilities computed from fewer than 100 sessions are noise. For synthetic bootstrapping during development you may want to lower this to 10 or 20; in production keep it at 100 or higher.

## Dev server details

The dev server is started by `cloudpack dev --port 3000`. There is no configuration file — all behavior is hard-coded with command-line flag overrides.

### Generated index.html

The dev server synthesizes `index.html` per request to `GET /`. The document contains:

1. An `<script type="importmap">` block listing every bare specifier in the entry module's transitive imports, resolved against a CDN (esm.sh by default).
2. A `<script type="module">` tag for the HMR client at `/__cloudpack__/hmr-client.js`.
3. A `<script type="module">` tag for the entry module configured in `cloudpack.toml` `[entry]`.

The HMR client opens an SSE connection to `/__cloudpack__/hmr` and calls `location.reload()` on any `event: change` message whose payload matches a path the page imports.

### Extension resolution

Requests for module paths are resolved in this order. The first match wins.

1. The exact path as requested (`./foo` → `./foo`).
2. `.tsx`
3. `.ts`
4. `.jsx`
5. `.js`
6. `/index.tsx`
7. `/index.ts`
8. `/index.jsx`
9. `/index.js`

If none match, the server returns 404. This is the only place cloudpack performs filesystem extension resolution; production builds resolve at summarization time and embed the resolved paths in the `ModuleSummary`.

### Bare specifiers in dev

`node_modules` is **not** bundled by the dev server. Bare specifiers (`react`, `react-dom`, `@radix-ui/...`) are emitted in the import map and resolved by the browser against the configured CDN. This avoids running rolldown on every cold start of the dev server, at the cost of a network round-trip per dependency on the first request. See `ROADMAP.md` for the dependency pre-bundling proposal.

### Transform behavior

Every request for a `.tsx`, `.ts`, `.jsx`, or `.js` file inside the workspace runs the full SWC transform synchronously. There is no transform cache in dev mode. This is intentional: it keeps the dev server stateless and ensures every reload reflects the exact source on disk. The transform cost is below 10ms per file for typical TypeScript modules; an entire page load is bounded by the longest dependency chain, not the total file count, because the browser issues parallel requests.
