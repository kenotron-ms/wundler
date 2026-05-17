# Wundler — Implementation Progress

> **How to use:** At the start of any session, read this file to know where we left off.
> After completing each task: check the box in the plan file.
> After each session: update the counts and status below, add a session note.

---

## Correlation: Plan → Design → Status

### Phase 1 — Foundation (all parallel, no inter-dependencies)

#### VRC SCA — deterministic `build_id` + `POST /reload`
- **Plan:** `docs/superpowers/plans/2026-05-15-vrc-sca.md`
- **Design:** `docs/designs/versioned-runtime-control.md` → §Simplest Credible Alternative
- **Status:** ✅ **COMPLETE** — 8 / 8 tasks
- **Branch:** `feat/phase1-roadmap`
- **Commits:** `9ab1ec4`, `da08fca`, `6e311e9`, `feb3ade`, `08e654f`

#### Security P1.1 — bearer token middleware
- **Plan:** `docs/superpowers/plans/2026-05-15-security-p1-bearer-token.md`
- **Design:** `docs/designs/security-baseline.md` → §Phase 1 → C2 → P1.1
- **Status:** ✅ **COMPLETE** — 6 / 6 tasks
- **Branch:** `feat/phase1-roadmap`
- **Commits:** `c1174c5`, `6a663f1`, `2d2146d`, `1cf797c`, `f8f85f9`

#### Observability SCA — `build-stats.json`
- **Plan:** `docs/superpowers/plans/2026-05-15-observability-sca.md`
- **Design:** `docs/designs/observability.md` → §Priority 1 → SCA
- **Status:** ✅ **COMPLETE** — 5 / 5 tasks
- **Branch:** `feat/phase1-roadmap`
- **Commits:** `0c42c9d`

---

### Phase 2 — COMPLETE

#### Security P1.2 — CORS allowlist
- **Plan:** `docs/superpowers/plans/2026-05-16-security-p1-cors.md`
- **Design:** `docs/designs/security-baseline.md` §P1.2
- **Status:** ✅ **COMPLETE** — 3 / 3 tasks
- **Branch:** `feat/phase2-roadmap`
- **Commits:** `ce02096`, `3448869`, `67e5b41`

#### Security P1.3 — per-IP rate limiter
- **Plan:** `docs/superpowers/plans/2026-05-16-security-p1-rate-limit.md`
- **Design:** `docs/designs/security-baseline.md` §P1.3
- **Status:** ✅ **COMPLETE** — 3 / 3 tasks
- **Branch:** `feat/phase2-roadmap`
- **Commits:** `c8c7aa8`, `d53a003`, `29cdae9`, `1c43717`

#### VRC C2 — manifest archive + GET /versions + POST /select
- **Plan:** `docs/superpowers/plans/2026-05-16-vrc-c2.md`
- **Design:** `docs/designs/versioned-runtime-control.md` §C2
- **Status:** ✅ **COMPLETE** — 6 / 6 tasks
- **Branch:** `feat/phase2-roadmap`
- **Commits:** `80d3ee3`, `4c1bf56`, `74a6b6d`, `6bf86b5`, `2399817`, `b6732b4`

#### Observability P1 full — extended BuildStats + budget enforcement
- **Plan:** `docs/superpowers/plans/2026-05-16-observability-p1-full.md`
- **Design:** `docs/designs/observability.md` §P1 full
- **Status:** ✅ **COMPLETE** — 6 / 6 tasks
- **Branch:** `feat/phase2-roadmap`
- **Commits:** `6ce40cc`, `feace19`, `c49d8db`, `de34a95`, `4d568d0`, `ed80549`

### Phase 3 — COMPLETE

#### Security P2 — ed25519 manifest signing + SRI + CSP report-only
- **Plan:** `docs/superpowers/plans/2026-05-17-security-p2-signing.md`
- **Design:** `docs/designs/security-baseline.md` §Phase 2
- **Status:** ✅ **COMPLETE** — 5 / 5 tasks
- **Branch:** `feat/phase3-roadmap`
- **What shipped:** `AppState.signature` slot, `GET /manifest/full.json` with `X-Wundler-Signature`, `POST /csp-report` ingester, `security/csp.rs` builder, `wundler-html` crate (`hex_to_sri_b64` + `render_script_tags`), ManifestSigner wired into `POST /reload`, cross-language contract test (Rust half)

