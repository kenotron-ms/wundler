# Versioned Runtime Control

**Status:** Design proposal
**Author:** systems-design
**Phase:** Roadmap Phase 1 — foundation for Security Baseline, Observability, Performance, HA/blue-green
**Scope:** `wundler-abs`, `wundler-pipeline`, `wundler-graph`, `wundler-cli`, `wundler-sw` (consumer contract only)

---

## TL;DR

Wundler already has the **kernel** of versioning in place — `ChunkManifest.build_id`, `Arc<RwLock<ChunkManifest>>` in `AppState`, build-id-gated cache trust in `compute_delta`, and a `wundler:build-id-changed` postMessage in the Service Worker. What it does not have is a **mechanism**: deterministic build-id generation, an on-disk archive of past manifests, atomic hot reload, and an operator endpoint to point ABS at a specific build.

This design fills that gap. ABS becomes a **mechanism** that holds a versioned archive and serves whichever build is currently selected. Rollback policy (when to roll back, what to roll back to, who decides) stays with the caller. Blue/green is explicitly deferred — it is a policy layered on top of two ABS instances, not a feature of this subsystem.

---

## 1. ANALYZE — System Map

### 1.1 Goals

| # | Goal |
|---|---|
| G1 | Every manifest has a stable, **deterministic** `build_id` derived from its content |
| G2 | ABS maintains an on-disk archive of past manifests; default retention = last 10 builds |
| G3 | ABS hot-reloads atomically: file change → load → validate → swap → in-flight requests complete against prior manifest. No restart |
| G4 | Operator can roll back to any archived `build_id` via `POST /select` |
| G5 | Service Worker records the `build_id` of the manifest it loaded |
| G6 | CDN caching rules are explicit and documented: content-hashed chunks immutable; manifest no-store/no-cache; index.html no-cache |
| G7 | Blue/green deferred. This subsystem must not preclude it but must not implement it |

### 1.2 Constraints

| # | Constraint |
|---|---|
| C1 | Rust only; no new runtimes |
| C2 | ABS is in-process; archive is on-disk files (no DB, no external state store) |
| C3 | `wundler.toml` is the config surface |
| C4 | Small team; operational simplicity is non-negotiable |
| C5 | Wundler does not own the SW lifecycle; design must compose with standard SW update semantics |
| C6 | CAS hashes are already stable; `build_id` must also be deterministic for a given build output |
| C7 | No external KMS / signing infrastructure yet (Security Baseline handles that) |
| C8 | Existing `wundler-bench` infrastructure must not break |

### 1.3 Actors

| Actor | Interaction |
|---|---|
| `wundler build` CLI | Produces `ChunkManifest`, writes manifest + chunks to `out_dir/`, installs into archive |
| Operator (human or CI/CD) | Calls `POST /select`, `GET /versions`, `POST /reload`; sets retention; rolls back |
| ABS HTTP clients (apps) | `POST /manifest` to get delta; report `build_id` they last saw |
| Browser Service Worker | Loads `manifest.json` from CDN, posts `POST /manifest` to ABS, receives `build_id`, postMessages clients on change |
| CDN / reverse proxy | Caches chunks (immutable), revalidates `manifest.json` and `index.html` |
| SRE on-call | Reads `/health`, `/versions`, archive directory; triggers `POST /select` for rollback |

### 1.4 Interfaces (current and proposed)

**Existing:**

- `POST /manifest` → `ManifestResponse { build_id, fetch_urls, prefetch_urls, ttl }`
- `GET  /health`   → `HealthResponse { status, build_id }`
- `GET  /sw.js`    → embedded Service Worker script
- `ChunkManifest.build_id: String` (already in `wundler-graph::types`)
- `AppState.manifest: Arc<RwLock<ChunkManifest>>` (already future-proofed for hot reload)
- SW `notifyBuildIdChanged(newBuildId)` (already wired)
- `compute_delta` gates cache trust on `build_id` match (already correct)

**Proposed (new in this design):**

- `GET  /versions`        → list archive entries
- `POST /select`          → swap to a specific `build_id`
- `POST /reload`          → re-read the `current` pointer (operator-driven refresh)
- `GET  /cdn-self-check`  → returns the cache-header policy ABS expects upstream proxies to honor
- `compute_build_id(manifest)` → deterministic hash of canonical manifest bytes
- `ManifestArchive` struct (new `wundler-abs/src/archive.rs`)

### 1.5 What already exists that this builds on

