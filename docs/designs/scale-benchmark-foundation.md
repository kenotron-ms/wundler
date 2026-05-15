# Scale Benchmark Foundation — Design

**Status:** Design accepted, ready for implementation planning
**Owner:** Wundler core
**Depends on:** None (orthogonal to VRC and Security Baseline)
**Blocks:** Performance work (HMR, pre-bundling, incremental graph) — measurement must precede optimization claims

---

## ANALYZE — System Map

### Goals

| ID | Goal |
|---|---|
| G1 | Generate a synthetic corpus that matches a real repo's *structural shape* (file/extension/byte/line/manifest counts) within tolerance, with zero LLM tokens |
| G2 | Make the corpus **Certifiably Verifiable** — a mechanical check answers "does this match the target?" yes/no |
| G3 | Run CAS / ABS / analysis benchmarks at 36k-file scale that are credibly representative of large-web-app stress |
| G8 | A `large-web-app-5x.v1.json` profile exists as the aspirational scale target (Teams-class monorepo, ~5× the 1x profile) for future benchmark validation |
| G4 | Capture and reproduce *graph shape* (degree distributions, SCCs, alive/dead ratios) without copying the real graph |
| G5 | Keep the existing simple `mod_00000` / skip-7 pattern available as a preset for backward-compat benchmarks |
| G6 | Ship the artifact (the committed profile) without leaking source content, identifiers, or paths from the real repo |
| G7 | Generation is fully deterministic: `(profile, seed) → byte-identical tree` |

### Verification Properties → Required Components

| Property | What it asserts | Required component |
|---|---|---|
| **V1 Structural conformance** | Re-profiling the generated tree produces a `RepoScaleReport` that matches the target profile within tolerances | `verify-corpus`: re-run `repo_scale::measure_repo`, diff against profile, produce `ConformanceReport` |
| **V2 Determinism** | Same `(profile, seed)` → byte-identical tree (SHA-256 of sorted file list + concatenated contents) | Deterministic PRNG (`StdRng` from `rand`, seeded), sorted directory traversal, no time/UUID in content |
| **V3 Syntactic validity** | Every generated `.ts`/`.tsx` parses with SWC, every `.json` parses with `serde_json` | Archetype templates produce valid syntax by construction; `verify-corpus` runs SWC-parse + JSON-parse over a sample |
| **V4 Graph validity** | Every internal import specifier resolves to a file that exists in the generated tree (no dangling refs) | Generator emits imports only against already-emitted files; verifier re-runs `build_adjacency()` and asserts no `Unresolved` warnings |
| **V5 Benchmark stability** | Across 5 runs of `analysis_bench` on the corpus, the coefficient of variation (stddev / mean) of cold-build wall time is < 2% | Existing `analysis_bench` runner + a new `--repeat N --check-cv` flag |

### What Already Exists (Build On)

| Component | Location | Used by this design |
|---|---|---|
| `RepoScaleReport` + `measure_repo` | `crates/wundler-bench/src/repo_scale.rs` | **The profile schema.** Generator reads it, verifier re-measures into it. |
| `RepoScaleReport.schema_version = 1` | same | Profile schema versioning is free; we add `compat_min_schema_version` |
| `SyntheticApp::generate_at` | `crates/wundler-bench/src/synthetic.rs:40` | Reuse the entry-point + `wundler.toml` boilerplate; replace `generate_module()` body |
| `bump_bench_version` / `__BENCH_VERSION__` | same:434 | Preserve — needed for warm-build / churn benchmarks |
| `build_adjacency()` + `tarjan_sccs()` | `crates/wundler-graph/src/graph.rs` | Verifier (V4) and `graph-profile` consume these |
| `GraphAnalyzer` + `AnalysisStats` | `crates/wundler-graph/src/analyzer.rs` | `graph-profile` runs full pipeline; produces `GraphShapeProfile` |
| Existing benchmark runners | `analysis_bench.rs`, `cas_bench.rs`, `abs_bench.rs` | Unchanged. They consume `&Path` and don't care how the corpus was produced. |
| `presets/uniform.json` placeholder | (to be created) | Captures the current `mod_00000`/skip-7 pattern as a frozen `GraphShapeProfile` for backward-compat runs |

### What Must Be Built