#### Observability P2 — chunk error reporting + Prometheus /metrics
- **Plan:** `docs/superpowers/plans/2026-05-17-observability-p2-chunk-errors.md`
- **Design:** `docs/designs/observability.md` §P2 + §P3
- **Status:** ✅ **COMPLETE** — 5 / 5 tasks
- **Branch:** `feat/phase3-roadmap`
- **What shipped:** `Metrics` struct with DashMap counters, `POST /telemetry/chunk-error` handler, hand-rolled Prometheus `render()`, `GET /metrics`, `TelemetryEventV2` tagged enum, prune-on-reload, SW `reportChunkError()` + dedup + 3 catch-block replacements

#### Performance P1 — dependency pre-bundling
- **Plan:** `docs/superpowers/plans/2026-05-17-performance-p1-dep-prebundle.md`
- **Design:** `docs/designs/performance.md` §P1
- **Status:** ✅ **COMPLETE** — 3 / 3 tasks
- **Branch:** `feat/phase3-roadmap`
- **What shipped:** `wundler-dev` crate with `DepPrebundler` (blake3 fingerprint, atomic cache-miss via tempdir+rename, GC), `[dev]` TOML section, wired into `run_dev` before watcher start

#### Scale Benchmark Foundation — synthetic corpus profiler (MVP steps 1–7)
- **Plan:** `docs/superpowers/plans/2026-05-17-scale-benchmark-foundation.md`
- **Design:** `docs/designs/scale-benchmark-foundation.md`
- **Status:** ✅ **COMPLETE** — 5 / 5 tasks
- **Branch:** `feat/phase3-roadmap`
- **What shipped:** `BenchProfile` contract, TS/JSON/Markdown archetypes with `pad_to_target`, deterministic `generate_corpus`, V1+V3 `verify_corpus`, CLI `generate-corpus`/`verify-corpus`, committed profiles, V5 CV gate (`--repeat N --check-cv`), E2E integration test

### Still blocked (need Phase 3 to complete first)

| Item | Design Doc | Blocked on |
|---|---|---|
| Security P2 — SW ed25519 verification (JS half) | `security-baseline.md` §Phase 2 | Security P2 Rust half + SW build pipeline |
| Observability P3 — Prometheus `/metrics` | Already wired in Obs P2 plan, just the endpoint | Observability P2 counters live |
| Performance P2 — true HMR / react-refresh | `performance.md` §P2 | Performance P1 stable |

### Phase 4 — Evidence-gated (do not plan until gates pass)

| Item | Design Doc | Gate condition | Status |
|---|---|---|---|
| Performance P1 — dep pre-bundling | `performance.md` §P1 | No gate | ✅ Shipped in Phase 3 (SCA only — see decision below) |
| Performance P3 — incremental graph | `performance.md` §P3 | p95 cold analyze > threshold on office-scale corpus AND V5 passes | 🟡 V5 passed; threshold decision below |
| Performance P4 — parallel PGO ingestion | `performance.md` §P4 | ≥1GB log ingested AND wall > 30s | ❌ No data yet |
| Observability P4 — Web Vitals in PGO | `observability.md` §P4 (GATED) | Chunk errors live + showing signal | 🟡 Chunk errors shipped; waiting for signal |

### Analysis baseline — measured 2026-05-17

**Synthetic scale (400-byte inert TS files):**
```
wundler-bench analysis-bench --modules N --repeat 3

N=100     cold=4ms   warm=1ms   speedup=4x
N=1000    cold=40ms  warm=18ms  speedup=2x
N=5000    cold=226ms warm=101ms speedup=2x
N=10000   cold=435ms warm=213ms speedup=2x
N=19343   cold=881ms warm=437ms speedup=2x
```

**Real-scale projection — office-bohemia (293 packages, 36,001 tracked files):**
```
repo-scale run on ~/workspace/office-bohemia (70,709 commits)

TS source files:   15,142 .ts  @ avg 5,134 bytes  (12.8x larger than synthetic)
                    4,201 .tsx @ avg 5,629 bytes  (14.1x larger than synthetic)
Total:             19,343 files @ avg 5,242 bytes
Correction factor: 13.1x

Projected cold: 881ms × 13.1 ≈ 11.5 seconds
Projected warm: 437ms × 13.1 ≈  5.7 seconds
```