| Component | File | What it gives us |
|---|---|---|
| `ChunkManifest.build_id` | `crates/wundler-graph/src/types.rs:89` | Field exists; currently set from a timestamp/SHA placeholder in the pipeline |
| `AppState` with `Arc<RwLock<ChunkManifest>>` | `crates/wundler-abs/src/state.rs:21` | Doc-commented "to support future hot-reload" |
| Build-id-aware cache trust | `crates/wundler-abs/src/manifest.rs:79` | Stale build_id → all client cache claims ignored (correct rollback semantics out of the box) |
| `HealthResponse.build_id` | `crates/wundler-abs/src/server.rs:100` | Operators can already poll `/health` to see what's loaded |
| SW build-id postMessage | `crates/wundler-abs/assets/sw.js` `notifyBuildIdChanged()` | SW already broadcasts version changes to clients |
| Telemetry log | `crates/wundler-abs/src/telemetry.rs` | Append-only JSONL — we can extend events to include observed `build_id` |
| Atomic file writes via temp+rename | `crates/wundler-pipeline/src/output.rs` | Already the convention; archive will adopt it |

This design adds ~3 files and modifies ~5. The kernel is already there.

### 1.6 Failure modes WITHOUT this design

| # | Failure mode | Blast radius | Severity |
|---|---|---|---|
| F1 | Bad build deployed → operator must restart ABS to roll back; restart drops in-flight `/manifest` requests | All currently-loading clients see 500s during restart; new clients get whichever build was on disk when ABS came up | **High** |
| F2 | `build_id` is non-deterministic (timestamp-based) → two identical builds get different IDs → cache trust is needlessly invalidated → clients re-download chunks they already have | Bandwidth/perf regression on every rebuild, even no-op rebuilds; defeats CAS | **High** |
| F3 | Archive does not exist → after a build, the previous manifest is gone → rollback is impossible without rebuilding the prior commit | Rollback time = build time (minutes), not pointer swap (milliseconds). Outage window stays open | **Critical for prod** |
| F4 | No `/versions` endpoint → operator has no way to know what's available to roll back to | Operator guesses; pages SRE; reads filesystem; bad calls under stress | **Medium** |
| F5 | CDN cache policy undocumented → chunk URLs cached with `no-cache`, manifest.json cached forever — possibilities exist for both client never sees new build and client never serves cached old chunk | First-load and cache-hit performance both regress silently | **High** (silent → very high) |
| F6 | SW has no way to know it's stuck on an old build → silently serves stale chunks until cache eviction | Stale users seeing old code; bug reports without reproducer; trust erosion | **Medium** (lifecycle eventually catches up, but the window is days) |
| F7 | Operator cannot ask "is this build still serving anyone?" → no signal for safe archive prune | Either we keep manifests forever (disk fill) or we prune blindly (rollback target disappears) | **Medium** |

### 1.7 Time horizons

- **Now (this design):** Mechanism — build_id, archive, hot reload, /select, CDN doc.
- **+1 quarter:** Security Baseline plugs into this — signed manifests verified at swap-in time; `/select` becomes authenticated.
- **+2 quarters:** Observability — telemetry events include observed build_id; per-build_id metrics; SLO dashboards.
- **+3 quarters:** Blue/green policy — two ABS instances, each pointing at a different build; load balancer routes by header/cookie. Requires nothing new from this subsystem.

---

## 2. DESIGN — Three Candidate Architectures

### 2.1 Candidate 1 — Minimal Viable Versioning

> Stable build_id; archive as append-only directory; file-watcher hot reload; no operator API; CDN rules documented only.

#### Components

| Component | File | Responsibility |
|---|---|---|
| `compute_build_id()` | new `wundler-pipeline/src/build_id.rs` | Pure function: canonicalize manifest → SHA-256 → 16-char hex |
| `canonical_bytes()` on `ChunkManifest` | extend `wundler-graph/src/types.rs` | Deterministic byte serialization: chunks sorted by id; modules sorted within chunk; `entry_chunks` keys sorted; BTreeMap, not HashMap |
| Pipeline archive write | extend `wundler-pipeline/src/output.rs` | After `write_manifest`, copy to `archive_dir/<build_id>.json` and atomically swap `archive_dir/current` symlink |
| Archive retention | extend `wundler-pipeline/src/output.rs` | Before install, prune oldest entries beyond `retention` count |
| ABS file watcher | extend `wundler-abs/src/server.rs` | `notify` crate watches `archive_dir/current`; on change → `AppState::reload_current()` |
| `AppState::reload_current()` | extend `wundler-abs/src/state.rs` | Read symlink target, parse manifest, validate (build_id matches filename), atomic write-lock swap |
| CDN policy doc | new `docs/runtime/cdn-cache-policy.md` | Static document; no enforcement |

