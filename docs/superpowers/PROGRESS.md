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

### Phase 2 — After Phase 1 complete (plans not yet written)

| Item | Design Doc | Blocked on |
|---|---|---|
| Security P1.2 — CORS allowlist | `security-baseline.md` §P1.2 | Phase 1 done |
| Security P1.3 — per-IP rate limiter | `security-baseline.md` §P1.3 | Phase 1 done |
| VRC full C2 — archive + `/select` + `/versions` | `versioned-runtime-control.md` §C2 | Phase 1 done |
| Observability P1 full — extended BuildStats + CI budgets | `observability.md` §P1 full | Phase 1 done |

### Phase 3 — After Phase 2 (plans not yet written)

| Item | Design Doc | Blocked on |
|---|---|---|
| Security P2 — ed25519 signing | `security-baseline.md` §Phase 2 | VRC SCA (`build_id` stable) |
| Security P2 — SRI + CSP report-only | `security-baseline.md` §Phase 2 | Security P2 signing |
| Scale Benchmark Foundation | `scale-benchmark-foundation.md` | Observability P1 (V5 check) |
| Observability P2 — chunk error reporting | `observability.md` §P2 | Phase 1 done |
| Observability P3 — Prometheus `/metrics` | `observability.md` §P3 | Observability P2 |

### Phase 4 — Evidence-gated (do not plan until gates pass)

| Item | Design Doc | Gate condition |
|---|---|---|
| Performance P1 — dep pre-bundling | `performance.md` §P1 | No gate — can start anytime |
| Performance P3 — incremental graph | `performance.md` §P3 | p95 analyze > [BASELINE TBD] on office-scale corpus |
| Performance P4 — parallel PGO ingestion | `performance.md` §P4 | ≥1GB log ingested AND wall > 30s |
| Observability P4 — Web Vitals in PGO | `observability.md` §P4 (GATED) | Chunk errors live + showing signal |

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