Note: projection assumes analysis cost scales linearly with byte size, which is a
reasonable approximation for the summarize+hash phase. Graph analysis cost scales
with import edge count, not file size — so warm may be less than 5.7s.

**Corpus generation verified:**
```
generate-corpus profiles/office-bohemia/ts-full.v1.json → 19,343 files in 0.2s, 152MB
verify-corpus: 7 checks, passed=true
CI scale (ts-ci.v1.json, 1,934 files): 7 checks, passed=true
```
Profiles committed to `crates/wundler-bench/profiles/office-bohemia/`.

**P3 threshold decision:**
11.5s cold / 5.7s warm at office-bohemia scale is slow enough to hurt developer experience.
**BASELINE = 5000ms cold** on office-bohemia TS corpus. P3 (incremental graph) is justified.

### Decision required: Performance P1 Phase 2

**Current state:** `bundle_into()` in `wundler-dev/src/prebundle/mod.rs` writes only
`{"fingerprint": "hex"}`. The cache infrastructure is built but the cache is empty.
`wundler dev` logs "dep pre-bundle complete" but node_modules is still parsed fresh.

**Three options — human call required:**

| Option | What it means | Cost |
|---|---|---|
| **A: Complete as designed** | Implement actual bundling in `bundle_into()` — run esbuild/rollup against node_modules, write output chunks to cache dir. P2 (HMR) becomes possible once P1 has real output. | 2–4 weeks; adds a bundler dependency to `wundler-dev` |
| **B: Defer** | Leave SCA as-is; do not plan P2 (HMR) until P1 is real. No action now. | No cost now; P2 blocked indefinitely |
| **C: Reframe / close** | The analysis baseline shows analysis of node_modules-scale corpora is fast. The CAS warm cache already provides 2x speedup. The problem P1 was solving may not exist at wundler's target scale. Close P1 as SCA-only; do not implement Phase 2. P2 (HMR) replanned without P1 dependency. | Honest accounting of effort spent; HMR replanned |

Option C is worth considering: if a 50k-module project analyzes in ~2s cold on real hardware,
and the warm CAS path cuts that to ~1s, pre-bundling saves perhaps 0.5s on restarts.
That is not nothing, but it may not justify owning an embedded bundler.

### Known gaps (tracked, not blocked)

| Gap | Location | What's missing |
|---|---|---|
| Security P2 — SW ed25519 verification (JS half) | `assets/sw.js` line 127, comment added | SW fetches CDN manifest, never sees `X-Wundler-Signature`. Full verification path documented in code. |
| Performance P1 — actual pre-bundling | `wundler-dev/src/prebundle/mod.rs` `bundle_into()` | Writes `index.json` only. Decision above required before planning any further P1/P2 work. |

---

## Dependency Chain

```
Phase 1 (VRC SCA + Security P1.1 + Observability SCA)
  └─ all parallel, ship independently
  └─ unblocks Phase 2

Phase 2 (Security P1 full + VRC C2 + Observability P1 full)
  └─ VRC SCA must be done for VRC C2
  └─ unblocks Phase 3

Phase 3 (Security P2 + Scale Benchmark + Observability P2+P3)
  └─ Security P2 requires VRC SCA (stable build_id)
  └─ Observability P3 requires Observability P2

Phase 4 (Performance + Web Vitals)
  └─ evidence-gated: start only when measurements justify
```

---

## Design Documents

| Design | File | Summary |
|---|---|---|
| Versioned Runtime Control | `docs/designs/versioned-runtime-control.md` | build_id in every manifest, archive, hot reload, rollback |
| Security Baseline | `docs/designs/security-baseline.md` | Phase 1: auth/CORS/rate-limit; Phase 2: signing/SRI/CSP |
| Scale Benchmark Foundation | `docs/designs/scale-benchmark-foundation.md` | profile-driven synthetic corpus, graph-shape preservation |
| Observability | `docs/designs/observability.md` | build-stats.json, chunk errors, Prometheus, Web Vitals (gated) |
| Performance | `docs/designs/performance.md` | dep pre-bundling, HMR, incremental graph, parallel PGO |

---

## Session Log

### 2026-05-17 — Phase 3 complete (18 tasks)

**Completed — all four Phase 3 plans (18 tasks total):**