#### Data flow

```
wundler build
   │
   ├─► ChunkManifest (in-memory)
   │     │
   │     └─► compute_build_id() → "a1b2c3d4e5f6..."
   │            │
   │            └─► manifest.build_id := that
   │
   ├─► write_manifest(out_dir/manifest.json)
   │
   ├─► install_archive(archive_dir, manifest)
   │     ├─► write archive_dir/<build_id>.json (tmp + rename)
   │     ├─► prune oldest beyond retention
   │     └─► atomic symlink swap: archive_dir/current → <build_id>.json
   │
   ▼
ABS  ── notify-rs event ──►  reload_current()
                                ├─► resolve symlink target
                                ├─► parse + validate
                                └─► write-lock swap manifest

Browser SW ── POST /manifest ──► compute_delta (uses Arc snapshot)
                                  └─► resp.build_id := current.build_id
```

#### What it optimizes for / sacrifices

- **Optimizes for:** Smallest change to existing code; no new endpoints; no auth surface to worry about; relies on filesystem semantics (atomic rename + symlink swap) which are battle-tested.
- **Sacrifices:** Operator rollback is **shell-level**: `ln -sfn archive_dir/<old>.json archive_dir/current && touch archive_dir/current`. No `/versions` endpoint — operator must `ls`. No way to introspect from outside the host. CDN policy is documentation, not validation.

#### Why this could be wrong

It fails **G4 explicitly**. The goal says "Operator can roll back to any archived `build_id` via `POST /select`." Candidate 1 says: "via shell command on the ABS host." That's not the same shape of capability — it forces operators onto the host at the exact moment they want to be remote (incident).

---

### 2.2 Candidate 2 — Full Operator Control (Recommended)

> Everything in C1, plus `/versions`, `/select`, SW build_id reporting, and CDN policy validated by a self-check endpoint.

#### Components

| Component | File | Responsibility |
|---|---|---|
| `compute_build_id()` | new `wundler-pipeline/src/build_id.rs` | Same as C1 |
| `canonical_bytes()` | extend `wundler-graph/src/types.rs` | Same as C1 |
| `ManifestArchive` | new `wundler-abs/src/archive.rs` | Owns archive dir; CRUD; retention; index in-memory |
| Pipeline archive write | extend `wundler-pipeline/src/output.rs` | Same install-into-archive logic; archive layout owned by ABS but pipeline knows the convention |
| `AppState` v2 | rewrite `wundler-abs/src/state.rs` | `manifest: Arc<RwLock<Arc<ChunkManifest>>>` (double-Arc snapshot pattern); `archive: Arc<ManifestArchive>`; `reload_lock: Arc<Mutex<()>>` |
| `swap_to(build_id)` | extend `AppState` | Load from archive, validate, atomic swap |
| `GET /versions` | extend `wundler-abs/src/server.rs` | Return `Vec<ArchiveEntry>` from `archive.list()` |
| `POST /select` | extend `wundler-abs/src/server.rs` | `{ "build_id": "..." }` → `swap_to()` → return new state. **Bound to 127.0.0.1 by default.** |
| `POST /reload` | extend `wundler-abs/src/server.rs` | Re-read `current` pointer; useful when pipeline updated it externally |
| `GET /cdn-self-check` | new in `server.rs` | Returns expected upstream cache-header policy + diagnostic info |
| `wundler diagnose cdn` | new in `wundler-cli/src/main.rs` | Fetches live URLs, compares headers against `/cdn-self-check` expectations |
| SW build_id reporting | extend `crates/wundler-sw/src/sw.ts` + `wundler-abs/src/types.rs::TelemetryEvent` | SW echoes `last_seen_build_id` in `POST /manifest` body; ABS records it |
| Optional file watcher | extend `server.rs` | Opt-in via `wundler.toml`; default OFF (mechanism not policy — let the operator/CI/CD trigger reloads explicitly) |

#### New Rust shape (concrete)