| Component | New file | Responsibility |
|---|---|---|
| `BenchProfile` type | `crates/wundler-bench/src/profile.rs` | Newtype alias around `RepoScaleReport` + generation hints (`seed`, `tolerances`) |
| Archetype library | `crates/wundler-bench/src/archetypes/` (module) | Hand-written content templates with line/byte-padding strategy |
| Corpus generator | `crates/wundler-bench/src/corpus_gen.rs` | Allocates files to dirs, picks archetype per extension, writes tree |
| Corpus verifier | `crates/wundler-bench/src/corpus_verify.rs` | Re-runs `measure_repo`, parses sample, returns `ConformanceReport` |
| Graph-shape profile | `crates/wundler-bench/src/graph_profile.rs` | Walks a `GraphAnalyzer` result, emits `GraphShapeProfile` (numbers + histograms) |
| Graph-driven generator | `crates/wundler-bench/src/graph_gen.rs` | Configuration-model graph sampling; replaces hard-coded `import_deps()` in synthetic.rs |
| Graph similarity | `crates/wundler-bench/src/graph_similarity.rs` | Scalar tolerance + EMD on histograms |
| CLI subcommands | `crates/wundler-bench/src/main.rs` | `generate-corpus`, `verify-corpus`, `graph-profile`, `graph-similarity` |
| Committed profiles | `crates/wundler-bench/profiles/` | `large-web-app.v1.json`, `large-web-app-small.v1.json`, `presets/uniform.v1.json` |

### Key Dependency: A ⟶ B

Part B (graph generator) needs Part A (a corpus on disk) to be meaningful, because:

1. A `GraphShapeProfile` is *computed by running the existing analyzer* over a real corpus. There is no other way to obtain it.
2. The graph generator emits an `import_deps()`-equivalent mapping, but those imports are stamped *into* archetype templates. Without the archetype library (Part A) there is nothing to stamp them into.

So the build order is:

```
Part A  ────────────────────────────►  CAS/ABS/analysis at 36k scale
   │
   └──► graph_profile (A.product → B.input)
           │
           └──► Part B  ────────────►  realistic graph stress
```

Part A alone is enough to unblock **all file-count-driven** benchmarks at scale. Part B refines the *shape* of the work the analyzer does over that corpus.

### Failure Modes (Without This Design)

| ID | Failure | Impact |
|---|---|---|
| F1 | Bench runs against `N=10k` synthetic with one mega-hub | Numbers don't predict 36k real-repo behavior; "optimization" wins on synthetic regress on real |
| F2 | Tweaking synthetic.rs ad-hoc to "look bigger" | Drift between what bench measures and what it claims to measure; benchmark loses scientific value |
| F3 | Generate corpus with LLM | Token costs prohibitive (millions of files × hundreds of lines = $$); non-deterministic; legal concerns |
| F4 | Copy the real source tree under NDA | Cannot ship; cannot commit; cannot CI |
| F5 | Generate but never verify | Generator drifts from profile silently; you don't find out until a benchmark gives the wrong answer |

---

## DESIGN — Three Candidates (Part A)

### Candidate 1 — Profile-as-Constraint (extended knobs)

**Shape:** Keep current `synthetic.rs`. Add explicit `SyntheticOptions { n_modules, n_packages, n_apps, big_json_count, big_json_size_kb, side_effect_density, ... }`. Caller manually picks numbers to roughly match the target.

**Files:**
```
crates/wundler-bench/src/synthetic.rs   ← extended in place
crates/wundler-bench/src/options.rs     ← new: SyntheticOptions
```

**Flow:**
```
SyntheticOptions  ──►  SyntheticApp::generate_at  ──►  tree on disk
                                                          │
                                                          └──► (no verifier; caller eyeballs)
```

**Tradeoffs (8-dim):**

| Dim | Score | Notes |
|---|---|---|
| Simplicity | ★★★★★ | Smallest diff to current code |
| Performance | ★★★★ | Pure I/O, no profile parsing |
| Reliability | ★★ | No oracle — silent drift between knobs and target |
| Scalability | ★★ | Adding a new attribute = new knob = new caller change |
| Cost | ★★★★★ | ~200 LOC |
| Operability | ★★ | "Did this match?" answered by hand |
| Security | ★★★★★ | Pure local generation |
| Evolvability | ★★ | No schema; every consumer learns every knob |

**Optimizes:** speed-to-first-bench.
**Sacrifices:** verifiability, traceability of "what does this corpus represent."

### Candidate 2 — Profile-as-Contract (**RECOMMENDED**)

