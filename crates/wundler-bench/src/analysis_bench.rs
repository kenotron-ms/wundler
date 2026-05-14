
//! Analysis pipeline benchmark — measures ONLY the summarize + graph-analysis
//! steps, deliberately excluding the transform (SWC/rolldown) step.
//!
//! This isolates what CAS actually controls: SWC parse + content-addressed
//! cache + graph analysis.  On a warm run (1 changed file out of N), N-1
//! modules are sub-millisecond cache reads, so the speedup is proportional to
//! N rather than the ~1.5× seen when transform time drowns it out.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};

use wundler_core::cache::local::LocalCache;
use wundler_core::summarizer::{summarize_directory, summarize_directory_with_stats};
use wundler_graph::analyzer::GraphAnalyzer;

use crate::synthetic::SyntheticApp;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result row for one scale point in the analysis-only benchmark.
#[derive(Debug, Clone)]
pub struct AnalysisBenchResult {
    /// Number of source modules in the synthetic app.
    pub n_modules: usize,
    /// Median cold-run wall-clock time (summarize-all + graph), milliseconds.
    pub cold_ms: u64,
    /// Median warm-run wall-clock time (1 re-parse + N-1 cache reads + graph), ms.
    pub warm_ms: u64,
    /// `cold_ms / warm_ms` — the CAS speedup ratio.
    pub speedup: f64,
    /// Graph-analysis portion of the warm run, milliseconds.
    pub warm_graph_ms: u64,
    /// `true` when this row was produced by [`extrapolate`], not measured.
    pub is_extrapolated: bool,
}

// ---------------------------------------------------------------------------
// Benchmark runner
// ---------------------------------------------------------------------------