```rust
// crates/wundler-abs/src/archive.rs
pub struct ManifestArchive {
    pub archive_dir: PathBuf,
    pub retention: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    pub build_id: String,
    pub installed_at_ms: u64,
    pub bytes: u64,
    pub is_current: bool,
}

impl ManifestArchive {
    pub fn open(dir: &Path, retention: usize) -> Result<Self>;
    pub fn list(&self) -> Result<Vec<ArchiveEntry>>;             // sorted newest-first
    pub fn load(&self, build_id: &str) -> Result<ChunkManifest>;
    pub fn install(&self, m: &ChunkManifest) -> Result<ArchiveEntry>;  // also prunes
    pub fn set_current(&self, build_id: &str) -> Result<()>;     // atomic symlink swap
    pub fn current(&self) -> Result<Option<String>>;             // resolves current → build_id
    fn prune(&self) -> Result<Vec<String>>;                      // returns pruned build_ids
}
```

```rust
// crates/wundler-abs/src/state.rs (rewritten)
#[derive(Clone)]
pub struct AppState {
    // Double-Arc: handlers clone the inner Arc and release the read lock
    // immediately, so reads operate on an immutable snapshot and never
    // block writers — and writers swap by replacing the inner Arc.
    manifest: Arc<RwLock<Arc<ChunkManifest>>>,
    archive: Arc<ManifestArchive>,
    reload_lock: Arc<Mutex<()>>,
    cdn_base_url: Arc<String>,
    ttl_seconds: u64,
}

impl AppState {
    pub fn snapshot(&self) -> Arc<ChunkManifest> {
        // O(1) read: clone Arc, drop guard
        let g = self.manifest.blocking_read();   // or `.read().await` in async ctx
        Arc::clone(&g)
    }

    pub async fn swap_to(&self, build_id: &str) -> Result<SwapReport> {
        let _serialize = self.reload_lock.lock().await;
        let new = self.archive.load(build_id)?;
        validate(&new, build_id)?;               // build_id matches recomputed hash
        self.archive.set_current(build_id)?;
        let mut w = self.manifest.write().await;
        let previous = (*w).build_id.clone();
        *w = Arc::new(new);
        Ok(SwapReport { previous, current: build_id.into() })
    }

    pub async fn reload_from_current(&self) -> Result<SwapReport> {
        let bid = self.archive.current()?.ok_or_else(|| anyhow!("no current"))?;
        self.swap_to(&bid).await
    }
}
```

```rust
// crates/wundler-pipeline/src/build_id.rs  (new)
pub fn compute_build_id(manifest: &ChunkManifest) -> String {
    let bytes = manifest.canonical_bytes();   // deterministic
    let digest = sha2::Sha256::digest(&bytes);
    hex::encode(&digest[..8])                  // 16 hex chars
}
```

#### HTTP API additions

```
GET  /versions
     → 200 OK
     [ { build_id, installed_at_ms, bytes, is_current }, ... ]

POST /select
     body: { "build_id": "a1b2c3d4e5f6a7b8" }
     → 200 OK { "previous": "...", "current": "a1b2..." }
     → 404 if build_id not in archive
     → 409 if a swap is already in progress

POST /reload
     body: {}
     → 200 OK { "previous": "...", "current": "..." }
     → 200 OK { "previous": "x", "current": "x" } if no change

GET  /cdn-self-check
     → 200 OK
     {
       "expected_headers": {
         "/chunks/*.js":    { "cache-control": "public, max-age=31536000, immutable" },
         "/manifest.json":  { "cache-control": "no-cache, must-revalidate" },
         "/index.html":     { "cache-control": "no-cache, must-revalidate" },
         "/sw.js":          { "cache-control": "public, max-age=0, must-revalidate" }
       },
       "current_build_id": "a1b2..."
     }
```

#### Data flow

```
wundler build ──► ChunkManifest ──► compute_build_id → "a1b2..."
                  │
                  └─► write_manifest(out_dir/manifest.json)
                        │
                        └─► archive.install(manifest)
                              ├─► tmp write + rename → archive_dir/<build_id>.json
                              ├─► prune oldest
                              └─► (optional) set_current

ABS startup ─► ManifestArchive::open(dir, retention)
              └─► AppState built from archive.current()

Operator ──POST /versions──► archive.list() → JSON
Operator ──POST /select──► swap_to(bid)
                          ├─► archive.load(bid)
                          ├─► validate (recomputed build_id == bid)
                          ├─► archive.set_current(bid)
                          └─► RwLock swap (double-Arc)

Browser SW ──POST /manifest, last_seen_build_id="x"──►
   handler: snapshot = state.snapshot()    // Arc<ChunkManifest>, no lock held
            resp = compute_delta(&snapshot, &req, &cdn)
            telemetry.log(TelemetryEvent { last_seen_build_id, ... })
            return resp
```