**Shape:** The `RepoScaleReport` IS the `BenchProfile`. Generator reads the report (extension counts, dir distribution, manifest counts, workspace layout) and allocates files to satisfy it. Verifier re-runs `measure_repo()` on the output and diffs. **Same Rust type on both sides.** Profile is spec AND oracle.

**Files:**
```
crates/wundler-bench/src/profile.rs        ← BenchProfile = wraps RepoScaleReport + GenHints
crates/wundler-bench/src/archetypes/       ← module dir
    mod.rs
    ts_module.rs
    tsx_component.rs
    json_config.rs
    json_data.rs
    package_json.rs
    tsconfig_json.rs
    markdown.rs
    cmdscript.rs
crates/wundler-bench/src/corpus_gen.rs     ← allocates files, picks archetypes, writes tree
crates/wundler-bench/src/corpus_verify.rs  ← re-measures + sample-parses; emits ConformanceReport
crates/wundler-bench/profiles/
    large-web-app.v1.json
    large-web-app-small.v1.json           ← 10% scale for CI
    presets/uniform.v1.json
```

**Type sketch:**

```rust
// profile.rs
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchProfile {
    /// Schema version of the embedded RepoScaleReport. Generator refuses
    /// profiles with schema_version > PROFILE_SCHEMA_VERSION.
    pub profile_schema_version: u32,
    /// The structural target — same type produced by repo_scale::measure_repo.
    pub target: RepoScaleReport,
    /// Generation hints (NOT part of the target — purely how to fill it).
    pub gen: GenHints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenHints {
    pub seed: u64,
    pub tolerances: Tolerances,
    /// Optional graph-shape profile to drive imports. None = use uniform preset.
    pub graph_shape: Option<GraphShapeProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tolerances {
    /// Allowed fractional drift for file_count per extension (default 0.02).
    pub file_count_pct: f64,
    /// Allowed fractional drift for byte_count per extension (default 0.05).
    pub byte_count_pct: f64,
    /// Allowed fractional drift for line_count per extension (default 0.05).
    pub line_count_pct: f64,
    /// Allowed absolute drift for manifest counts (default 0).
    pub manifest_abs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub profile_schema_version: u32,
    pub target_summary: ProfileSummary, // tiny — counts only, for diff readability
    pub actual_summary: ProfileSummary,
    pub checks: Vec<Check>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,           // "ext:ts.file_count", "workspace.packages_immediate_subdirs", ...
    pub target: f64,
    pub actual: f64,
    pub tolerance: f64,         // either fractional or absolute, depending on check
    pub status: CheckStatus,    // Pass | Fail | Skipped { reason }
}
```

**Flow:**
```
real repo  ──► measure_repo() ──► RepoScaleReport  ──┐
                                                     ├──► BenchProfile  ──► generate-corpus  ──► tree on disk
                                            seed ────┘                                              │
                                                                                                    │
                                                                            verify-corpus ◄─────────┘
                                                                            │
                                                                            ├── measure_repo() again
                                                                            ├── sample-parse SWC + serde_json
                                                                            ├── re-run build_adjacency()
                                                                            └─► ConformanceReport (pass/fail)
```

**Why "profile is spec AND oracle":** The same `RepoScaleReport` Rust type is used to capture the target and to validate the output. There is no possibility of spec/check schema drift — if `RepoScaleReport` gains a field, the verifier checks it next run.

**Tradeoffs:**

| Dim | Score | Notes |
|---|---|---|
| Simplicity | ★★★★ | Single type contract; small archetype library |
| Performance | ★★★★ | Generation is ~filesystem-write-bound; verification is one extra `measure_repo` pass |
| Reliability | ★★★★★ | V1–V5 mechanically checkable |
| Scalability | ★★★★★ | New profile field → checker picks it up automatically via serde |
| Cost | ★★★ | ~1500 LOC across profile/gen/verify/archetypes |
| Operability | ★★★★★ | "Did this match?" answered by `verify-corpus` exit code |
| Security | ★★★★★ | Profile is counts-only — no paths, no identifiers, no source content |
| Evolvability | ★★★★★ | Profile schema versioned; `compat_min_schema_version` gate refuses unknown future profiles |

**Optimizes:** verifiability, traceability, schema-stability.
**Sacrifices:** marginal LOC cost vs. C1; cannot match real per-file path structure (only aggregates).

### Candidate 3 — Anonymized Skeleton Replay