/// Run the analysis-only benchmark at each scale in `scales`.
///
/// For every N in `scales`:
/// 1. Generate a fresh `SyntheticApp` with N modules and a fresh temp cache.
/// 2. Run 3 cold iterations (fresh cache each time); take the median.
/// 3. Warm-prime one additional cold build (hot cache for N files).
/// 4. `touch_module(0)` to invalidate exactly one cache entry.
/// 5. Run 3 warm iterations; take the median.
/// 6. Assert that ≥ N-2 cache hits occurred on the warm run.
pub fn run(scales: &[usize]) -> Result<Vec<AnalysisBenchResult>> {
    let mut results = Vec::new();

    for &n in scales {
        eprintln!("  analysis bench N={} …", n);

        // Build a shared entry-point map used for every GraphAnalyzer call.
        // Suffix-matching in GraphAnalyzer resolves `src/main.tsx` against
        // absolute WalkDir paths.
        let entry_points: HashMap<String, PathBuf> = {
            let mut m = HashMap::new();
            m.insert("/".to_string(), PathBuf::from("src/main.tsx"));
            m
        };

        // ── Cold builds (3 runs, fresh cache every time) ──────────────────
        let mut cold_times: Vec<u64> = Vec::new();
        for run_idx in 0..3 {
            let app = SyntheticApp::generate(n)
                .with_context(|| format!("generate failed N={n} run={run_idx}"))?;
            let cache_dir = tempfile::TempDir::new()?;
            let cache = LocalCache::new(cache_dir.path().to_path_buf())?;
            let src_dir = app.dir.path().join("src");

            let t0 = Instant::now();
            let nodes = summarize_directory(&src_dir, &cache)
                .with_context(|| format!("cold summarize failed N={n} run={run_idx}"))?;
            let analyzer = GraphAnalyzer::new(entry_points.clone());
            let _ = analyzer
                .analyze(nodes)
                .with_context(|| format!("cold analyze failed N={n} run={run_idx}"))?;
            cold_times.push(t0.elapsed().as_millis() as u64);
        }
        let cold_ms = median_u64(&mut cold_times);

        // ── Warm builds (primed cache, 1 changed file) ────────────────────
        let warm_app = SyntheticApp::generate(n)?;
        let warm_cache_dir = tempfile::TempDir::new()?;
        let warm_cache = LocalCache::new(warm_cache_dir.path().to_path_buf())?;
        let warm_src_dir = warm_app.dir.path().join("src");

        // Prime: populate the cache for all N files.
        {
            let nodes = summarize_directory(&warm_src_dir, &warm_cache)
                .context("warm-prime summarize failed")?;
            let analyzer = GraphAnalyzer::new(entry_points.clone());
            let _ = analyzer
                .analyze(nodes)
                .context("warm-prime analyze failed")?;
        }

        let mut warm_times: Vec<u64> = Vec::new();
        let mut graph_times: Vec<u64> = Vec::new();
        for run_idx in 0..3 {
            // Invalidate exactly one cache entry.
            warm_app
                .touch_module(0)
                .with_context(|| format!("touch_module failed N={n} run={run_idx}"))?;

            let t0 = Instant::now();

            // On the last (3rd) warm run, use the stats variant to assert
            // cache behaviour.  This adds negligible overhead and only runs
            // once per scale.
            let nodes = if run_idx == 2 {
                let result = summarize_directory_with_stats(&warm_src_dir, &warm_cache)
                    .with_context(|| {
                        format!("warm summarize_with_stats failed N={n} run={run_idx}")
                    })?;
                // Core correctness assertion: N-1 files must be cache hits.
                assert!(
                    result.stats.cache_hits >= n.saturating_sub(2),
                    "expected ≥{} cache hits on warm run, got {} (N={n})",
                    n.saturating_sub(2),
                    result.stats.cache_hits
                );
                result.nodes
            } else {
                summarize_directory(&warm_src_dir, &warm_cache)
                    .with_context(|| format!("warm summarize failed N={n} run={run_idx}"))?
            };

            // Time graph analysis separately so we can report warm_graph_ms.
            let tg = Instant::now();
            let analyzer = GraphAnalyzer::new(entry_points.clone());
            let _ = analyzer
                .analyze(nodes)
                .with_context(|| format!("warm analyze failed N={n} run={run_idx}"))?;
            let graph_ms = tg.elapsed().as_millis() as u64;

            warm_times.push(t0.elapsed().as_millis() as u64);
            graph_times.push(graph_ms);
        }
        let warm_ms = median_u64(&mut warm_times);
        let warm_graph_ms = median_u64(&mut graph_times);

        let speedup = if warm_ms > 0 {
            cold_ms as f64 / warm_ms as f64
        } else {
            cold_ms as f64
        };

        results.push(AnalysisBenchResult {
            n_modules: n,
            cold_ms,
            warm_ms,
            speedup,
            warm_graph_ms,
            is_extrapolated: false,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Linear extrapolation
// ---------------------------------------------------------------------------

/// Extrapolate the analysis benchmark to `target_n` using least-squares linear
/// regression on `cold_ms` vs `n_modules` from the measured `results`.
///
/// `warm_ms` is taken from the largest measured scale (it barely grows with N
/// because it is dominated by 1 SWC re-parse + fast cache I/O).
pub fn extrapolate(results: &[AnalysisBenchResult], target_n: usize) -> AnalysisBenchResult {
    // Linear regression: cold_ms = a + b * n_modules
    let xs: Vec<f64> = results.iter().map(|r| r.n_modules as f64).collect();
    let ys: Vec<f64> = results.iter().map(|r| r.cold_ms as f64).collect();

    let n = xs.len() as f64;
    let sum_x: f64 = xs.iter().sum();
    let sum_y: f64 = ys.iter().sum();
    let sum_xx: f64 = xs.iter().map(|x| x * x).sum();
    let sum_xy: f64 = xs.iter().zip(ys.iter()).map(|(x, y)| x * y).sum();

    let denom = n * sum_xx - sum_x * sum_x;
    let (a, b) = if denom.abs() < 1e-12 {
        // Degenerate — just use average slope.
        let avg_slope = if xs[0].abs() > 1e-12 { ys[0] / xs[0] } else { 0.0 };
        (0.0, avg_slope)
    } else {
        let b = (n * sum_xy - sum_x * sum_y) / denom;
        let a = (sum_y - b * sum_x) / n;
        (a, b)
    };

    let cold_ms = (a + b * target_n as f64).round().max(1.0) as u64;

    // warm_ms: use the largest measured warm_ms (effectively constant).
    let warm_ms = results
        .iter()
        .max_by_key(|r| r.n_modules)
        .map(|r| r.warm_ms)
        .unwrap_or(2);
    let warm_graph_ms = results
        .iter()
        .max_by_key(|r| r.n_modules)
        .map(|r| r.warm_graph_ms)
        .unwrap_or(1);

    let speedup = if warm_ms > 0 {
        cold_ms as f64 / warm_ms as f64
    } else {
        cold_ms as f64
    };

    AnalysisBenchResult {
        n_modules: target_n,
        cold_ms,
        warm_ms,
        speedup,
        warm_graph_ms,
        is_extrapolated: true,
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Return the median of a mutable slice of `u64` values.
fn median_u64(vals: &mut [u64]) -> u64 {
    if vals.is_empty() {
        return 0;
    }
    vals.sort_unstable();
    vals[vals.len() / 2]
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrapolate_perfect_linear() {
        // 10ms per 100 modules → at 10,000 modules expect ~1,000ms.
        let data = vec![
            AnalysisBenchResult {
                n_modules: 100,
                cold_ms: 10,
                warm_ms: 2,
                speedup: 5.0,
                warm_graph_ms: 1,
                is_extrapolated: false,
            },
            AnalysisBenchResult {
                n_modules: 1_000,
                cold_ms: 100,
                warm_ms: 2,
                speedup: 50.0,
                warm_graph_ms: 1,
                is_extrapolated: false,
            },
        ];
        let p = extrapolate(&data, 10_000);
        assert_eq!(p.n_modules, 10_000);
        assert!(p.is_extrapolated);
        // ~1000ms ± 50%
        assert!(
            p.cold_ms > 500 && p.cold_ms < 2_000,
            "expected ~1000ms, got {}",
            p.cold_ms
        );
        // warm stays near the last measured warm
        assert_eq!(p.warm_ms, 2);
    }

    #[test]
    fn extrapolate_single_point_uses_slope() {
        let data = vec![AnalysisBenchResult {
            n_modules: 1_000,
            cold_ms: 100,
            warm_ms: 2,
            speedup: 50.0,
            warm_graph_ms: 1,
            is_extrapolated: false,
        }];
        let p = extrapolate(&data, 10_000);
        assert_eq!(p.n_modules, 10_000);
        assert!(p.is_extrapolated);
        assert!(p.cold_ms > 0, "extrapolated cold_ms must be > 0");
    }

    #[test]
    fn speedup_computed_correctly_in_extrapolate() {
        let data = vec![
            AnalysisBenchResult {
                n_modules: 100,
                cold_ms: 100,
                warm_ms: 1,
                speedup: 100.0,
                warm_graph_ms: 1,
                is_extrapolated: false,
            },
            AnalysisBenchResult {
                n_modules: 1_000,
                cold_ms: 1_000,
                warm_ms: 1,
                speedup: 1_000.0,
                warm_graph_ms: 1,
                is_extrapolated: false,
            },
        ];
        let p = extrapolate(&data, 10_000);
        // speedup ≈ 10_000 (cold ~10_000ms, warm ~1ms)
        assert!(
            (p.speedup - 10_000.0).abs() < 3_000.0,
            "speedup should be ~10000, got {}",
            p.speedup
        );
    }
}