#### What it optimizes for / sacrifices

- **Optimizes for:** Operational simplicity over the network; meets every stated goal exactly; minimum new abstractions; the swap path is short and reviewable.
- **Sacrifices:** Slightly more code than C1; ABS now owns archive state (which means startup includes a directory scan — bounded by retention, so O(10)); `/select` is an auth-shaped hole until Security Baseline lands.

---

### 2.3 Candidate 3 — Event-Sourced Manifest History

> Manifests stored as an immutable append-only log; `/select` replays to a point-in-time; supports future blue/green natively; more complex but extensible.

#### Components

| Component | File | Responsibility |
|---|---|---|
| `ManifestLog` | new `wundler-abs/src/log.rs` | Append-only `log.jsonl` of `LogEntry { build_id, parent_build_id, timestamp_ms, ref_path }` |
| `LogEntry` snapshot index | in-memory | Built from log on startup; supports replay |
| Named tags | `archive_dir/tags/<name>` symlinks | `current`, `prod`, `canary`, `blue`, `green` — multiple coexist |
| Compaction | new `compactor.rs` | After N entries, write snapshot summary; old entries archived |
| `POST /select` with tag support | extend `server.rs` | `{ build_id }` OR `{ tag }` OR `{ tag, build_id }` (point tag at build) |
| `GET /log` | extend `server.rs` | Return full or paginated log |

#### Data flow

```
wundler build ──► ChunkManifest ──► compute_build_id
                                    │
                                    └─► log.append(LogEntry {
                                          build_id,
                                          parent_build_id: previous_head,
                                          timestamp_ms,
                                          ref_path: "by-hash/<bid>.json"
                                        })
                                        │
                                        └─► tag.set("current", build_id)

Operator ──POST /select──► tag.set("current", build_id)
                          │
                          └─► swap from tag

Future blue/green:
   - ABS instance A has tag "blue" → its current
   - ABS instance B has tag "green" → its current
   - LB routes by header
   - Both share the same log
```

#### What it optimizes for / sacrifices

- **Optimizes for:** Long-term extensibility; audit trail; native multi-tag (blue/green/canary); time-travel debugging.
- **Sacrifices:** Significant additional state-machine complexity (log writes need fsync ordering; partial writes need recovery; compaction is a real subsystem). Solves problems we have explicitly deferred (G7). The state machine becomes a maintenance burden that pays off only when blue/green ships, and blue/green doesn't need event sourcing — it needs two ABS processes pointing at two builds.

#### Why this is probably the wrong shape today

Event sourcing is the right answer when you need **replay, time-travel, or branching history**. We need none of these. We need "swap a pointer atomically and keep the last 10 around." The log is solving a problem we have only hypothesized.

---

## 3. Eight-Dimension Tradeoff Matrix

| Dimension | C1 — Minimal | C2 — Full operator (RECOMMENDED) | C3 — Event-sourced |
|---|---|---|---|
| **Latency** | ✅ Best. File-watcher + symlink read. No HTTP work for swap. | ✅ Equal to C1 for the read path; swap is one extra HTTP round-trip but operator-driven (not on hot path). | ⚠️ Equal read path, slower swap (log append + fsync). |
| **Complexity** | ✅ Smallest delta: ~150 LoC of new code, no new endpoints. | 🟡 ~500 LoC; 4 new endpoints; `ManifestArchive` is one bounded module. | ❌ ~1500+ LoC; log writer, compactor, tag manager, recovery. Real state machine. |
| **Reliability** | 🟡 Filesystem atomicity is fine; rollback path is human-shell-on-host — error-prone under stress. | ✅ Mechanism is testable (RED→GREEN), rollback is one HTTP call. Idempotent /select. | 🟡 More moving parts to fail; log corruption is a new class of failure. |
| **Cost (build/run)** | ✅ Negligible: 10 × small JSON files on disk. | ✅ Same. Bounded by retention. | ⚠️ Log grows unbounded without compaction; compaction is itself overhead. |
| **Security** | 🟡 No /select endpoint = no auth surface. Good now, bad later (host shell access for rollback is its own auth boundary, usually weaker). | 🟡 /select **must** bind to 127.0.0.1 by default. Becomes proper-authenticated when Security Baseline lands; minimum-viable today. | 🟡 Same /select surface plus log endpoint. More to authenticate. |
| **Scalability** | ✅ Stateless except for archive dir. | ✅ Same. Archive is bounded by retention. | 🟡 Log size grows. Compaction is mandatory at scale. |
| **Reversibility** | ⚠️ Rollback requires shell access. Reverses a *deployment* in seconds, but only from on the box. | ✅ Rollback is one HTTP call from anywhere with network access to /select. Strict superset of C1. | ✅ Same operator API as C2. Same reversibility. |
| **Org fit** | 🟡 OK for solo dev. Bad for any team with an SRE who is not the developer. | ✅ Fits a small team because the API is small. Fits a large team because the API is sufficient. | ❌ Demands ongoing investment in a subsystem nobody asked for. |