**Shape:** Capture every real file as a `FileSkel { path_template, ext, byte_count, line_count }`. Stamp an archetype into each path entry, padded to its exact byte/line target.

**Files:**
```
crates/wundler-bench/src/skeleton.rs      ← SkeletonProfile { files: Vec<FileSkel>, seed }
crates/wundler-bench/src/archetypes/      ← same as C2
crates/wundler-bench/src/corpus_gen.rs    ← reads skeleton, stamps archetypes
crates/wundler-bench/profiles/
    large-web-app.v1.skel.json           ← LARGE: 36k file rows
```

**Flow:**
```
real repo ──► capture-skeleton ──► SkeletonProfile (36k rows of {path-template, ext, bytes, lines})
                                          │
                                          └──► generate-corpus ──► byte-exact match for every file
```

**Tradeoffs:**

| Dim | Score | Notes |
|---|---|---|
| Simplicity | ★★★ | Concept is simple, but per-file accounting is tedious |
| Performance | ★★★★ | Same generation cost as C2 |
| Reliability | ★★★★★ | Conformance trivial (1:1 file mapping) |
| Scalability | ★★★ | 36k rows × N projects = blob bloat |
| Cost | ★★★ | Comparable to C2 |
| Operability | ★★★★ | Easy to read, but file is 5+ MB |
| Security | ★★ | **Path-template leakage risk.** Even anonymized paths reveal directory structure shape, which can leak proprietary layout (e.g. `apps/<3-letter-codename>/...`) |
| Evolvability | ★★ | Tied to a snapshot — any code reorg in the real repo invalidates the profile |

**Optimizes:** exact-replay fidelity.
**Sacrifices:** profile size; privacy of directory structure; profile becomes a snapshot, not a portable shape descriptor.

---

## DESIGN — Part B Graph Generator

### What the current rule-based approach (`uniform.json` preset) gets wrong

The current `synthetic.rs` rule:
- `mod_00000` is imported by everyone (one hub, in-degree N-1)
- Every other module imports `mod_{i-1}` and `mod_{i-7}` (out-degree exactly 3 except boundary modules)
- Every 20th module has a side effect; every 5th has a dead export

What this **does not** match in real repos:

| Aspect | Real repo (typical large TS monorepo) | Current synthetic |
|---|---|---|
| Out-degree distribution | Heavy-tailed; most modules import 2–5, a few barrel files import 50+ | Almost-constant 3 |
| In-degree distribution | Power-law-ish; a handful of utility modules with in-degree 100s–1000s; many leaves with in-degree 1 | Single peak at N-1 (mod_00000) + uniform 1–2 elsewhere |
| SCC count | Small but nonzero — barrel re-exports cause cycles | Zero (`(i-1, i-7)` is strictly backward-edged → DAG) |
| Dynamic import fraction | 1–5% of edges (lazy routes, code-split chunks) | 2 dynamic imports total in `main.tsx` regardless of N |
| Commons-module ratio | 5–15% of modules reached by ≥2 entries | Trivial — single entry, no commons |
| Depth | log-ish growth, wide middle layers | Linear chain N-deep |

The chunker's worst case (commons extraction phase) is **never exercised** by the current pattern. The analyzer's Tarjan SCC path is **never exercised** because the graph is strictly DAG-by-construction.

### Three approaches

**B1 — Keep rule-based, tune knobs**
Preserve `import_deps(i)`, add a few more rules (`mod_{i mod hubs}` for K hubs, a configurable skip stride). Cheap, but every distribution is still implicit and rigid.

**B2 — Configuration-model with degree-sequence sampling (RECOMMENDED)**
Read `GraphShapeProfile.out_degree_hist` and `in_degree_hist`. Sample a degree sequence for N modules. Use the Chung–Lu / configuration-model approach: each edge `(u → v)` is drawn with probability proportional to `out_deg(u) × in_deg(v)`. Reject self-loops; deduplicate. Then:
- Inject `scc_count` Tarjan cycles by randomly back-edging within a chosen subset
- Mark `dynamic_import_ratio` fraction of edges as `import()`
- Mark `commons_module_ratio` fraction of modules as reached from ≥2 entries (pick K entries from low-in-degree nodes)
- Mark `dead_ratio` fraction of modules as unreachable from any entry (separate disconnected component)

