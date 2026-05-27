# cloudpack roadmap

Upcoming work, grouped by category. These are directions, not promises. No dates. Each item is honest about what is missing today and why it matters.

## Security

### ABS authentication

The ABS `/manifest` endpoint is currently unauthenticated. Any client that can reach the network can POST a request, observe the response, and enumerate the chunk graph for any entry point. Production deployments should add:

- Bearer token verification, with either a static shared secret or a JWKS endpoint for rotating keys.
- An origin allowlist (CORS configuration plus per-origin rate limiting at a reverse proxy).
- mTLS between the ABS and internal services that act on behalf of users.

### Service Worker manifest signature verification

The SW currently trusts the manifest response from the ABS as long as the `build_id` matches the manifest cached at install time. A compromised ABS could serve a malicious manifest that points to attacker-controlled URLs at a different origin. The SW should verify the ed25519 signature of the manifest before using `fetch_urls`. The verifying key can be embedded in the SW source at build time.

### Subresource Integrity

The generated `index.html` should include `integrity="sha256-..."` attributes on every `<script>` tag so the browser refuses to execute tampered chunks. Cloudpack already has the content hashes; emitting them is a small change.

### Content Security Policy

The generated `index.html` should emit a CSP `<meta>` or HTTP header derived from the chunk hashes. There is currently no CSP support; consumers must configure their reverse proxy or hosting platform to add one.

### Rate limiting

The ABS `/manifest` endpoint has no rate limiting. A determined client can enumerate the full module graph by hammering it with different `entry_point` and `cached_hashes` combinations. Per-IP rate limiting with exponential backoff should be added at the ABS layer, not just at the reverse proxy, because the response shape depends on the request body in a way that proxies cannot rate-limit well.

## Performance

### True HMR

See `docs/superpowers/plans/future/true-hmr.md`. Replace `location.reload()` with react-refresh integration: per-module hot swap that preserves React state across edits. The SSE channel already carries enough information; the dev client needs a runtime that wraps each module in a refresh boundary.

### Dependency pre-bundling

Bare specifiers in dev mode are resolved via CDN import maps, which requires a network round-trip per dependency on cold start. Vite pre-bundles `node_modules` into a single ESM file on first start. Cloudpack should do the same: on the first `cloudpack dev` after a `package.json` change, scan all bare specifiers across the module graph, invoke rolldown to pre-bundle them into `node_modules/.cloudpack/`, and rewrite the import map to point at the pre-bundled file. Cache keyed on `package.json` hash.

### Compressed manifests

The `ChunkManifest` grows linearly with module count. For applications with 50k modules, the uncompressed JSON can exceed 5MB. The ABS should serve gzip- and brotli-compressed manifests and respect the client's `Accept-Encoding` header. The dev server should compress the static `manifest.json` too.

### Parallel PGO ingestor

The current ingestor reads JSONL line by line on a single thread and inserts each session inside its own SQLite transaction. For multi-gigabyte telemetry files this is slow. The ingestor should batch inserts under SQLite WAL mode and dispatch parsing across rayon threads.

### Incremental graph analysis

The graph analyzer currently re-runs reachability and SCC over the entire summary set when any module changes. The analyzer should track which summaries changed since the last build and recompute reachability only for the affected subgraph.

### ABS manifest hot reload

The ABS loads `manifest.json` once at startup and holds the parsed structure in memory. When `cloudpack pgo apply` writes a new manifest, the ABS does not pick up the change until restart. Adding inotify (Linux) and FSEvents (macOS) watchers to reload the manifest atomically on file change would eliminate the restart and let PGO updates take effect without downtime.

## Telemetry and observability

### Core Web Vitals in PGO

The PGO store currently tracks co-request patterns: which chunks are served together. The browser exposes richer signals via the Performance API:

- LCP (Largest Contentful Paint) per route. Tells you which lazy chunks are on the critical rendering path.
- FID and INP per route. Identify chunks that contribute to interaction-blocking work, worth prefetching aggressively.
- CLS. Identify chunks that inject layout-shifting elements; these benefit from being inlined or preloaded.

The ABS telemetry format should accept CWV readings alongside `chunks_served`, and the PGO store should be extended to weight clustering decisions by these signals.