---

## 4. RECOMMEND

### Recommended: **Candidate 2 — Full Operator Control**

#### Reasoning

**C2 is the smallest design that satisfies every stated goal.**

- G1 (deterministic build_id), G3 (atomic hot reload), G5 (SW reports build_id), G6 (CDN policy) are present in all three candidates. G2 (archive) is present in C1 and C2.
- **G4 (rollback via POST /select) eliminates C1.** The goal explicitly says "via `POST /select`." C1's "via shell on the ABS host" is a different capability with a different blast radius. Under incident conditions, requiring host shell access is exactly when it goes wrong.
- **G7 (blue/green deferred) eliminates C3.** C3's event-sourced log earns its complexity by enabling features we have *explicitly deferred*. The right time to pay that complexity is when blue/green is on the active roadmap — and even then, the typical blue/green deployment runs two ABS instances pointed at two builds; it does not require event sourcing.

#### Linux philosophy alignment

> Mechanism, not policy.

ABS provides:

- **Mechanism — versioned manifest store:** archive a build, list builds, swap to a build, report current build.
- **Mechanism — atomic swap:** in-flight requests complete against the prior snapshot (double-Arc pattern), the next request sees the new one. The mechanism is correct regardless of *when* it's invoked.

The operator chooses the **policy**:

- "Roll back on error rate > X%" — operator's CI/CD calls /select.
- "Canary 10% of traffic to next build" — operator runs two ABS instances and lets the load balancer split.
- "Blue/green" — same as canary, with header-based routing instead of percentage.
- "Auto-deploy newest" — operator's pipeline calls /reload after install. Default OFF in ABS.

ABS does not implement any of these policies. It exposes the **smallest set of primitives that lets the operator build all of them**.

#### What we explicitly will not build

- No automatic rollback on error rate. That's Observability + a policy decision.
- No automatic deploy of newest manifest. Default behavior is **stay where you are**; the operator must POST /select or /reload.
- No traffic routing. That's a load balancer / reverse proxy concern.
- No event log. C3 is a future option if and only if we ever need replay or branching.
- No /select authentication beyond bind-to-loopback. Security Baseline (next subsystem) handles auth.

---

## 5. RISKS

### Risks, ranked by severity

#### R1 — `/select` without auth (Critical until Security Baseline lands)

`POST /select` is a network-reachable rollback knob. Without auth, anyone who can reach ABS can roll the production app back to an arbitrary archived build, including a deliberately-vulnerable one.

**Mitigation in this design:**

- `POST /select`, `POST /reload`, `GET /versions`, `GET /cdn-self-check` bind to a **separate listener on `127.0.0.1`** by default. Public listener serves only `POST /manifest`, `GET /health`, `GET /sw.js`.
- New `wundler.toml` section:
  ```toml
  [abs.operator]
  bind = "127.0.0.1:9090"          # default
  # bind = "0.0.0.0:9090"          # explicit opt-in, required for remote rollback
  ```
- ABS logs `WARN` on every `/select` and `/reload`.
- Security Baseline replaces this with mTLS + Ed25519-signed select tokens. The mechanism here is forward-compatible.

#### R2 — Disk fill from archive accumulation (High; easy mitigation)

Each manifest is small (tens of KB typically), but a runaway pipeline that builds every commit could accumulate fast.

**Mitigation:**

- Default retention = 10 builds. Configurable via `[abs.archive] retention = N`.
- `ManifestArchive::install` prunes **before** writing, so steady-state disk use is bounded.
- ABS logs the archive directory size on startup; logs `WARN` if total > 100 MB (sanity threshold — a normal archive should be < 1 MB).
- `wundler diagnose archive` CLI command reports size, oldest entry, count.