The output is a `Vec<ImportEdge { from: usize, to: usize, kind: Static|Dynamic }>` which is the input to the corpus generator's archetype-stamping step.

**B3 — Stochastic block model**
Model modules as belonging to K "packages," with high intra-block edge probability and low inter-block. More realistic for monorepos than configuration-model, but introduces another parameter the profile would have to capture (block count, intra/inter rates). Defer.

### Does SCC injection and dead-subgraph injection actually change the cost profile?

Yes — measurably:

| Operation | Cost on DAG (current synthetic) | Cost with realistic SCCs/dead |
|---|---|---|
| `tarjan_sccs()` | O(V+E) but trivial — every SCC is size 1 | O(V+E) with non-trivial path compression; ~5–15% slower in practice |
| `compute_reachability_with_sccs` | Touches only the path | Touches the SCC-collapsed view |
| `compute_dead_exports` | Trivial — DCE has no work | Non-trivial — must scan all dead modules' exports |
| `assign_chunks` commons phase | Empty (no module appears in ≥2 candidates) | **Major work** — N × E membership probes |

The chunker's commons-extraction phase is currently invisible to the bench. With realistic graph shape (multiple entries, shared utility hubs), it becomes a measurable phase. **This is exactly the kind of stress real bundlers see at scale.** B2 will move the needle on the bench output where B1 won't.

### Backward-compat preset

The current `synthetic.rs` rule produces a graph that, when profiled, yields a specific `GraphShapeProfile`. We capture that profile once, freeze it as `profiles/presets/uniform.v1.json`, and the new generator can reproduce it byte-equivalent. **No existing benchmark breaks.**

---

## RECOMMENDATION

**Part A: Candidate 2 — Profile-as-Contract.**

The reason is structural, not performance: in C2 the *spec and the oracle are the same Rust type*. There is exactly one schema (`RepoScaleReport`), there is exactly one place to add a field (the report), and the verifier inherits all new fields automatically. C1 has no oracle. C3's oracle is per-file equality, which is overkill and leaks path structure.

C2 is also the cheapest path to **V1–V5 all mechanically checkable**, which is the "Certifiably Verifiable" bar the user specifically asked for. With C1, V1 is not checkable at all. With C3, V2 is trivially satisfied but V6 (the unstated property: "I can ship this") fails because the skeleton file itself encodes proprietary structure.

**Part B: B2 — Configuration-model with degree-sequence sampling**, gated behind `GenHints.graph_shape`. If absent, the generator falls back to the `presets/uniform.v1.json` rule-equivalent (preserving the current synthetic.rs behavior for backward-compat benchmarks).

---

## ARCHETYPE LIBRARY DESIGN

**Constraint recap:** zero LLM tokens; pure Rust; deterministic; valid syntax by construction; boring is correct. The library is hand-written, lives in `crates/wundler-bench/src/archetypes/`, and exposes one function per file type.

### Padding strategy (the core trick)

Every archetype produces output in two halves:

1. **Semantic head** — fixed-shape, syntactically meaningful (imports, exports, types, JSX, JSON keys).
2. **Padding tail** — repeatable, valid-by-construction comments or array entries that grow until the file hits its target `byte_count` and `line_count`.

Padding is **trivial valid syntax that the SWC parser will skip past quickly**, ensuring it adds bytes/lines without distorting parse time disproportionately. Concretely:

| File type | Pad token (one per line) | Cost to parser |
|---|---|---|
| `.ts` / `.tsx` | `// pad-NNNNNNNN` (line comment) | O(byte), parser skips |
| `.json` (data) | `,"pad_NNNNNN": "p_NNNNNN_............"` (array or object entry) | O(byte), JSON parser still O(n) |
| `.md` | `Pad line NNNNNN.` | N/A |
| `.yaml` | `# pad-NNNNNN` | N/A |

The padding loop:
```rust
fn pad_to_target(
    out: &mut String,
    mut current_lines: u64,
    mut current_bytes: u64,
    target_lines: u64,
    target_bytes: u64,
    pad_line_fn: &dyn Fn(u64) -> String,
) {
    let mut i: u64 = 0;
    while current_lines < target_lines || current_bytes < target_bytes {
        let line = pad_line_fn(i);
        current_bytes += line.len() as u64 + 1; // +1 for '\n'
        current_lines += 1;
        out.push_str(&line);
        out.push('\n');
        i += 1;
        if i > target_bytes.saturating_add(1) { break; } // hard stop, defensive
    }
}
```

