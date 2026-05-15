# Observability

> Build-time visibility first. Runtime chunk-error visibility next. Prometheus export third. Web Vitals last — **gated**, because PGO optimizing against an incomplete runtime signal is a footgun.

**Status:** Design.
**Depends on:** `versioned-runtime-control.md` (provides deterministic `build_id` used in `build-stats.json` and `/metrics` labels).
**Blocks:** Performance work (cannot tune what you cannot see), Web Vitals adoption in PGO (gated on Priority 2 being live).
**System types:** data pipeline (build artifact), web service / API (`/metrics`, `/telemetry/*`), edge / offline-first (SW collection), CLI tool (`wundler build` exit code), philosophy: Linux/Unix (mechanism, not policy — alerts live in the caller).

---

## 1. ANALYZE — System Map

### 1.1 Goals

| ID | Goal |
|---|---|
| G1 | `wundler build` emits a machine-readable `build-stats.json` next to `manifest.json` on every successful build |
| G2 | A `[budget]` section in `wundler.toml` makes the build exit non-zero with an actionable message when configured thresholds are exceeded |
| G3 | The Service Worker reports chunk load/eval/integrity errors to ABS over a fire-and-forget endpoint that **never** blocks chunk delivery |
| G4 | ABS accumulates labelled in-process counters and exposes them on `GET /metrics` in Prometheus text format |
| G5 | The build-stats JSON contains everything Scale Benchmark Foundation's V5 stability check needs to flag drift across runs |
| G6 | Web Vitals collection in PGO is **gated** behind G3+G4 being live and producing signal, not shipped speculatively |
| G7 | Existing CI behavior is preserved — budgets are opt-in; a missing `[budget]` section produces no exit-code change |

### 1.2 Actors / Interfaces

```
       ┌──────────────────────────────────────┐
       │  wundler build (CLI)                  │
       │  ┌──────────────────────────────────┐ │   build-stats.json
       │  │ wundler-pipeline ── BuildOutput ─┼─┼──▶  out_dir/
       │  └──────────────────────────────────┘ │
       │  budget check ── exit 0 / exit 1 ─────┼──▶  stderr
       └──────────────────────────────────────┘
                                                       │
                                              CI reads ▼
                                              ┌──────────────────┐
                                              │ build-stats.json │
                                              └──────────────────┘
                                                       │
                                  diff vs baseline ────┘
                                                       ▼
                                              PR comment / fail

  Browser
  ┌────────────────────────┐   POST /telemetry/chunk-error   ┌──────────────┐
  │ Service Worker         ├──── fire-and-forget ───────────▶│              │
  │  - install/fetch errs  │                                  │     ABS      │
  │  - integrity_mismatch  │   POST /telemetry/web-vitals    │              │
  │  - eval_failed         ├──── fire-and-forget ───────────▶│  in-process  │
  └────────────────────────┘   (GATED, Priority 4)           │   counters   │
                                                              │              │
                                                              │              │
  Prometheus / curl ◀───────────────── GET /metrics ──────────┤              │
  PGO ingestor      ◀───────────────── JSONL tail ────────────┤  telemetry   │
                                                              │   .jsonl     │
                                                              └──────────────┘
```

### 1.3 What exists today

The pattern from the prior three designs holds: **the scaffolding is already there.** Concrete inventory, verified by reading the source:

| What | Where | Status / Gap |
|---|---|---|
| `BuildStats { total_modules, alive_modules, dead_modules, chunks_written, build_time_ms, largest_chunk_bytes }` | `crates/wundler-pipeline/src/pipeline.rs:27-40` | Struct exists, returned from `BuildPipeline::build()` in `BuildOutput`. Currently **only consumed by stdout printer at `crates/wundler-cli/src/main.rs:372`.** Not serialised. |
| `BuildOutput { manifest, chunk_files, stats }` | `crates/wundler-pipeline/src/pipeline.rs:44-51` | Already carries `chunk_files: Vec<PathBuf>` — sufficient to derive per-chunk sizes without re-walking `out_dir`. |
| `AnalysisStats { total, alive, dead, chunks }` | `crates/wundler-graph/src/analyzer.rs:59-68` | Already populated by `GraphAnalyzer::analyze`. Subset of `BuildStats`; no new code needed. |
| `output::write_manifest` | `crates/wundler-pipeline/src/output.rs` | The pattern to follow: a sibling `write_build_stats` slots in next to it without touching `pipeline.rs` orchestration. |
| `TelemetryLogger` (append-only JSONL, `Arc<Mutex<BufWriter>>`, thread-safe, clones share writer) | `crates/wundler-abs/src/telemetry.rs` | Already accepts arbitrary `TelemetryEvent`. Can carry new event kinds via an `enum` discriminant; **must remain backward-compatible with `wundler-pgo`'s deserializer.** |
| `TelemetryEvent { session_id, entry_point, chunks_served, client_had, timestamp_ms }` | `crates/wundler-abs/src/types.rs:81-96` | **Does NOT currently carry `build_id` as a typed field.** The parent context's claim that `last_seen_build_id` already exists is incorrect. The handler at `crates/wundler-abs/src/server.rs:200-203` shoves `req.build_id` into the `session_id` field — this is a latent bug (named field carries semantically wrong data) that this design **does not silently fix**; see §5.2 contract note. |
| `wundler-pgo::ingestor::TelemetryEvent` (private deserialize-only mirror) | `crates/wundler-pgo/src/ingestor.rs:32-41` | **Critical compatibility constraint.** It tolerates unknown fields (no `deny_unknown_fields`), so additive JSON fields are safe. Field renames or new required fields will break PGO. |
| `notifyBuildIdChanged(newBuildId)` in SW | `crates/wundler-abs/assets/sw.js:62-72` | Already postMessages clients. Hook point for SW → ABS chunk-error reporting lives **immediately around the `fetch(url)` / `cache.match(url)` calls at lines 128-148** — that block currently has `catch {}` returning `null` silently; it is the exact ground zero for chunk-error reporting. |
| ABS HTTP server | `crates/wundler-abs/src/server.rs` | Three routes: `/manifest`, `/health`, `/sw.js`. **No `/metrics`. No `/telemetry/*`.** Telemetry currently piggybacks on the `/manifest` handler (only events that imply a successful manifest fetch ever get recorded). |
| PGO store + ingestor | `crates/wundler-pgo/src/` | Reads the same JSONL `TelemetryLogger` writes. Already idempotent (`sessions_skipped_duplicate`). Web Vitals events would extend `SessionRecord` shape — see Priority 4 gate. |

