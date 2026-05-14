# Wundler Benchmark Results

Platform: macos (aarch64) | Generated: 2026-05-14 19:12

---

## 1. Analysis Pipeline — CAS Speedup (no transform)

This section isolates what CAS actually controls: SWC parse + content-addressed cache + graph analysis.  The rolldown subprocess is excluded so transform time does not drown out the summarizer speedup.

On a warm rebuild (1 file changed out of N), N-1 modules are sub-millisecond cache reads.  The speedup therefore scales with N.

| Modules | Cold (ms) | Warm (ms) | Speedup | Warm graph (ms) |
|--------:|----------:|----------:|--------:|----------------:|
|       100 |        19 |         3 |    6.3× |               0 |
|       500 |        73 |        27 |    2.7× |               3 |
|     1,000 |       141 |        58 |    2.4× |               7 |
|     5,000 |       694 |       297 |    2.3× |              38 |
|    10,000 |      1363 |       604 |    2.3× |              84 |

_`*` rows are extrapolated via linear regression, not measured._

## 2. Full Pipeline — CAS + Transform

The full build includes the transform step (SWC or rolldown).  Transform time is O(N) even on a warm rebuild because the engine re-processes every alive module.  This is why the speedup numbers here are much lower than Section 1 — the pipeline bottleneck shifts from cache I/O to transform.

| Modules | Cold build | Warm +1 change | Speedup |
|--------:|----------:|---------------:|--------:|
|     100 |        20 ms |            15 ms |    1.3× |
|     500 |       131 ms |            79 ms |    1.7× |
|   1,000 |       262 ms |           172 ms |    1.5× |
|   5,000 |      1306 ms |           897 ms |    1.5× |
|  10,000 |      2770 ms |          1845 ms |    1.5× |

_Warm build speedup is modest (≈1.5×) because rolldown re-transforms every module regardless of cache hits._

## 3. ABS — Delivery Delta Efficiency

With ABS, the browser only downloads chunks whose module-hash sets changed. The tables below show download fraction per deploy for a client that was current on the previous build.

### N = 1,000 modules

| Code churn | Traditional CDN | ABS delta | Savings |
|-----------:|----------------:|----------:|--------:|
|      0.5% |          100.0% |     49.9% |   50.1% |
|        1% |          100.0% |     49.9% |   50.1% |
|        5% |          100.0% |     49.9% |   50.1% |
|       10% |          100.0% |     49.9% |   50.1% |
|       25% |          100.0% |     49.9% |   50.1% |
|       50% |          100.0% |     99.9% |    0.1% |

### N = 5,000 modules

| Code churn | Traditional CDN | ABS delta | Savings |
|-----------:|----------------:|----------:|--------:|
|      0.5% |          100.0% |     50.0% |   50.0% |
|        1% |          100.0% |     50.0% |   50.0% |
|        5% |          100.0% |     50.0% |   50.0% |
|       10% |          100.0% |     50.0% |   50.0% |
|       25% |          100.0% |     50.0% |   50.0% |
|       50% |          100.0% |    100.0% |    0.0% |

### N = 10,000 modules

| Code churn | Traditional CDN | ABS delta | Savings |
|-----------:|----------------:|----------:|--------:|
|      0.5% |          100.0% |     50.0% |   50.0% |
|        1% |          100.0% |     50.0% |   50.0% |
|        5% |          100.0% |     50.0% |   50.0% |
|       10% |          100.0% |     50.0% |   50.0% |
|       25% |          100.0% |     50.0% |   50.0% |
|       50% |          100.0% |    100.0% |    0.0% |

> **Note**: savings are measured at chunk granularity, not module granularity. Changes within a single chunk result in that whole chunk being re-fetched regardless of how few modules changed within it. PGO-driven fine-grained chunk splitting (Plan 5) reduces per-module delta cost.
## 4. Real-World Scenarios

These projections combine the measured analysis-only speedup with ABS delta fractions to estimate build savings and CDN cost reductions at production scale.

### Mid-scale: 5,000 modules

**Weekly build time** (analysis pipeline only, no transform):
- Weekly build time (cold):  694 ms (0.7 s)
- Weekly build time (warm):  297 ms (2× faster)

**CDN bandwidth** (2% weekly churn, 1,000 DAU, 5 sessions/user):
- Per-deploy download per user:
- Without ABS: 500 KB (100%)
- With ABS:    250 KB (50% less)
- Weekly CDN bandwidth for all users:
- Without ABS: 2.38 GB / week
- With ABS:    1.19 GB / week
- **Saved: 1.19 GB / week ≈ $0.11 / week at $0.09/GB**

### Large-scale: 50,000 modules

**Weekly build time** (analysis pipeline only, no transform):
- Weekly build time (cold):  6807 ms (6.8 s)
- Weekly build time (warm):  604 ms (11× faster)

**CDN bandwidth** (1% weekly churn, 10,000 DAU, 5 sessions/user):
- Per-deploy download per user:
- Without ABS: 2000 KB (100%)
- With ABS:    1000 KB (50% less)
- Weekly CDN bandwidth for all users:
- Without ABS: 95.37 GB / week
- With ABS:    47.69 GB / week
- **Saved: 47.68 GB / week ≈ $4.29 / week at $0.09/GB**