#### R3 — CDN misconfiguration (High; silent)

If the CDN puts `Cache-Control: max-age=...` on `manifest.json`, clients never see new builds. If it puts `no-cache` on `/chunks/*.js`, performance regresses to "everyone re-downloads everything every request."

**Mitigation:**

- `GET /cdn-self-check` returns the expected upstream cache-header policy as authoritative JSON.
- `wundler diagnose cdn` CLI fetches the live CDN URLs (configured in `wundler.toml`) and diffs their headers against `/cdn-self-check`. Exits non-zero on mismatch.
- Designed to run in staging CI; gates promotion to prod.
- Documented separately in `docs/runtime/cdn-cache-policy.md` with copy-paste configs for the common CDNs we expect to use.

#### R4 — Hot reload race condition (Medium; eliminated by design)

The naive risk is: ABS reads the manifest file while the pipeline is mid-write. With this design:

- Pipeline always writes via temp file + `rename(2)`. POSIX guarantees the rename is atomic.
- ABS only ever reads through the `current` symlink. Symlink swap is also atomic.
- Readers operate on `Arc<ChunkManifest>` snapshots cloned out of `RwLock` then released. A swap in progress does not stall any reader and does not produce a torn read.
- The `reload_lock: Arc<Mutex<_>>` in `AppState` serializes concurrent `/select` calls so two operators can't race each other.

#### R5 — Service Worker stuck on old build_id (Medium; expected SW lifecycle, must document)

The SW update model is: new SW found → install → wait → activate when no clients are open. This is not a bug in our design; it's the SW spec. The risk is operator surprise: "I rolled back five minutes ago, why is my browser still on the old build_id?"

**Mitigation:**

- Document this in the design doc (this section) and operator runbook.
- `compute_delta` already does the correct thing: stale `build_id` → ignore client cache claims → return full chunk set. Stale SW degrades to "fetches everything" — slow but correct. No incorrect chunks ever served.
- `notifyBuildIdChanged` already postMessages clients; SW consumer code can choose to force-reload the page on build_id change. Default behavior: announce, don't force.

#### R6 — `build_id` non-determinism / collisions (Medium → Low with canonical serialization)

If `compute_build_id()` is not deterministic, two identical builds produce different IDs and the whole cache-trust mechanism leaks bandwidth.

**Mitigation:**

- `canonical_bytes()` is the canonicalization spec:
  - `chunks` sorted by `id` ascending.
  - Within each chunk, `modules` sorted by hex content hash ascending.
  - `entry_chunks` serialized via `BTreeMap` (sorted keys); inner Vec kept in insertion order (semantically meaningful — load order).
  - `module_index` serialized via `BTreeMap`.
  - No timestamps, no random nonces, no file paths.
- Unit test: serialize a fixed manifest twice and assert byte equality.
- Property test: build the same fixture twice and assert `build_id` equality.
- Collisions: SHA-256 truncated to 64 bits ≈ 1 in 18 quintillion. At 10 retained builds, collision probability is negligible. If it ever happened, `archive.install` would detect (file already exists with this build_id) and treat as no-op — which is correct, because the content *is* the same.

#### R7 — File watcher misfire (Low; opt-in by default)

`notify`-based watchers can deliver duplicate events, miss events on some filesystems, or fire on metadata-only changes.

**Mitigation:**

- File watcher is **opt-in** via `[abs.archive] watch = true`. Default OFF. The canonical mechanism is `POST /reload`.
- When enabled, watcher serves only as "ping ABS to call `/reload` internally." It does not bypass the swap protocol — it goes through the same code path.
- Debounced 500ms to coalesce duplicates.

#### R8 — Archive corruption (Low; tolerable)

If an archive file is partially written or corrupted, `archive.load(build_id)` fails. `/select` returns 500. ABS does not swap; previous build continues serving.

**Mitigation:**

- Validation on load: parsed manifest's recomputed `build_id` must equal the filename's `build_id`. Catches truncation and bit-flips.
- `GET /versions` includes a `valid: bool` field per entry, surfacing this before operator attempts /select.

---

## 6. SIMPLEST CREDIBLE ALTERNATIVE

> Implement deterministic `build_id` + `canonical_bytes()` + `POST /reload` (re-reads `manifest_path` from config). No archive, no `/versions`, no `/select`. Operators "roll back" by replacing `manifest.json` on disk with a prior copy and calling `POST /reload`.