The generator picks per-extension *average* line/byte targets from the profile and applies them per file with a tight distribution (jitter ±10% via the seeded PRNG) so totals match while individual files vary.

### Minimum archetype list

For the reference profile's top extensions, **8 archetypes cover ~95% of files**:

| # | Archetype | Variants | Drives |
|---|---|---|---|
| 1 | `ts_module` | 3 variants: leaf (no imports), intermediate (imports + exports + class), barrel (re-exports only) | `.ts` (15k files / 707k lines). 90% intermediate, 5% leaf, 5% barrel |
| 2 | `tsx_component` | 2 variants: simple component, container component | `.tsx` (4.2k files / 636k lines) |
| 3 | `json_config` | 1 template: small key/value object with array-of-strings + nested-object | `.json` < 4 KB |
| 4 | `json_data` | 1 template: large array of `{ id, name, value, pad: "...." }` rows — padding rows scale to byte target | `.json` ≥ 4 KB — must hit the 543 MB total |
| 5 | `package_json` | 1 template: `name`, `version`, `dependencies`, `devDependencies`, `scripts` | `package.json` (293 files) |
| 6 | `tsconfig_json` | 1 template: `compilerOptions`, `include`, `exclude`, `references` | `tsconfig.json` (287 files) |
| 7 | `markdown` | 1 template: H1, paragraph, code block, list, paragraph (+ padding) | `.md` files |
| 8 | `cmdscript` / `yaml` / `sh` | 1 boilerplate per ext | `.sh`, `.yml`, etc — long tail |

**3 archetype variants for `ts_module` is the key complexity choice.** Without variants, every TS file would have identical fan-out structure and the analyzer would see a degenerate degree distribution. With 3 variants (leaf 0 imports, intermediate 2–5 imports, barrel 10–30 imports), the in/out-degree histogram naturally widens to match real-repo shape.

### Why this is enough

The user is right: this is **"contrived and useless as a benchmark for compiler output"** — the generated TS doesn't *compute* anything interesting. But for measuring the analyzer's wall-time per N modules, the chunker's commons-extraction cost per K entries, and the ABS delta-efficiency per churn fraction, **the wall-time is dominated by file I/O, AST parsing, and graph traversal, none of which care what the function bodies compute.** Boring is correct.

### Archetype variant selection (deterministic)

Given a file slot `(dir, ext, file_index)`:

```rust
let archetype = match ext.as_str() {
    "ts" => {
        let v = (seeded_hash(dir, file_index) % 100) as u32;
        if v < 5 { TsVariant::Leaf }
        else if v < 95 { TsVariant::Intermediate }
        else { TsVariant::Barrel }
    }
    "tsx" => TsxVariant::pick(seeded_hash(dir, file_index)),
    "json" if target_byte_count > LARGE_JSON_THRESHOLD => JsonVariant::Data,
    "json" => JsonVariant::Config,
    "package.json" => Archetype::PackageJson,
    "tsconfig.json" => Archetype::TsconfigJson,
    "md" => Archetype::Markdown,
    _ => Archetype::Generic,
};
```

`seeded_hash(dir, file_index)` is a `u64` derived from the profile seed; this guarantees the same file in the same dir picks the same variant across runs.

---

## RISKS (Ranked)

### R1 — File-size distribution within an extension (HIGH)

**Risk:** Profile says `json: 8553 files, 543 MB total`. The mean is 64 KB. But the real distribution is bimodal: 8000 small (~2 KB) configs + 553 large data dumps averaging 1 MB. If we generate 8553 files at ~64 KB each, the IO pattern, the parser cost profile, and the CAS hash-block distribution are all wrong.

**Mitigation:**
- `RepoScaleReport` currently aggregates per-extension only. **Extend it (schema v2) to add per-extension percentile points: `byte_count_p50`, `byte_count_p90`, `byte_count_p99`, `byte_count_max`.**
- Generator uses a bimodal or log-normal sampler fit to those percentiles when emitting per-file sizes.
- For schema-v1 profiles (the current `large-web-app.v1.json`), assume the worst-case bimodal split — call it out in `verify-corpus` warnings.

**Owner:** `repo_scale.rs` gets a new `percentiles` field; `corpus_gen` gets a `FileSizeSampler`.

### R2 — Archetype templates drifting from valid TS syntax (MEDIUM)