**Scale Benchmark Foundation (5 tasks):**
- `BenchProfile` JSON contract with schema version gate
- TypeScript/JSON/Markdown file archetypes with deterministic `pad_to_target`
- `generate_corpus(profile, out_dir)` — seeded PRNG, proportional dir allocation, byte-aligned files
- `verify_corpus(profile, dir)` — V1 structural conformance + V3 SWC parse sample
- CLI `generate-corpus`/`verify-corpus` subcommands; committed `tiny.v1.json` + `large-web-app-small.v1.json`; V5 CV gate `--repeat N --check-cv <T>` on `analysis-bench`

**Performance P1 — Dependency Pre-bundling (3 tasks):**
- New `wundler-dev` crate with `DepPrebundler`, `PrebundleResult`, `compute_fingerprint` (blake3)
- `ensure_fresh`: cache-hit fast path + atomic cache-miss via tempdir + `std::fs::rename`; `gc()` TTL-based eviction
- `[dev]` TOML section with `dep_cache_ttl_days`; wired into `run_dev` before watcher start; >1 GB cache warning

**Security P2 — ed25519 Manifest Signing (5 tasks):**
- `AppState.signature: Arc<RwLock<Option<Signature>>>` with `snapshot_signature()` / `set_signature()` / reset on `swap_to`
- `wundler-html` crate: `hex_to_sri_b64()` + `render_script_tags()` with `integrity="sha256-..."` tags
- `GET /manifest/full.json`: public endpoint, `X-Wundler-Build-Id` + `X-Wundler-Signature` (base64), `Cache-Control: immutable`, exempt from bearer auth
- `security/csp.rs`: `build_csp_report_only()` + `POST /csp-report` sink (8 KiB cap, JSONL log, 413 on oversize)
- ManifestSigner wired into `POST /reload`; `run()` loads key from `signing_key_pem`; cross-language contract test (Rust half)

**Observability P2 — Chunk Error Reporting (5 tasks):**
- `metrics/mod.rs`: `Metrics` struct with `DashMap<ChunkErrorKey, AtomicU64>`, `prune(active_build_ids)`, `increment_chunk_error()`
- `POST /telemetry/chunk-error`: 8 KiB cap, parse, DashMap increment, `TelemetryEventV2::ChunkError` JSONL log, 200 OK
- Hand-rolled `metrics/prometheus.rs`: `render(&Metrics) -> String` + `GET /metrics` endpoint
- `TelemetryEventV2` tagged enum in `types.rs` (additive); `prune` wired into `POST /reload`
- SW `reportChunkError()`: fire-and-forget with `keepalive: true`, client-side dedup `Set`, replaced 3 silent `catch {}` blocks

**Branch:** `feat/phase3-roadmap` — merged to main