This is approximately **two days of work**, completely subsetted by C2, and meaningfully unblocks Security Baseline (which needs a stable `build_id` for signing) and Observability (which needs a stable `build_id` for per-build metrics). The archive and `/select` can be added later without revisiting any decisions made in this phase.

It does not satisfy G2 or G4 on its own. It is the right *interim* step if scheduling forces a smaller scope than C2, and it commits to no decision that C2 would have to undo.

---

## 7. Open Questions

| # | Question | Default if unanswered |
|---|---|---|
| Q1 | Should `compute_build_id` mix in the `git_commit` SHA, or stay purely content-derived? | Stay purely content-derived. Record `git_commit` separately in `ArchiveEntry`. Two identical content builds *are* the same build for cache purposes; the git commit is operator metadata, not identity. |
| Q2 | Should the file watcher be on by default in dev (via `wundler dev`) and off in prod? | Yes. Two `wundler.toml` profiles: `[abs.archive.dev] watch = true`, `[abs.archive.prod] watch = false`. |
| Q3 | Should `/select` return immediately or after the swap completes? | After. The swap is bounded (< 50ms for a typical manifest) and the operator wants confirmation, not optimism. |
| Q4 | What should `is_current` return during a swap-in-progress? | The previous build_id. The new build_id becomes `is_current` only after `RwLock::write` completes. |
| Q5 | Do we expose `last_seen_build_id` from clients as a Prometheus-style metric? | Yes, but that's Observability's job, not this subsystem's. We just need to make sure the data is recorded in telemetry — Observability builds the dashboards. |

---

## 8. Acceptance Criteria

A working implementation of C2 must demonstrate, with tests:

1. **Determinism:** Two builds of the same source tree produce byte-identical `manifest.json` and identical `build_id`. (Unit test on `canonical_bytes`; integration test in `wundler-bench` against the synthetic app generator.)
2. **Archive bound:** After 15 sequential builds with `retention = 10`, exactly 10 manifests exist in `archive_dir`. (Pipeline integration test.)
3. **Atomic swap:** While a long-running `POST /manifest` is in flight, a concurrent `POST /select` completes; the in-flight request returns the prior build's response; the next `POST /manifest` returns the new build's response. (Tokio-based concurrent test in `wundler-abs/tests/`.)
4. **Rollback safety:** After `POST /select` to a prior build_id, clients that previously cached chunks from a different build send a stale `build_id` and receive the full chunk set (no stale-cache poisoning). (Already tested by `manifest_delta_test.rs::stale_build_id_ignores_client_cache` — add a rollback-flavored variant.)
5. **Loopback default:** With default `wundler.toml`, `curl -X POST http://0.0.0.0:9090/select` from another host fails to connect. (Integration test.)
6. **CDN self-check:** `wundler diagnose cdn` exits 0 when CDN serves expected headers and non-zero with a structured diff when it doesn't. (CLI test with a mock CDN.)
7. **SW build_id roundtrip:** Service Worker reads `build_id` from `manifest.json` at install, sends it as `last_seen_build_id` in subsequent `POST /manifest` calls, and the telemetry log contains that field. (SW unit test + ABS telemetry test.)

---

## 9. Implementation Sequencing (for plan-writer)

Suggested decomposition into RED→GREEN tasks, ordered for minimum risk:

1. `canonical_bytes()` + `compute_build_id()` — pure functions, fully testable in isolation. *No behavior change yet.*
2. Pipeline wires `compute_build_id()` into `BuildPipeline::build()`. Replace the existing placeholder. *Verify with bench.*
3. `ManifestArchive` struct + `install`/`list`/`load`/`set_current`/`prune`. *Tests use `tempfile`.*
4. Pipeline writes into archive after `write_manifest`. *Verify retention bound.*
5. `AppState` v2 with double-Arc pattern and `ManifestArchive`. *Verify reader-during-writer test.*
6. `GET /versions`. *Read-only, low risk.*
7. `POST /reload`. *Single-purpose, easy to test.*
8. `POST /select`. *Includes loopback-bind default.*
9. `GET /cdn-self-check` + `wundler diagnose cdn` CLI.
10. SW `last_seen_build_id` in `POST /manifest` body + telemetry capture.
11. Optional: file watcher behind `[abs.archive] watch = true`. Default off.
12. Documentation: `docs/runtime/cdn-cache-policy.md`, `docs/runtime/operator-runbook.md`.

Each step is independently shippable and independently revertible. Step 1 alone unblocks Security Baseline's signing work and Observability's per-build metrics.