**Risk:** Someone edits `ts_module.rs` to add a new export form. It happens to produce a syntax error at certain seeds. The corpus generates, but `wundler analyze` crashes mid-bench.

**Mitigation:**
- `cargo test` runs `corpus_verify` against a tiny fixture profile (`profiles/test/tiny.v1.json` — 20 files) on every PR. V3 syntactic check (SWC parse) on 100% of generated files at this size is fast (<1s).
- At full scale, V3 samples 1% of files (~360 of 36k); samples are stratified by archetype variant.
- An additional `archetype-self-test` unit test asserts each archetype's output parses cleanly for 100 seeded inputs.

### R3 — Generated graph too disconnected; wrong alive/dead ratio (MEDIUM)

**Risk:** Configuration-model sampling can produce graphs where the entry-reachable subgraph is too small relative to `target.alive_ratio`. Bench measures DCE on a corpus that's 80% dead when target was 5%.

**Mitigation:**
- After sampling, the graph generator runs a fast BFS from the chosen entries; if `actual_alive_ratio / target_alive_ratio` is outside `[0.9, 1.1]`, **re-sample with edges rewired toward unreached nodes** (capped at 5 iterations).
- `verify-corpus` re-runs `GraphAnalyzer` and emits `actual_alive_ratio` in the `ConformanceReport` — V4 fails if outside tolerance.

### R4 — CI corpus generation too slow (MEDIUM)

**Risk:** Even at 10% scale (3600 files), generating + verifying on every PR adds minutes to CI.

**Mitigation:**
- `large-web-app-small.v1.json` targets 3600 files / ~54 MB. At ~10k files/sec generation rate (filesystem-write-bound), that's <1s to generate and ~2s to verify (one `measure_repo` pass + 1% SWC sample-parse).
- CI runs `generate-corpus --profile large-web-app-small.v1.json --out target/bench-corpus && verify-corpus target/bench-corpus --profile ...`. Total budget: <10s.
- Full-scale `large-web-app.v1.json` runs only on nightly or manual trigger (`make bench-full`).

### R5 — Profile schema version skew (LOW–MEDIUM)

**Risk:** We commit `large-web-app.v1.json` at `SCHEMA_VERSION = 1`. Six months later, `RepoScaleReport` is at v3. Old profile is ambiguous: missing fields, default to what?

**Mitigation:**
- `BenchProfile.profile_schema_version` is checked at load time.
- The generator declares `MIN_SUPPORTED = N` and `MAX_SUPPORTED = M` constants.
- Profiles outside the range produce a clear error: `"profile is schema v1 but generator requires v2..v3. Re-measure with: wundler-bench repo-scale --path <repo> --output json > new-profile.json"`.
- We commit profiles **with the schema_version they were measured at** — never silently upgrade.

### R6 — `verify-corpus` passes but bench wall-time still doesn't match real (LOW, unmeasurable)

**Risk:** All five V's pass, but real-repo CAS run takes 8s and synthetic-corpus run takes 3s. Profile is a faithful structural shadow that still isn't a faithful workload.

**Mitigation:** None pre-emptively — this is the **inherent gap between any synthetic and the real thing.** Document it: report alongside every benchmark run a "ground truth" run against the reference monorepo (when available locally) and publish the ratio `synthetic_time / real_time`. If the ratio is stable (e.g., synthetic is always 0.6× real), the benchmark is still useful for measuring *change* even if absolute numbers differ. This is the same compromise SPEC benchmarks make.

---

## SEQUENCING

### MVP (unblocks CAS/ABS at real scale): Part A skeleton, no graph generator

```
Step 1.  profile.rs                 ── BenchProfile = RepoScaleReport + GenHints
Step 2.  archetypes/ (all 8)        ── hand-written templates with pad_to_target
Step 3.  corpus_gen.rs              ── allocate files to dirs, pick archetypes, write tree
                                       (uses presets/uniform.v1.json behavior for imports)
Step 4.  corpus_verify.rs           ── V1 + V3 + V4 mechanical checks
Step 5.  CLI: generate-corpus, verify-corpus
Step 6.  Commit profiles/large-web-app.v1.json (already have a real measurement)
         Commit profiles/large-web-app-small.v1.json (10% sample)
Step 7.  Wire CAS/ABS/analysis benchmarks to accept a corpus path
```

**At Step 7, the existing benchmark runners (`analysis_bench`, `cas_bench`, `abs_bench`) can immediately run against a 36k-file corpus.** This is the proof-of-concept the user asked for.

