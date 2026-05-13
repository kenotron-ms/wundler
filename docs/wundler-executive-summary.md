# Wundler: Next-Generation Build Infrastructure for Large-Scale Web Applications

## Executive Summary

Large-scale web applications have outgrown the architectural assumptions of every available web build tool. Build times and developer iteration latency now degrade with every module added, and no amount of faster hardware will reverse the trend — the bottleneck is architectural, not computational. **Wundler** is a proposal for a new class of build system that treats the application as a continuously-maintained module graph rather than something rebuilt from scratch on each run, delivering faster CI, faster developer iteration, and adaptive per-user delivery from a single underlying system. We are requesting approval for a four-week, single-engineer validation experiment that will produce a clear pass/fail decision before any further investment.

## The Problem

A large-scale web application at this tier can easily accumulate 50,000 or more modules, and every one of those modules is re-analyzed on every build. CI builds are measured in tens of minutes. Developers experience hot-reload delays significant enough that they break flow on every save. Across a development organization of this size, this compounds into thousands of engineer-hours lost per month — and it is getting worse linearly as the product grows. The cost is not theoretical; it shows up directly in feature throughput, time-to-mitigate for production incidents, and the increasing reluctance of teams to take on cross-cutting work that triggers large rebuild surfaces.

This is not a vendor problem. It is a category problem. The same architectural ceiling appears regardless of which tool the team uses.

## Why Existing Solutions Don't Scale

Every modern web build tool — Vite, webpack, esbuild, Rspack, Rolldown — shares the same foundational assumption: start from scratch, analyze the entire module graph, then produce an output. Each successive generation has made that traversal faster, but the work itself still scales linearly with the size of the application. Applications at this scale have crossed the threshold where linear is no longer fast enough.

Vite specifically deserves mention because it is widely cited as the modern answer. Vite's contribution was deferring the bundling work in development — but that is a workaround, not a solution. In production, Vite falls back to a full bundle, and at this scale, that fallback is just as slow as everything else. The development/production divergence introduces its own correctness risks. We need a system that does not have two different pipelines arguing over which one tells the truth.

## What We Validated

Before proposing new work, we examined an existing internal project (Cloudpack) that set out to solve a closely related problem at comparable scale. Cloudpack independently arrived at three of the conclusions central to Wundler's design: pre-compute lightweight metadata before any bundling work, decompose the application into independently-buildable units, and stitch results together at runtime rather than at build time. Cloudpack validates the architectural direction. Wundler extends it by solving the production-delivery problem that Cloudpack explicitly deferred and by operating at finer granularity, which preserves correctness guarantees that Cloudpack trades away for speed.

## The Approach

Wundler is structured as three phases that operate on a continuously-maintained module graph, plus an optional adaptive delivery layer.

```
   Source code
       |
       v
+----------------+   For each module, in parallel.
|  Phase 1:      |   Produces a small metadata snapshot per module.
|  Summarize     |   Cached by content. Unchanged modules are skipped.
+----------------+
       |
       v
+----------------+   Operates on metadata only — never source.
|  Phase 2:      |   Determines which modules are reachable, which are
|  Analyze       |   dead, and how to group them into delivery chunks.
+----------------+   Runs in milliseconds even at 50,000+ modules.
       |
       v
+----------------+   Transforms only modules that changed AND are reachable.
|  Phase 3:      |   Everything else served from cache.
|  Transform     |   Build cost becomes proportional to what changed.
+----------------+
       |
       v
+----------------+   Optional. Coordinates with each client to serve
|  Adaptive      |   only the modules the user does not already have.
|  Delivery      |   Learns from real usage patterns to improve groupings
+----------------+   over time — no rebuild required.
       |
       v
    Browser
```

The conceptual shift that makes this work: **the bundle is no longer a compiled artifact. It is a query result from a module graph that stays continuously up to date.** The same graph powers development (instant reloads), CI (fast incremental builds), and production (optimal per-user delivery). There is no separate rebuild step between environments — only different ways of querying the same graph.

The transform engine itself is pluggable. Wundler does not reinvent the production-grade bundler; it drives Rolldown or Rspack underneath, inheriting their correctness and their plugin ecosystems. The innovation is in everything above that layer.

## Investment & Timeline

| Phase | Milestone | Outcome | Duration |
|---|---|---|---|
| Validation | Phase 1 summarizer running across the full module graph | Pass/fail decision on the architecture | 1 month, 1 engineer |
| Phase 2 | Graph analysis layer producing the chunk plan | Replaces the slow part of every build | 1 month |
| Level 0 | Production pipeline rollout, CDN-backed | First measurable developer impact: faster CI and incrementals | 1 month |
| Level 1 | Adaptive delivery service | Per-user delta serving in production | Month 4+ |
| Level 2 | Usage-driven optimization | Automatically improved chunking from real telemetry | Month 6+ |

## Risk Management

The architecture is designed to fail safely at every level. Level 0 produces a static manifest hosted on the existing CDN; if the build pipeline encounters any problem, rollback to the current toolchain is immediate and uneventful. The adaptive delivery service in Level 1 is never a hard dependency — clients degrade cleanly to the static manifest if the service is unavailable. Each level is independently valuable and independently deployable, which means partial success is still success.

Correctness risk is contained by leaning on Rolldown or Rspack for the actual code transformation. Wundler controls the orchestration and graph analysis; it does not rewrite JavaScript semantics. A correctness audit mode is built in from the start, comparing Wundler's output against a known-good baseline before any developer-facing rollout.

The largest unknown — whether the metadata layer is actually compact enough at this scale — is the question the Month 1 validation experiment exists to answer.

## Validation-First Commitment

The single most important property of this proposal is that the riskiest assumption is validated first, cheaply. The Month 1 experiment runs Phase 1 across the complete module graph and answers one measurable question: does the metadata fit in roughly 100MB and does it produce within acceptable time bounds? If the answer is yes, the rest of the architecture follows. If the answer is no, we learn that before committing to Phase 2 — and the redesign happens before any further engineering investment. No other approach we are aware of offers a comparable single-measurement gate this early in the lifecycle.

## The Ask

We are requesting approval to staff one engineer for four weeks to run the Phase 1 validation experiment against the full production module graph. The deliverable is a single measurement with a clear pass/fail interpretation, a written report on the result, and a recommendation on whether to proceed to Phase 2. This is the smallest possible commitment that produces decision-quality information about an architectural direction that has the potential to meaningfully change developer velocity and customer-facing performance at this scale. We would like to begin within the next sprint.
