
//! CAS (Content-Addressed Summarizer) cache build-speed benchmark.
//!
//! Measures cold (from-scratch) and warm (one file changed) build times at
//! multiple module-count scales, demonstrating that incremental rebuild time
//! is roughly O(1) with respect to N while cold build time is O(N).

use std::time::Instant;

use anyhow::{Context, Result};

use crate::abs_bench::build_app;
use crate::synthetic::SyntheticApp;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result row for one scale point in the CAS benchmark.
#[derive(Debug, Clone)]
pub struct CasBenchResult {
    /// Number of source modules in the synthetic app.
    pub n_modules: usize,
    /// Median cold-build wall-clock time over 3 runs, in milliseconds.
    pub cold_build_ms: u64,
    /// Median warm-build wall-clock time over 3 runs, in milliseconds.
    pub warm_build_ms: u64,
    /// `cold_build_ms / warm_build_ms` speedup ratio.
    pub speedup: f64,
    /// Fraction of modules that are cache hits on a warm build: `(N-1) / N`.
    pub cache_hit_rate: f64,
}

// ---------------------------------------------------------------------------
// Benchmark runner
// ---------------------------------------------------------------------------

/// Run the CAS benchmark at each scale in `scales`.
///
/// For every N in `scales`:
/// 1. Generate a fresh `SyntheticApp` with N modules.
/// 2. Run 3 cold builds (each into a fresh `out_dir`); take the median time.
/// 3. Touch module 0 to invalidate exactly one summary cache entry.
/// 4. Run 3 warm builds; take the median time.
/// 5. Compute speedup and cache-hit rate.
pub fn run(scales: &[usize]) -> Result<Vec<CasBenchResult>> {
    let mut results = Vec::new();

    for &n in scales {
        eprintln!("  CAS bench N={} …", n);

        // ----- Cold builds (3 runs) ----------------------------------------
        let mut cold_times: Vec<u64> = Vec::new();
        for run_idx in 0..3 {
            let app = SyntheticApp::generate(n)
                .with_context(|| format!("generate failed for N={} run={}", n, run_idx))?;
            let out = tempfile::TempDir::new()?;

            let t0 = Instant::now();
            build_app(&app, out.path())
                .with_context(|| format!("cold build failed for N={} run={}", n, run_idx))?;
            cold_times.push(t0.elapsed().as_millis() as u64);
        }
        let cold_build_ms = median_u64(&mut cold_times);

        // ----- Warm builds (3 runs) -----------------------------------------
        // Warm builds reuse the same app (so cache was populated on its cold build),
        // then touch module 0 to create exactly one cache miss.
        let warm_app = SyntheticApp::generate(n)?;
        let warm_prime_out = tempfile::TempDir::new()?;
        // Prime the cache with one cold build.
        build_app(&warm_app, warm_prime_out.path())
            .context("warm-prime build failed")?;

        let mut warm_times: Vec<u64> = Vec::new();
        for run_idx in 0..3 {
            // Touch module 0 to invalidate its summary.
            warm_app
                .touch_module(0)
                .with_context(|| format!("touch_module failed for N={} run={}", n, run_idx))?;

            let out = tempfile::TempDir::new()?;
            let t0 = Instant::now();
            build_app(&warm_app, out.path())
                .with_context(|| format!("warm build failed for N={} run={}", n, run_idx))?;
            warm_times.push(t0.elapsed().as_millis() as u64);
        }
        let warm_build_ms = median_u64(&mut warm_times);

        let speedup = if warm_build_ms > 0 {
            cold_build_ms as f64 / warm_build_ms as f64
        } else {
            cold_build_ms as f64
        };
        let cache_hit_rate = if n > 1 {
            (n - 1) as f64 / n as f64
        } else {
            0.0
        };

        results.push(CasBenchResult {
            n_modules: n,
            cold_build_ms,
            warm_build_ms,
            speedup,
            cache_hit_rate,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Return the median of a mutable slice of `u64` values.
/// The slice is sorted in place. Returns 0 for an empty slice.
fn median_u64(vals: &mut [u64]) -> u64 {
    if vals.is_empty() {
        return 0;
    }
    vals.sort_unstable();
    let mid = vals.len() / 2;
    vals[mid]
}