### Phase 2 (refines graph shape):

```
Step 8.  graph_profile.rs           ── extract GraphShapeProfile from analyzer output
Step 9.  graph_gen.rs               ── configuration-model sampling + SCC/dead injection
Step 10. graph_similarity.rs        ── scalar tolerance + EMD on histograms
Step 11. CLI: graph-profile, graph-similarity
Step 12. Re-emit presets/uniform.v1.json from the OLD synthetic.rs output
         (so backward-compat benchmarks bit-identical)
Step 13. Capture large-web-app's actual graph shape, commit as
         profiles/large-web-app.v1.json's gen.graph_shape field
```

### Phase 3 (schema v2 — addresses R1):

```
Step 14. repo_scale.rs schema v2: per-extension byte/line percentiles
Step 15. corpus_gen uses log-normal/bimodal sampler when v2 percentiles present
Step 16. Re-measure the reference monorepo, commit large-web-app.v2.json
```

### What can be built in parallel

- **Steps 1, 2 in parallel.** Different files, no shared logic.
- **Step 4 (verifier) in parallel with Steps 2-3 (generator).** Both depend only on `BenchProfile` shape (Step 1). The verifier can be written against `RepoScaleReport`'s existing measurement code without waiting for the generator.
- **Phase 2 (Steps 8–13) fully independent of Phase 3.**
- **The MVP (Steps 1–7) unblocks all CAS/ABS/analysis benchmark work at real scale, even with the uniform-preset graph.**

### Non-goals for v1

- No real Node.js / npm install — generated `package.json` files are never actually installed; SWC and `build_adjacency` work from source alone
- No actual TypeScript type-checking — `tsc` is never invoked; only SWC parse path is exercised
- No graph similarity sanity check between the two real measurements at the measured commit and one commit later — that's a separate "graph drift over time" study

---

## OPEN QUESTIONS (not blocking this design — answer at planning time)

1. **Profile location: `crates/wundler-bench/profiles/` (in-crate) or `bench/profiles/` (repo-root)?**
   - In-crate: ships with the crate, simple `include_bytes!` if we ever embed.
   - Repo-root: separate from code, no rebuild on profile change.
   - *Suggest: in-crate.* Profiles are small (<100 KB), and crate ownership = clearer change review.

2. **Should `verify-corpus` warn or fail on schema-version mismatch between `RepoScaleReport` produced by re-measurement and the target profile?**
   - *Suggest: warn but proceed if compat range allows; fail if outside.*

3. **`generate-corpus` output directory: temp by default, persistent via `--out`?**
   - *Suggest: require explicit `--out`, no temp default. Generating 36k files into `/tmp` and walking away is a footgun.*

4. **Do we measure the reference monorepo at a *pinned commit* and pin that SHA in the profile, or measure at "current HEAD whenever someone updates"?**
   - *Suggest: pin a commit hash in `git.head_commit` (already in `RepoScaleReport`). When the profile is re-measured, the commit changes — that's a meaningful diff in PR review.*

---

## SUMMARY

| | |
|---|---|
| **Part A recommendation** | **Candidate 2 — Profile-as-Contract.** `RepoScaleReport` is the `BenchProfile`. Generator + verifier share the type. V1–V5 mechanically checkable. |
| **Part B recommendation** | **B2 — Configuration-model graph sampling.** Gated behind `GenHints.graph_shape`. Falls back to `presets/uniform.v1.json` preserving current `synthetic.rs` behavior. |
| **MVP** | Steps 1–7. Unblocks CAS/ABS/analysis benchmarks at 36k-file scale without graph shape work. |
| **First crusty test passes** | `wundler-bench generate-corpus --profile profiles/large-web-app.v1.json --out target/corpus && wundler-bench verify-corpus target/corpus --profile profiles/large-web-app.v1.json` exits 0. |
| **Top risk to watch** | R1 — file-size distribution within an extension. Mitigated by schema v2 percentiles in Phase 3. |
| **What this is NOT** | Not a real-workload benchmark. Function bodies are inert; this measures analyzer/chunker/CAS cost on a *structurally faithful* corpus, not on real computation. |
| **Why this is right** | The user explicitly accepted "contrived and useless as a benchmark but Certifiably Verifiable at this scale." V1–V5 mechanical checkability is the contract. C2 delivers it with the smallest schema surface. |