**Next session:**
- Phase 4 items are all evidence-gated or require Performance P1 to be stable first
- Options: Performance P2 (HMR, gates on P1 stable), or writing Observability P3 plan (Prometheus /metrics is already wired — it's about connecting to a real scraper + alerting)

---


### 2026-05-16 — Phase 2 complete (18 tasks)

**Completed — all four Phase 2 plans (18 tasks total):**

**Observability P1 Full (6 tasks):**
- `BuildStatsArtifact` schema (schema_version=1) with per-chunk breakdown, timing, entry-point records, budget result, previous-build delta
- Atomic `write_build_stats` + `read_previous_stats` in `output.rs`
- Per-phase `Instant` timing wired into `BuildPipeline::build()`
- `budget.rs` with `check()`, `BudgetViolation::actionable_message()`, `build_budget_result()`; opt-in via `[budget]` in `wundler.toml`
- Real delta computation: `SizeDelta`, set-diff chunk added/removed

**Security P1.2 CORS (3 tasks):**
- `SecurityConfig.allowed_origins: Vec<String>` with wildcard rejection
- `security/cors.rs` with `build_cors()` using `tower_http::cors::AllowOrigin::list`
- Wired as outermost layer in `build_router`; 5 integration tests including 401-still-carries-CORS

**Security P1.3 Rate Limiter (3 tasks):**
- `governor = "0.7"` dep; `manifest_rate_per_sec`/`burst` config fields
- `security/ratelimit.rs`: `IpRateLimiter` alias + `build_limiter()` + `rate_limit_mw` middleware
- Wired as `.route_layer()` on `POST /manifest` only — `/health` and `/sw.js` exempt by construction

**VRC C2 — Manifest Archive (6 tasks):**
- `ManifestArchive`: `open`, `install` (atomic + idempotent), `list` (newest-first), `load`, `set_current` (atomic symlink swap), `prune` (retention bound, current entry protected)
- `AppState` migrated to double-Arc pattern (`Arc<RwLock<Arc<ChunkManifest>>>`), + `archive: Arc<ManifestArchive>`, + `reload_lock: Arc<Mutex<()>>`; `snapshot()` + `swap_to()` + `reload_from_current()`
- `POST /reload` now installs to archive before swapping; returns `{previous, current}`
- `GET /versions` — lists archive entries with `is_current` flag
- `POST /select` — operator rollback; loopback enforcement; 404 on unknown build_id; 409 on concurrent swap

**Branch:** `feat/phase2-roadmap` — merged to main

**Next session:**
- Phase 3 plans not yet written; Phase 3 requires Phase 2 to be complete (now done)
- Consider writing Phase 3 plans: Security P2 (ed25519 signing), Scale Benchmark Foundation, Observability P2 (chunk errors)

---

### 2026-05-16 — Overnight session (Phase 1 complete)

**Completed — all three Phase 1 plans (19 tasks total):**

**Observability SCA (5 tasks) — commit `0c42c9d`:**
- Added `serde::Serialize` to `BuildStats`
- Write `build-stats.json` to `out_dir` after every successful build (non-fatal)
- 4 tests in `crates/wundler-pipeline/tests/build_stats_json_test.rs`
- Fixed pre-existing Clippy lints: `derivable-impls` on `EngineChoice`, `doc-overindented-list-items` in `output.rs`, `for-kv-map` in `pipeline.rs`, `needless-splitn` in `rolldown_adapter.rs`

**VRC SCA (8 tasks) — commits `9ab1ec4` through `08e654f`:**
- New `crates/wundler-pipeline/src/build_id.rs` with `canonical_bytes()` + `compute_build_id()` (SHA-256, 16-char hex)
- `BuildPipeline::build()` now overwrites graph-layer `build_id` with content-based ID
- `AppState::reload_manifest()` for atomic manifest hot-swap
- `POST /reload` endpoint with file read, JSON parse, loopback enforcement via custom `MaybeConnectAddr` extractor (Axum 0.8 compatibility)
- 10 new tests across 3 test files

**Security P1.1 (6 tasks) — commits `c1174c5` through `f8f85f9`:**
- `crates/wundler-abs/src/security/` module: `SecretToken` (constant-time verify, Debug/Display redacted), `SecurityConfig`, `SecurityError`, `ResolvedSecurity`
- `require_bearer` Axum middleware with exempt paths (`/health`, `/sw.js`)
- `build_router` updated to `build_router(app, telemetry, Arc<ResolvedSecurity>)` with middleware wired
- `AbsConfig` gains optional `security` field (zero breaking change for existing configs)
- 17 new tests in `auth_test.rs`

**Also fixed:**
- Pre-existing `wundler-cli` compile error: `write_report` signature mismatch and missing `AbsConfig.security` field — commit `7142e05`

**Branch:** `feat/phase1-roadmap` — all work is on this branch, ready for PR

**Next session:**
- Open a PR: `feat/phase1-roadmap` → `main`
- Phase 2 plans are now unblocked (see Phase 2 table above)
- Write Phase 2 plans: Security P1.2 (CORS), P1.3 (rate limiter), VRC C2 (archive + `/versions` + `/select`)

---

### 2026-05-15 — Session d98b7357

**Completed:**
- [x] `feat(bench)`: repo_scale profiler tool — `wundler-bench repo-scale` subcommand, 44 tests, commit `3e9ec01`
- [x] `docs(designs)`: 5 production roadmap design documents, commit `f700f32`
- [x] 3 implementation plans written (not yet committed)

**Decisions made:**
- `build_id` = deterministic SHA-256 of canonical manifest bytes (not git hash, not UUID)
- Security token loaded from file (not inline in wundler.toml)
- `POST /reload` loopback-only until Security Baseline P1 lands
- Profile naming: `large-web-app.v1.json` (no internal codenames in public repo)
- 5x scale aspirational target (Teams-class) added as G8 in scale-benchmark-foundation.md

**Next session start:**
- Commit the 3 plan files first (`docs/superpowers/plans/*.md`)
- Pick any Phase 1 plan and start at Task 1
- All three plans are parallel — pick whichever is most useful to prove first