### 1.4 Failure modes WITHOUT this design

| ID | Failure | Blast radius | Today's detection |
|---|---|---|---|
| F1 | Bundle silently grows 40% across two PRs | All users — page load regression that compounds | None until a user complains |
| F2 | A chunk hash collision / CDN misconfig produces `integrity_mismatch` storms | Cohort of users on a stale SW | None — SW swallows the error with `catch {}` |
| F3 | A specific chunk fails to evaluate (`SyntaxError` from a transform regression) | Every user hitting that route | None — `eval_failed` is never reported anywhere |
| F4 | Manifest TTL mistuning causes 100× the expected request rate | ABS capacity, CDN bill | None — no `manifest_requests_total` counter exists |
| F5 | A deploy produces `build-stats.json` that differs from a previous baseline only in chunk ordering (non-determinism leak) | Cache effectiveness; CDN egress | Caught by Scale Benchmark Foundation's V5 check **only if `build-stats.json` exists and is stable** |
| F6 | PGO ingests Web Vitals data but cannot correlate poor LCP with chunk load failures (because chunk errors aren't reported) | PGO optimises against noise, recommends wrong chunk splits | **This is the COE's footgun.** Latent; surfaces only after PGO has shipped bad hints |

### 1.5 The dependency chain — why this order

```
  Priority 1: build-stats.json + budget check
              │
              │ provides:  stable per-build artifact, deterministic build_id
              │            (the SCA build_id from Versioned Runtime Control)
              ▼
  Priority 2: SW → POST /telemetry/chunk-error, in-process counters
              │
              │ provides:  ground-truth runtime error visibility per build_id
              │            (without this, every other runtime signal is suspect)
              ▼
  Priority 3: GET /metrics (Prometheus text)
              │
              │ provides:  external scrape surface; the counters Priority 2
              │            already wrote into in-process now have a readable
              │            format. NO new measurement; just exposition.
              ▼
  Priority 4: Web Vitals — GATED
              Must NOT land until Priority 2 is producing signal in
              production, because: a poor LCP that is actually a chunk
              load failure looks identical to a poor LCP that is genuinely
              a layout problem — and PGO will recommend the wrong fix.
```

**Why Priority 1 is first:** zero runtime infrastructure needed. The pipeline already returns `BuildStats`. Wiring it to disk + adding a TOML threshold check is local to two crates (`wundler-pipeline`, `wundler-cli`) and produces immediate, visible value (CI signal). It also unblocks Scale Benchmark Foundation V5, which is independently waiting.

**Why Priority 2 before Priority 3:** Prometheus format is exposition over the *same counters*. Building Priority 3 first means inventing exposition for counters with no producer — a contract test with no implementation. Build the producer (Priority 2), then the format (Priority 3).

**Why Priority 4 is gated, not just last:** Web Vitals data through PGO becomes a directed action on chunking. If it lands while chunk-error data is missing, every "poor LCP route" looks like a chunking problem when many will actually be `load_failed`/`integrity_mismatch` problems invisible to PGO. The COE was explicit: "PGO based on incomplete runtime telemetry is a footgun." See §6 for the precise gate condition.

---

## 2. DESIGN — Candidates

### 2.1 Priority 1: build-time artifact + budget

#### C1 — Serialize existing `BuildStats` as-is

A new file `wundler-pipeline/src/output.rs::write_build_stats` derives `Serialize` on `BuildStats` and writes `out_dir/build-stats.json`. No new fields. No budget check.

| Dimension | C1 |
|---|---|
| Components | 1 new function in `output.rs`; 1 line of `#[derive(Serialize)]` |
| Complexity | Trivial |
| Operability | Minimum — file appears; nothing fails on absence |
| Coupling | None new |
| Evolvability | Field additions later require schema versioning |
| Failure modes | F1, F5 still possible — no enforcement |
| Cost | ~10 LoC |
| What it sacrifices | Budget enforcement, per-chunk visibility, build_id linkage, baseline diffing |

#### C2 — Extended schema + budget + build_id linkage   **(RECOMMENDED)**

Adds:

1. A `BuildStatsArtifact` wire type in `wundler-pipeline/src/build_stats.rs` distinct from the in-memory `BuildStats` (separation of concerns — internal struct vs file format).
2. Per-chunk breakdown derived from `BuildOutput.chunk_files` (size from `fs::metadata`, chunk-id resolution from the manifest in the same `BuildOutput`).
3. `build_id` field populated from `BuildOutput.manifest.build_id` (deterministic from VRC SCA).
4. `total_bundle_bytes` (sum of all chunks).
5. `previous_build` diff (optional, present iff `out_dir/build-stats.json` exists from the prior run — read **before** the new file is written, hold in memory, then overwrite).
6. `[budget]` table in `wundler.toml` (`initial_bundle_max_bytes`, `lazy_chunk_max_bytes`, `total_bundle_max_bytes`); a new `budget` module in `wundler-pipeline/src/budget.rs` consumes `BuildStatsArtifact` and returns `Result<(), BudgetViolation>`; CLI maps that to `exit(1)` with the actionable message.

File locations (all in `crates/wundler-pipeline/`):

```
src/
├── build_stats.rs   # BuildStatsArtifact, ChunkRecord, BudgetSection — Serialize
├── budget.rs        # check_budget(&BuildStatsArtifact, &BudgetSection) -> Result<()>
├── output.rs        # +write_build_stats(out_dir, &BuildStatsArtifact)
└── pipeline.rs      # call site at end of build() — additive, no signature change
```

The CLI does:

```rust
let out = pipeline.build()?;
let stats = BuildStatsArtifact::from_build(&out, previous);
output::write_build_stats(&out_dir, &stats)?;
if let Some(budget) = config.budget {
    if let Err(v) = budget::check(&stats, &budget) {
        eprintln!("{}", v.actionable_message());
        std::process::exit(1);
    }
}
```

| Dimension | C2 |
|---|---|
| Components | 3 new files, 1 added call site, 1 new TOML section |
| Complexity | Low — pure functions, no I/O outside `output.rs` |
| Operability | Good — exit code + stderr message; no silent breakage (budget is opt-in) |
| Coupling | `wundler-pipeline` ↔ `BuildOutput` (already exists); no new cross-crate dependency |
| Evolvability | Schema versioned (`schema_version: "1"`); additive fields without breaking older diff consumers |
| Failure modes addressed | F1 (size growth), partial F5 (size+chunk count surfaced; ordering left to V5) |
| Cost | ~150 LoC + tests |
| What it sacrifices | No baseline-pinning mechanism (C3 adds that); no human-readable side artifact (just JSON) |

#### C3 — C2 + pinned baseline + PR diff helper

C2 plus:

- A `wundler-bench`-adjacent `wundler bench-stats diff <baseline.json> <head.json>` subcommand that produces a structured markdown table for PR comments.
- A `[budget.baseline]` knob pointing to a checked-in baseline JSON; budget violations express as deltas ("+38 KB lazy chunk vs baseline") rather than absolutes.

| Dimension | C3 |
|---|---|
| Components | C2 + new bench subcommand + baseline pinning |
| Complexity | Medium — introduces a workflow question (where does the baseline live? when does it update?) |
| Operability | Best — PR-native UX |
| Coupling | Couples build artifact to CI / VCS workflow conventions |
| Evolvability | High but workflow-dependent |
| Failure modes addressed | F1 with finer granularity |
| Cost | ~300 LoC + workflow docs |
| What it sacrifices | Time-to-first-value (C2 lands in days, C3 in weeks); operational simplicity |

**Recommendation: C2.** It is the smallest design that closes F1, F5-size, and unblocks Scale Benchmark Foundation V5. C3's PR workflow can be built on top of C2's artifact later without retrofitting.

---

### 2.2 Priority 2: runtime chunk-error reporting

#### C1 — `console.error` only

SW logs chunk load/eval failures to the browser console. Nothing leaves the device.

| Dimension | C1 |
|---|---|
| Components | 0 server, ~10 LoC SW |
| Visibility | Per-device only — useless in production |
| Sacrifices | F2, F3, F4 all unaddressed |

#### C2 — SW fire-and-forget POST + in-process counters   **(RECOMMENDED)**

**SW side** (`crates/wundler-abs/assets/sw.js`, modify the install/fetch handlers and the `catch {}` blocks at lines 84-86, 113-114, 137-148):

```js
function reportChunkError(absBaseUrl, payload) {
  // Fire-and-forget. Never await. Never block the calling fetch.
  try {
    void fetch(`${absBaseUrl}/telemetry/chunk-error`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
      keepalive: true,   // survives navigation
    }).catch(() => {});  // explicit swallow
  } catch {}
}
```

Error types reported:

| `error_type` | When | SW signal |
|---|---|---|
| `load_failed` | `fetch(url)` rejects or returns `!ok` | `catch` block in `handleNavigation` |
| `network_timeout` | `AbortController` fires after timeout | already exists in `fetchDelta`, extend to chunk fetches |
| `eval_failed` | (Future) Page-side reporter posts to SW via `postMessage` | Out of scope for SW alone — see §5.3 |
| `integrity_mismatch` | SRI check fails (depends on Security Baseline Phase 2) | Conditional — only when SRI is in effect |

Payload shape:

```jsonc
{
  "build_id": "9c1a...",       // from cache's static manifest
  "chunk_id": "main-a1b2c3",   // resolved from URL → chunk-id mapping
  "url": "https://cdn.example.com/chunks/a1b2c3d4.js",
  "error_type": "load_failed",
  "timestamp_ms": 1715774400000,
  "session_id": "uuid-v4"      // same per-tab UUID as existing telemetry
}
```

**ABS side** — new module `crates/wundler-abs/src/metrics/`:

```
src/metrics/
├── mod.rs        # pub struct Metrics; pub fn new() -> Arc<Metrics>
├── counters.rs   # AtomicU64-backed labelled counters
├── chunk_error.rs # POST /telemetry/chunk-error handler; updates counters + JSONL
└── prometheus.rs # exposition for Priority 3 (lives here, wired in P3)
```

`Metrics` is held in `RouterState` alongside `app` and `telemetry`:

```rust
#[derive(Clone)]
struct RouterState {
    app: AppState,
    telemetry: TelemetryLogger,
    metrics: Arc<Metrics>,
}
```

The counters are a fixed shape:

```rust
pub struct Metrics {
    manifest_requests_total: AtomicU64,
    manifest_bytes_total: AtomicU64,
    chunk_errors: DashMap<ChunkErrorKey, AtomicU64>,
    cache_hits_total: AtomicU64,
    cache_misses_total: AtomicU64,
}
struct ChunkErrorKey { build_id: String, chunk_id: String, error_type: ErrorType }
```

`DashMap` cardinality is bounded by `(unique_build_ids_seen × unique_chunk_ids × 4_error_types)`. With VRC retention bounding archived build_ids to a small number (default 5), this is bounded. The retention contract from VRC carries through: when a build_id ages out of the archive, its counters can be reaped (a `prune(active_build_ids: &HashSet<String>)` method).

The `/telemetry/chunk-error` handler also appends a JSONL event to `TelemetryLogger` so PGO can ingest it. **Schema-additive:** new event with discriminator field; PGO ignores unknown event kinds. See §5.1.

| Dimension | C2 |
|---|---|
| Components | 1 SW patch, 1 new ABS module (~4 files), 1 new route, 1 new event variant |
| Complexity | Low-medium |
| Operability | Good — restart loses counters, but JSONL persists; doc this as expected |
| Coupling | New dependency: `dashmap`. Pulls one well-trodden crate; no SDK |
| Evolvability | High — new error types are additive enum variants |
| Failure modes addressed | F2, F3, partial F4 (manifest counters trivially included) |
| Cost | ~250 LoC + SW changes + tests |
| What it sacrifices | Per-restart counter loss; no offline buffering (C3 adds) |

#### C3 — C2 + SW batched queue with `sessionStorage` persistence

Adds a small SW-side queue that batches events and persists across reloads via `sessionStorage`.

| Dimension | C3 |
|---|---|
| Components | C2 + ~80 LoC SW + tests |
| Complexity | Medium |
| Operability | Resilient across page reloads |
| Sacrifices | Time-to-value; complexity in the SW which is the highest-risk component |

**Recommendation: C2.** Fire-and-forget with `keepalive: true` already survives most navigation cases. Persistent batching is an optimisation; ship visibility first.

---

### 2.3 Priority 3: Prometheus exposition

#### C1 — Custom JSON `/stats` endpoint

`GET /stats` → `{ manifest_requests_total: 12345, ... }`.

| Dimension | C1 |
|---|---|
| Compatibility | Bespoke — no Prometheus/Grafana/Datadog integration |
| Sacrifices | Industry-standard scraping; alerting tooling |

#### C2 — Prometheus text format `/metrics`, no SDK   **(RECOMMENDED)**

Hand-roll the exposition format. It is ~50 lines of `format!` calls. The format is stable, well-documented, and the *only* thing the SDK does for counters at this scale is provide a registry and locking primitives we already have.

`crates/wundler-abs/src/metrics/prometheus.rs`:

```rust
pub fn render(metrics: &Metrics) -> String {
    let mut s = String::new();
    writeln!(s, "# HELP wundler_manifest_requests_total Manifest requests served.").unwrap();
    writeln!(s, "# TYPE wundler_manifest_requests_total counter").unwrap();
    writeln!(s, "wundler_manifest_requests_total {}", metrics.manifest_requests_total.load(Ordering::Relaxed)).unwrap();
    // ... chunk_errors with labels ...
    for entry in metrics.chunk_errors.iter() {
        let k = entry.key();
        writeln!(
            s,
            r#"wundler_chunk_errors_total{{build_id="{}",chunk_id="{}",error_type="{}"}} {}"#,
            escape(&k.build_id), escape(&k.chunk_id), k.error_type.as_str(),
            entry.value().load(Ordering::Relaxed),
        ).unwrap();
    }
    s
}
```

Route:

```rust
.route("/metrics", get(|State(s): State<RouterState>| async move {
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], render(&s.metrics))
}))
```

| Dimension | C2 |
|---|---|
| Components | 1 file, ~100 LoC |
| Dependencies | Zero new crates |
| Compatibility | Full — `prometheus`, `victoriametrics`, `grafana-agent`, `datadog-agent`, `vector`, every Prom-compatible scraper works |
| Sacrifices | No histograms (not needed yet); manual label escaping (test-covered) |

#### C3 — OTel SDK + OTLP export + Prometheus bridge

Full OpenTelemetry Rust SDK.

| Dimension | C3 |
|---|---|
| Components | `opentelemetry`, `opentelemetry-otlp`, `opentelemetry-prometheus`, exporter pipeline |
| Complexity | High — SDK lifecycle, batch exporter, async runtime hooks |
| Sacrifices | Operational simplicity; binary size; "thin counter struct" principle |

**Recommendation: C2.** The constraint is explicit: *no external collector, no OTel SDK unless very thin.* The thinnest thing is also the simplest: hand-rolled exposition. We can adopt OTel later if a use case demands traces — counters do not.

---

### 2.4 Priority 4: Web Vitals — GATED

Web Vitals (LCP, CLS, INP) require a page-side reporter that posts to the SW or directly to ABS via `POST /telemetry/web-vitals`. The PGO ingestor learns a new event variant. **No design choice is being made here** — the design choice is the gate (§6). When the gate opens, the implementation is mechanically analogous to Priority 2 chunk-error reporting: same fire-and-forget pattern, same Metrics struct, same JSONL append for PGO.

---

## 3. RECOMMENDATIONS

| Priority | Pick | One-line rationale |
|---|---|---|
| P1 — build-stats + budget | **C2** | Smallest design that closes "bundle silently grew", surfaces per-chunk size, and feeds Scale Benchmark Foundation V5. Budget is opt-in so CI doesn't break silently. |
| P2 — chunk errors | **C2** | Fire-and-forget + `keepalive: true` + in-process counters. Persistent SW queue (C3) is a refinement, not a prerequisite. |
| P3 — Prometheus | **C2** | Hand-roll the text format. ~100 LoC, zero new SDK dependencies, full ecosystem compatibility. |
| P4 — Web Vitals | **Gated** | See §6. |

---

## 4. `build-stats.json` SCHEMA

**File:** `<out_dir>/build-stats.json`
**Schema version:** `"1"` (additive evolution only; rename or remove = bump)
**Stability contract:** for a fixed input tree and `wundler.toml`, two consecutive `wundler build` invocations on the same host produce byte-identical `build-stats.json` (modulo the `timing` block and `previous_build`). This is the V5 contract.

```json
{
  "schema_version": "1",
  "build_id": "9c1a7f0e2d3b...",
  "wundler_version": "0.1.0",
  "generated_at": "2026-05-15T12:30:12Z",

  "summary": {
    "total_modules": 36042,
    "alive_modules": 35110,
    "dead_modules": 932,
    "chunks_written": 247,
    "total_bundle_bytes": 8421376,
    "largest_chunk_bytes": 412800,
    "initial_bundle_bytes": 612480
  },

  "timing": {
    "build_time_ms": 4821,
    "summarize_ms": 1240,
    "analyze_ms": 386,
    "transform_ms": 3105,
    "emit_ms": 90
  },

  "chunks": [
    {
      "id": "main-a1b2c3",
      "hash": "a1b2c3d4e5f6...",
      "file": "chunks/a1b2c3d4.js",
      "size_bytes": 145280,
      "module_count": 412,
      "role": "entry",          // "entry" | "lazy" | "commons" | "dead"
      "entry_points": ["src/index.ts"]
    }
  ],

  "entry_points": {
    "src/index.ts": {
      "initial_chunks": ["main-a1b2c3", "commons-7e8f9a"],
      "initial_bytes": 612480,
      "lazy_chunks": ["route-foo-x1y2", "route-bar-p3q4"]
    }
  },

  "budget": {
    "configured": true,
    "checks": [
      {"name": "initial_bundle_max_bytes", "limit": 700000, "actual": 612480, "status": "ok"},
      {"name": "lazy_chunk_max_bytes",     "limit": 250000, "actual": 412800, "status": "violated", "offender": "route-foo-x1y2"},
      {"name": "total_bundle_max_bytes",   "limit": 12000000, "actual": 8421376, "status": "ok"}
    ],
    "result": "violated"
  },

  "previous_build": {
    "present": true,
    "build_id": "8b4d2e1c9f3a...",
    "delta": {
      "total_bundle_bytes": {"prev": 8198144, "curr": 8421376, "delta": 223232, "pct": 2.72},
      "initial_bundle_bytes": {"prev": 598400, "curr": 612480, "delta": 14080, "pct": 2.35},
      "chunks_added":   ["route-baz-z9y8"],
      "chunks_removed": [],
      "chunks_resized": [
        {"id": "main-a1b2c3", "prev_bytes": 140100, "curr_bytes": 145280, "delta": 5180}
      ]
    }
  }
}
```

**Consumer obligations:**

- CI budget gate: read `budget.result`; non-`ok` ⇒ already failed exit code, but CI can also annotate.
- PR size comment: render `previous_build.delta`.
- Historical regression detection (Scale Benchmark Foundation V5): hash `chunks[].id` and `chunks[].size_bytes` sorted by `id`; compare across runs of the synthetic app. Drift = V5 fail.
- PGO: optional — currently no consumer; field shape allows future linkage of chunk-id → role → ingest.

**Schema evolution rules:**

1. Adding a field at any level: allowed, no version bump.
2. Renaming a field: bump `schema_version` to `"2"`.
3. Removing a field: bump `schema_version` to `"2"`.
4. Tightening a type (e.g., string → enum): bump `schema_version` to `"2"`.

Consumers MUST tolerate unknown fields. The diff helper SHOULD warn on `schema_version` mismatch but continue.

---

## 5. THE INTEGRATION POINTS

### 5.1 JSONL telemetry compatibility (critical — PGO contract)

`wundler-pgo/src/ingestor.rs:32-41` deserialises lines into:

```rust
struct TelemetryEvent {
    session_id: String,
    entry_point: String,
    chunks_served: Vec<String>,
    client_had: Vec<String>,
    timestamp_ms: u64,
}
```

It does **not** use `#[serde(deny_unknown_fields)]`, so:

- Adding optional fields to the existing event: **safe**.
- Adding new event kinds as separate JSON shapes (no `session_id`/`entry_point`/`chunks_served`): the ingestor will `parse_errors += 1; continue;`. **This is acceptable today but wastes parse cycles.**

The clean evolution: introduce a tagged-enum wire type, but emit the existing shape as the default-untagged variant for backward compatibility.

```rust
// crates/wundler-abs/src/types.rs  (additive)
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEventV2 {
    Manifest(ManifestEvent),     // existing shape, kind: "manifest"
    ChunkError(ChunkErrorEvent),
    WebVital(WebVitalEvent),     // Priority 4
}
```

For the existing line shape (no `"kind"` field), the writer emits the legacy shape until `wundler-pgo` learns to recognise `"kind": "manifest"` and tolerate the new shapes. Migration sequence:

1. **Now:** ABS writes legacy shape for manifest events; new shape for `chunk_error` events. PGO `parse_errors` counts but ignores chunk_error lines (acceptable — PGO doesn't use them yet).
2. **Next:** PGO learns to read both shapes (one-line `match` on presence of `"kind"`).
3. **Later:** ABS migrates manifest events to `kind: "manifest"` form. PGO ignores legacy shape after a deprecation window.

This sequencing keeps the contract additive at every step.

### 5.2 `TelemetryEvent.session_id` carries `build_id` today — do not silently fix

`crates/wundler-abs/src/server.rs:200-203`:

```rust
let session_id = req.build_id.clone().unwrap_or_else(|| "anonymous".to_string());
```

This is wrong (the field is called `session_id` but receives `build_id`). It is **out of scope** for this design to fix, because:

- `wundler-pgo` reads `session_id` literally and would suddenly see "anonymous" or a real session id where it was getting build_id-as-session-id before.
- Fixing this is a small, separate change that belongs in a dedicated PR with PGO test coverage.

This design adds a *new* `build_id` field to the new event variants but does **not** touch the existing event shape. Flag this in the implementation plan.

### 5.3 `eval_failed` is page-side, not SW-side

SW cannot observe JS evaluation errors in the page. To report `eval_failed`, a small page-side script must `window.addEventListener('error', ...)` and either:

- `postMessage` to the SW, which posts to ABS (preferred — single network code path), or
- Direct `fetch` to ABS with `keepalive: true`.

Either way, this is outside the SW patch and belongs in a small `wundler-abs/assets/error-reporter.js` injected by `output::write_index_html`. **Defer to a follow-up** — Priority 2 ships without `eval_failed` and addresses F3 partially via `load_failed` + `network_timeout` only. Document this gap.

### 5.4 Where `/metrics` and `/telemetry/*` live in the route table

```rust
pub fn build_router(app: AppState, telemetry: TelemetryLogger, metrics: Arc<Metrics>) -> Router {
    let state = RouterState { app, telemetry, metrics };
    Router::new()
        // Existing
        .route("/manifest",                post(post_manifest))
        .route("/health",                  get(get_health))
        .route("/sw.js",                   get(get_service_worker))
        // New (Priority 2)
        .route("/telemetry/chunk-error",   post(post_chunk_error))
        // New (Priority 3)
        .route("/metrics",                 get(get_metrics))
        // New (Priority 4 — gated)
        // .route("/telemetry/web-vitals", post(post_web_vital))
        .with_state(state)
}
```

When Security Baseline lands, `/metrics` SHOULD require bearer auth and `/telemetry/*` SHOULD be rate-limited (it is the obvious DoS amplification surface — see §7.1).

---

## 6. THE GATE CONDITION (Priority 4)

Web Vitals collection in PGO MUST NOT land until **all** of the following are demonstrably true in a production-equivalent environment for at least one continuous week:

| # | Condition | How to verify |
|---|---|---|
| G-1 | `POST /telemetry/chunk-error` is wired in SW and exercised on real chunk failure | Synthetic test: deploy a manifest pointing to a 404 chunk URL; observe `wundler_chunk_errors_total{error_type="load_failed"}` increment on `/metrics` |
| G-2 | `wundler_chunk_errors_total` is non-zero in steady-state production scrape, OR a deliberately-injected error increments the counter end-to-end | Grafana panel / curl `/metrics` shows the counter family present and changing |
| G-3 | PGO ingestor either consumes `kind: "chunk_error"` events OR explicitly ignores them with a counter (no parse-error spam) | `IngestStats.parse_errors == 0` over a fresh log file containing chunk_error events |
| G-4 | The chunk-error → chunk-id resolution is correct: a known-failing chunk produces the matching `chunk_id` label in the counter | Cross-check `/metrics` label against the deployed `manifest.json` chunk-id field |
| G-5 | `/metrics` scrape latency p99 < 50ms under the realistic chunk-error counter cardinality (build_ids × chunk_ids × 4 error_types) | Load test with VRC's archive default of 5 build_ids and the synthetic app's chunk count (~250) → ~5000 series — well within budget |

When all five are green, Priority 4 may begin. Until then, Web Vitals reporting does not exist server-side, the PGO `SessionRecord` schema does not carry vitals, and `compute_hints` continues to consume only `chunks_served` ordering.

**Why these specific conditions:** they collectively establish that the runtime telemetry path is alive, end-to-end correct, label-stable, and scalable to the cardinality Priority 4 will add (web vitals adds *route* as a label dimension, multiplying cardinality by routes-per-app). If Priority 2's much-lower cardinality already strains `/metrics`, Priority 4 will be worse — measure first.

---

## 7. RISKS

### 7.1 Error storm overwhelming `/telemetry/chunk-error`

A bad deploy can cause every active client to fire `load_failed` on every navigation. Conservative back-of-envelope: 10k DAU × 5 errors × 3 retries = 150k requests in a short window. ABS is `axum`/`tokio` — can soak this — but the JSONL writer holds a `Mutex<BufWriter>` and every event also `.flush()`es; that becomes the bottleneck.

**Mitigations:**

1. Drop the per-event `flush()` for chunk-error events specifically; rely on `BufWriter` capacity + periodic flush. This is a deliberate durability trade — losing the last few hundred ms of chunk errors during an unclean shutdown is acceptable; losing all events under load is not.
2. Per-IP rate limit on `/telemetry/chunk-error` once Security Baseline lands (it's already designed for `/manifest`; reuse the layer).
3. SW client-side deduplication: don't re-report the same `(build_id, chunk_id, error_type)` triple more than once per session.

### 7.2 Budget exit-code 1 breaking existing CI silently

The change "build now exits 1 when X" is backward-incompatible if X used to fail silently. Mitigations baked into the design:

1. `[budget]` is **opt-in** — absent section, no exit-code change. This is the single most important property of the design.
2. The default `wundler.toml` we ship MUST NOT contain a `[budget]` section. Templates that demo budgets live under `examples/`, not the default config.
3. Document a one-line override: `wundler build --no-budget` skips the check (useful for `cargo run` style local iteration that intentionally exceeds limits).

### 7.3 In-process counter loss on ABS restart

Documented as expected. Mitigation: the JSONL log persists chunk-error events; a `wundler abs replay-counters --since <ts> <log>` recovery command can rebuild counters from the log on cold start if needed. Not building this now — restart resets counters, Prometheus's `rate()` handles this correctly.

### 7.4 `build-stats.json` missing from partial build paths

If a build fails after some files are written but before `write_build_stats`, the file is absent or stale. Mitigations:

1. `write_build_stats` is the **last** thing the pipeline does — after manifest, chunks, index.html. A stale `build-stats.json` always corresponds to a complete build.
2. Pre-write delete: at the start of a build, if `out_dir/build-stats.json` exists, *read* it (for the `previous_build` block) but do NOT delete it until the new file is ready. Use a write-then-rename via a tempfile to make the swap atomic.
3. CI consumers should check `mtime(build-stats.json) >= mtime(manifest.json)`; if not, treat as missing.

### 7.5 SW telemetry POST consuming mobile battery

Realistic for a bundler tool: chunk loads are bursty, not continuous. With keepalive-fetch and client-side dedup (§7.1.3), the SW emits at most a handful of POSTs per session even under failure conditions. The bigger concern would be Web Vitals (Priority 4), which fire on every navigation — flag for re-evaluation at the §6 gate. Not a Priority 2 blocker.

### 7.6 Cardinality blow-up on `/metrics`

`wundler_chunk_errors_total{build_id, chunk_id, error_type}` can grow unboundedly if we never prune. VRC's archive bounds active `build_id`s (default 5), but a long-running ABS that has seen many deploys might accumulate stale entries.

**Mitigation:** add `Metrics::prune(active_build_ids: &HashSet<String>)` called whenever the manifest hot-reloads (the `POST /reload` path from VRC). Stale build_id counters drop. This couples Observability to VRC — explicitly. It is correct coupling: cardinality control is downstream of versioning.

### 7.7 The "session_id carries build_id" latent bug

Already in §5.2. Risk if not flagged in implementation plan: someone "fixes" it during this work and silently breaks PGO. Mitigation: explicit out-of-scope note in the plan; dedicated PR with PGO contract test.

---

## 8. EXISTING INFRASTRUCTURE FOUND

The pattern from the prior three designs holds. Concrete reuse:

| Existing | Reused for | Net new code |
|---|---|---|
| `BuildStats` struct (`wundler-pipeline/src/pipeline.rs:27`) | Source of truth for `summary` block | `Serialize` derive + adapter to `BuildStatsArtifact` |
| `BuildOutput.chunk_files` (`wundler-pipeline/src/pipeline.rs:48`) | Source of per-chunk paths/sizes | `fs::metadata` walk |
| `AnalysisStats` (`wundler-graph/src/analyzer.rs:59`) | Already subset of `BuildStats` | None |
| `output::write_manifest` (`wundler-pipeline/src/output.rs`) | Sibling pattern for `write_build_stats` | Mirror the function |
| `TelemetryLogger` (`wundler-abs/src/telemetry.rs`) | Append-only persistence for chunk-error JSONL | Add event variants, no logger changes |
| `TelemetryEvent` (`wundler-abs/src/types.rs`) | Base wire type | New variants `ChunkError`, `WebVital` (Priority 4) |
| `notifyBuildIdChanged` site in SW (`wundler-abs/assets/sw.js:62`) | Adjacent to where chunk-load happens — clean hook neighborhood | New `reportChunkError` helper |
| The three silent `catch {}` blocks in SW (`assets/sw.js:84`, `113`, `137-148`) | Ground zero for every error-reporting concern | Replace with `reportChunkError(...)` |
| `wundler-pgo` ingestor's tolerant deserializer (`crates/wundler-pgo/src/ingestor.rs:32-41`) | Lets us evolve event shapes additively without coordinated PGO release | Schema-tagged enum (§5.1) |
| `wundler-cli` stats printer (`crates/wundler-cli/src/main.rs:372`) | Already reads `BuildOutput.stats` — adjacent to where budget check + `write_build_stats` call belongs | Add ~15 LoC |

Genuinely new infrastructure:

- `crates/wundler-abs/src/metrics/` (4 files, ~350 LoC).
- `crates/wundler-pipeline/src/budget.rs` (~80 LoC).
- `crates/wundler-pipeline/src/build_stats.rs` (~150 LoC).
- One new SW helper (~25 LoC).
- One new TOML section under `[budget]`.

Total net new: ~600-700 LoC across two crates plus SW. Same shape as the prior designs.

---

## 9. SIMPLEST CREDIBLE ALTERNATIVE

If we built **only the absolute minimum** that gives a developer standing information about build-size regression:

1. Add `#[derive(Serialize)]` to `BuildStats`.
2. In `BuildPipeline::build()`, after the existing stats computation, `serde_json::to_writer_pretty(out_dir.join("build-stats.json"), &stats)`.
3. Done.

~10 lines of code. Closes F1 the moment any consumer (a `jq` script in CI, a Makefile, a developer's eyeballs) reads the file. No budget enforcement, no per-chunk breakdown, no build_id, no diff — but the **regression-detection floor is established** and Scale Benchmark Foundation can start consuming a partial artifact immediately.

Everything else in this design is *credible reasons to do more than this*. The credibility comes from concrete failure modes (F1-F6), concrete consumers (CI, SBF V5, PGO), and concrete gate constraints (the COE's Web Vitals footgun). When in doubt, ship the 10-line version and earn the rest.

---

## 10. OPEN QUESTIONS

| # | Question | Default answer if unanswered |
|---|---|---|
| Q1 | Does `wundler build --no-budget` skip both checks AND `build-stats.json` emission, or just the budget check? | Skip only the check. Always emit the artifact. |
| Q2 | When `previous_build` is present but `schema_version` differs, do we still emit a delta? | No. Set `previous_build.present: false, schema_mismatch: true`. |
| Q3 | Should `/metrics` require bearer auth from day one? | Yes, conditioned on whether Security Baseline P1 has landed. If not, document the gap. |
| Q4 | Does Priority 2 ship with `eval_failed` page-side reporter, or as a deliberate follow-up? | Follow-up. Priority 2 covers `load_failed`, `network_timeout`, and `integrity_mismatch` (the last only when SRI is on). |
| Q5 | Where does `build-stats.json` live in the output dir relative to `manifest.json` — same dir, or `dist/.wundler/`? | Same dir as `manifest.json`. Convention beats nesting; tooling reads `out_dir`. |

---

## 11. SUMMARY

**Recommended pick per priority:** C2 / C2 / C2 / Gated.

**Sequencing for implementation:**

1. **P1** ships first and standalone. Unblocks Scale Benchmark Foundation V5 immediately. No runtime infrastructure.
2. **P2** ships after P1. Adds the `Metrics` struct + `/telemetry/chunk-error` route + SW patch. Counters live in-process.
3. **P3** is a thin layer over P2 — exposition for the counters P2 already maintains. Lands days after P2.
4. **P4** waits for the §6 gate. Implementation is mechanically analogous to P2; the gate is the design.

**Critical contracts:**

- `build-stats.json` schema (§4) — frozen at `schema_version: "1"`; additive evolution only.
- JSONL event compatibility with `wundler-pgo` (§5.1) — tagged-enum migration path; no coordinated release required.
- `Metrics::prune` coupling to VRC's manifest hot-reload (§7.6) — cardinality control downstream of versioning.

**What this design refuses to do:**

- Fix the `session_id`-carries-`build_id` bug in passing (§5.2). Separate PR.
- Ship Web Vitals on faith (§6). Earn the gate first.
- Pull the OTel SDK to get Prometheus exposition (§2.3 C3). 100 LoC of `format!` is the right tool.