### Error rate per chunk

If a chunk fails to evaluate — syntax error, network error, integrity mismatch — the SW should report the failure to the ABS. A chunk with a 5%+ error rate is a deployment problem and should raise an alert.

### Prometheus / OpenTelemetry export

The ABS should expose `/metrics` in Prometheus format. Useful counters and histograms:

- `cloudpack_manifest_requests_total{entry_point, status}`
- `cloudpack_chunks_served_total{chunk_id}`
- `cloudpack_cache_hit_rate{entry_point}`
- `cloudpack_delta_size_bytes` (histogram of bytes returned per response)

OpenTelemetry export would let users push these into existing observability stacks without scraping.

### Build size tracking

`cloudpack build` should emit `build-stats.json` alongside the manifest, containing per-chunk sizes (raw and gzipped), total bundle size, and a diff against the previous build's stats file. CI can fail the build when total size grows by more than a configured threshold.

## Production readiness

### ABS high availability

Today a single ABS instance is the deployment model. In production, you want multiple ABS replicas behind a load balancer. The manifest is loaded from disk, so as long as every replica points at the same `manifest_path` — NFS, shared volume, or object storage with a sidecar that syncs to local disk — they serve consistent deltas. Telemetry needs more thought: a single JSONL file does not work across replicas. The log should fan out to a centralized sink (Kafka, Kinesis, S3) and `cloudpack pgo ingest` should consume from there.

### Manifest versioning and rollback

When a new build is deployed, the old manifest should be archived rather than overwritten. The ABS should support a `?version=` query parameter or a `/manifest/v2` URL to serve a specific build's manifest. This enables:

- Canary rollouts. Serve v2 to 10% of users by session hash; the other 90% continue to receive v1.
- Immediate rollback. Re-point the ABS at the previous manifest without redeploying chunks.

### Blue/green deployment

The ABS should serve two manifests simultaneously: the old build and the new one. The `build_id` cached in the SW determines which version the SW asks for. New users receive the new manifest; existing sessions continue with their cached version until they re-fetch after the TTL expires. The current single-manifest model forces a hard cutover.

### CDN integration details

The expected CDN configuration is currently scattered across `DEVELOPERS.md` and the test app. It should be documented in one place:

- Chunks are content-hashed and never change. Set `Cache-Control: public, max-age=31536000, immutable`.
- `index.html` is mutable. Set `Cache-Control: no-cache`.
- `manifest.json` is served by the ABS, not the CDN. Do not cache it at the CDN layer.

### Graceful degradation without Service Worker

The generated `index.html` requires a working Service Worker. In private browsing mode in some browsers, and in environments where SW registration fails for unrelated reasons, the page does not render. The `index.html` should include a no-SW fallback that loads all initial chunks directly via `<script type="module">` tags. The delta protocol's benefits are unavailable in that mode, but the page works.

## Developer experience

### Bundle analysis viewer

`cloudpack build --analyze` should open a treemap visualization showing module sizes, chunk membership, dead code, and suggested merges. Comparable to `rollup-plugin-visualizer` and webpack-bundle-analyzer. The data is already in the `ChunkManifest`; this is a UI task.

### VS Code extension

Surface cloudpack diagnostics inline. Greyed-out unused exports, strikethrough unreachable modules, status-bar chunk membership for the active file. The `cloudpack analyze` JSON output is enough to drive the extension.

### CI size budget enforcement

A `[budget]` section in `cloudpack.toml`:

```toml
[budget]
initial_bundle_kb = 200     # fail build if initial chunks > 200KB gzipped
lazy_chunk_kb = 50          # fail build if any lazy chunk > 50KB gzipped
new_module_budget_kb = 5    # warn if a new module adds > 5KB to any chunk
```

The build pipeline knows every chunk's compressed size; enforcing budgets is a check at emit time.

### Rspack adapter

Add `RspackAdapter` alongside `RolldownAdapter` as a third `TransformEngine`. The `ChunkManifest` from cloudpack's graph analysis can drive rspack's `optimization.splitChunks` config. Useful for teams already invested in the rspack and webpack plugin ecosystem who want cloudpack's incremental summarization and PGO without abandoning their loader stack.
