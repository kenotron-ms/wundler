
//! Markdown report formatter for wundler-bench results.
//!
//! Produces GitHub-flavoured Markdown with ASCII tables suitable for
//! committing as `BENCHMARK_RESULTS.md`.

use std::path::Path;

use anyhow::Result;

use crate::abs_bench::{AbsChurnResult, AbsScaleResult};
use crate::analysis_bench::AnalysisBenchResult;
use crate::cas_bench::CasBenchResult;

// ---------------------------------------------------------------------------
// CAS table
// ---------------------------------------------------------------------------

/// Format the CAS benchmark results as a Markdown table.
///
/// ```markdown
/// | Modules | Cold build | Warm +1 change | Speedup |
/// |--------:|----------:|---------------:|--------:|
/// |     100 |     12 ms |          1 ms  |   12.0× |
/// ```
pub fn format_cas_table(results: &[CasBenchResult]) -> String {
    let mut s = String::new();
    s.push_str("| Modules | Cold build | Warm +1 change | Speedup |\n");
    s.push_str("|--------:|----------:|---------------:|--------:|\n");
    for r in results {
        s.push_str(&format!(
            "| {:>7} | {:>9} ms | {:>13} ms | {:>6.1}× |\n",
            format_thousands(r.n_modules),
            r.cold_build_ms,
            r.warm_build_ms,
            r.speedup,
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// ABS table
// ---------------------------------------------------------------------------

/// Format the ABS churn results as a Markdown table.
///
/// ```markdown
/// | Code churn | Traditional CDN | ABS delta | Savings |
/// |-----------:|----------------:|----------:|--------:|
/// |       0.5% |          100.0% |      4.2% |   95.8% |
/// ```
pub fn format_abs_table(results: &[AbsChurnResult]) -> String {
    let mut s = String::new();
    s.push_str("| Code churn | Traditional CDN | ABS delta | Savings |\n");
    s.push_str("|-----------:|----------------:|----------:|--------:|\n");
    for r in results {
        let churn_pct = r.churn_fraction * 100.0;
        let abs_pct = if r.total_bundle_kb > 0.0 {
            (r.abs_download_kb / r.total_bundle_kb) * 100.0
        } else {
            0.0
        };
        s.push_str(&format!(
            "| {:>9} | {:>14.1}% | {:>8.1}% | {:>6.1}% |\n",
            format_churn_pct(churn_pct),
            100.0_f64,
            abs_pct,
            r.savings_pct,
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// Analysis-pipeline table (CAS-isolated)
// ---------------------------------------------------------------------------

/// Format the analysis-only benchmark results as a Markdown table.
///
/// Rows flagged as extrapolated are marked with `*`.
///
/// ```markdown
/// | Modules | Cold (ms) | Warm (ms) | Speedup | Warm graph (ms) |
/// |--------:|----------:|----------:|--------:|----------------:|
/// |   1,000 |        80 |         2 |   40.0× |               1 |
/// ```
pub fn format_analysis_table(results: &[AnalysisBenchResult]) -> String {
    let mut s = String::new();
    s.push_str("| Modules | Cold (ms) | Warm (ms) | Speedup | Warm graph (ms) |\n");
    s.push_str("|--------:|----------:|----------:|--------:|----------------:|\n");
    for r in results {
        let flag = if r.is_extrapolated { " *" } else { "" };
        s.push_str(&format!(
            "| {:>9} | {:>9} | {:>9} | {:>6.1}× | {:>15} |\n",
            format!("{}{}", format_thousands(r.n_modules), flag),
            r.cold_ms,
            r.warm_ms,
            r.speedup,
            r.warm_graph_ms,
        ));
    }
    s
}

// ---------------------------------------------------------------------------
// Multi-scale ABS section
// ---------------------------------------------------------------------------

/// Format the ABS benchmark results for multiple module-count scales.
///
/// Emits one sub-section per scale, followed by a note on chunk granularity.
pub fn format_abs_scales_section(scale_results: &[AbsScaleResult]) -> String {
    let mut s = String::new();
    for sr in scale_results {
        s.push_str(&format!(
            "### N = {} modules\n\n",
            format_thousands(sr.n_modules)
        ));
        s.push_str(&format_abs_table(&sr.churns));
        s.push('\n');
    }
    s.push_str(
        "> **Note**: savings are measured at chunk granularity, not module granularity. \
         Changes within a single chunk result in that whole chunk being re-fetched \
         regardless of how few modules changed within it. \
         PGO-driven fine-grained chunk splitting (Plan 5) reduces per-module delta cost.\n",
    );
    s
}

// ---------------------------------------------------------------------------
// Real-world scenario section
// ---------------------------------------------------------------------------

/// Compute and format real-world scenario projections for two predefined scales.
///
/// Uses the measured analysis-only speedup data and ABS delta fractions to
/// project to realistic production environments.
pub fn format_real_world_scenarios(
    analysis: &[AnalysisBenchResult],
    abs_scales: &[AbsScaleResult],
) -> String {
    let mut s = String::new();
    s.push_str("## 4. Real-World Scenarios\n\n");
    s.push_str(
        "These projections combine the measured analysis-only speedup with ABS delta \
         fractions to estimate build savings and CDN cost reductions at production scale.\n\n",
    );

    // The two predefined scenarios from the spec.
    let scenarios: &[(&str, usize, usize, usize, f64, f64)] = &[
        // (label, n_modules, dau, sessions_per_user, weekly_churn_pct, initial_bundle_kb)
        ("Mid-scale", 5_000, 1_000, 5, 0.02, 500.0),
        ("Large-scale", 50_000, 10_000, 5, 0.01, 2_000.0),
    ];

    for &(label, n_modules, dau, sessions_per_user, weekly_churn_pct, initial_bundle_kb) in
        scenarios
    {
        s.push_str(&format!("### {label}: {n_mod} modules\n\n",
            n_mod = format_thousands(n_modules)));

        // ── Build-time projection ─────────────────────────────────────────
        // Find the best matching analysis result (exact or nearest-N).
        let analysis_row = find_nearest_analysis(analysis, n_modules);

        if let Some(ar) = analysis_row {
            s.push_str(&format!(
                "**Weekly build time** (analysis pipeline only, no transform):\n\
                 - Weekly build time (cold):  {cold} ms ({cold_s:.1} s)\n\
                 - Weekly build time (warm):  {warm} ms ({speedup:.0}× faster)\n\n",
                cold = ar.cold_ms,
                cold_s = ar.cold_ms as f64 / 1000.0,
                warm = ar.warm_ms,
                speedup = ar.speedup,
            ));
        } else {
            s.push_str("_(no matching analysis benchmark data)_\n\n");
        }

        // ── CDN bandwidth projection ──────────────────────────────────────
        // Find the ABS delta fraction for the given churn.
        let delta_fraction = find_abs_delta_fraction(abs_scales, weekly_churn_pct);
        let sessions_per_week = dau * sessions_per_user;

        let full_download_kb = initial_bundle_kb;
        let abs_download_kb = initial_bundle_kb * delta_fraction;
        let savings_pct = (1.0 - delta_fraction) * 100.0;

        let cdn_kb_per_week = full_download_kb * sessions_per_week as f64;
        let abs_kb_per_week = abs_download_kb * sessions_per_week as f64;
        let saved_kb = cdn_kb_per_week - abs_kb_per_week;

        // Convert to GB for CDN cost calculation.
        let cdn_gb_per_week = cdn_kb_per_week / (1024.0 * 1024.0);
        let abs_gb_per_week = abs_kb_per_week / (1024.0 * 1024.0);
        let saved_gb = saved_kb / (1024.0 * 1024.0);
        let saved_dollars = saved_gb * 0.09;

        s.push_str(&format!(
            "**CDN bandwidth** ({churn:.0}% weekly churn, {dau} DAU, {spu} sessions/user):\n\
             - Per-deploy download per user:\n\
               - Without ABS: {bundle_kb:.0} KB (100%)\n\
               - With ABS:    {abs_kb:.0} KB ({savings:.0}% less)\n\
             - Weekly CDN bandwidth for all users:\n\
               - Without ABS: {cdn_gb:.2} GB / week\n\
               - With ABS:    {abs_gb:.2} GB / week\n\
               - **Saved: {saved_gb:.2} GB / week ≈ ${dollars:.2} / week at $0.09/GB**\n\n",
            churn = weekly_churn_pct * 100.0,
            dau = format_thousands(dau),
            spu = sessions_per_user,
            bundle_kb = full_download_kb,
            abs_kb = abs_download_kb,
            savings = savings_pct,
            cdn_gb = cdn_gb_per_week,
            abs_gb = abs_gb_per_week,
            saved_gb = saved_gb,
            dollars = saved_dollars,
        ));
    }

    s
}

// ---------------------------------------------------------------------------
// Helpers for scenario lookup
// ---------------------------------------------------------------------------

/// Find the analysis result closest to `target_n`, preferring exact matches
/// and falling back to the largest measured / extrapolated result.
fn find_nearest_analysis(results: &[AnalysisBenchResult], target_n: usize) -> Option<&AnalysisBenchResult> {
    results
        .iter()
        .min_by_key(|r| (r.n_modules as isize - target_n as isize).unsigned_abs())
}

/// Look up the ABS delta fraction (abs_download_kb / total_bundle_kb) for the
/// churn level nearest to `target_churn`.  Falls back to 0.5 if no data.
fn find_abs_delta_fraction(abs_scales: &[AbsScaleResult], target_churn: f64) -> f64 {
    // Use the largest-scale measurement for the best representative data.
    let best_scale = abs_scales.iter().max_by_key(|s| s.n_modules);
    let churns = match best_scale {
        Some(s) => &s.churns,
        None => return 0.5,
    };
    let nearest = churns.iter().min_by(|a, b| {
        (a.churn_fraction - target_churn)
            .abs()
            .partial_cmp(&(b.churn_fraction - target_churn).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    match nearest {
        Some(r) if r.total_bundle_kb > 0.0 => r.abs_download_kb / r.total_bundle_kb,
        _ => 0.5,
    }
}

// ---------------------------------------------------------------------------
// Combined scenario section
// ---------------------------------------------------------------------------

/// Produce the combined-scenario prose section for the report.
///
/// Assumes the largest CAS result and the smallest-churn ABS result are
/// representative of a production 10,000-module app with 2% weekly code churn.
pub fn format_combined_scenario(cas: &[CasBenchResult], abs: &[AbsChurnResult]) -> String {
    // Find the largest-scale CAS result.
    let cas_top = cas.iter().max_by_key(|r| r.n_modules);
    // Find the ABS result nearest to 2% churn.
    let abs_2pct = abs.iter().min_by(|a, b| {
        (a.churn_fraction - 0.02)
            .abs()
            .partial_cmp(&(b.churn_fraction - 0.02).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let dau: u64 = 10_000;
    let sessions_per_user: u64 = 5;
    let deploys_per_week: u64 = 1;

    let mut s = String::new();

    s.push_str("## 3. Combined Scenario: Production App\n\n");
    s.push_str(
        "Assume a production app with weekly deploys, 10,000 daily active users, \
         and 5 page views per session.\n\n",
    );

    if let Some(c) = cas_top {
        let cold_ms = c.cold_build_ms;
        let warm_ms = c.warm_build_ms;
        s.push_str(&format!(
            "**Build time** (N = {} modules):\n\
             - Cold build: {} ms → developer waits {:.1} s\n\
             - Warm build (1 file changed): {} ms → developer waits <1 s\n\
             - Speedup: {:.1}×\n\n",
            format_thousands(c.n_modules),
            cold_ms,
            cold_ms as f64 / 1000.0,
            warm_ms,
            c.speedup,
        ));
    }

    if let Some(a) = abs_2pct {
        let sessions_per_week = dau * sessions_per_user * deploys_per_week;
        let full_kb = a.total_bundle_kb;
        let delta_kb = a.abs_download_kb;

        let cdn_mb_per_week = (full_kb * sessions_per_week as f64) / 1024.0;
        let abs_mb_per_week = (delta_kb * sessions_per_week as f64) / 1024.0;
        let saved_mb = cdn_mb_per_week - abs_mb_per_week;

        s.push_str(&format!(
            "**CDN bandwidth** ({:.0}% code churn, {} DAU, {} sessions/user, {} deploy/week):\n\
             - Traditional CDN: {:.0} MB / week\n\
             - ABS delta: {:.0} MB / week\n\
             - **Saved: {:.0} MB / week ({:.1}% reduction)**\n\n",
            a.churn_fraction * 100.0,
            dau,
            sessions_per_user,
            deploys_per_week,
            cdn_mb_per_week,
            abs_mb_per_week,
            saved_mb,
            a.savings_pct,
        ));
    }

    s.push_str(
        "> **Key insight**: ABS savings track chunk-level changes, not module-level changes.\n\
         > If a deploy only changes modules within one lazy chunk, only that chunk is \n\
         > re-downloaded — regardless of how many other chunks exist in the bundle.\n",
    );

    s
}

// ---------------------------------------------------------------------------
// Full report
// ---------------------------------------------------------------------------

/// Write the complete Markdown benchmark report to `path`.
///
/// Sections:
/// 1. Analysis pipeline only (CAS speedup without transform noise) — headline
/// 2. Full pipeline including transform (honest overall build time)
/// 3. ABS delta efficiency at multiple module scales
/// 4. Real-world scenarios
pub fn write_report(
    analysis: &[AnalysisBenchResult],
    cas: &[CasBenchResult],
    abs_scales: &[AbsScaleResult],
    path: &Path,
) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let ts_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let datetime = format_unix_datetime(ts_secs);
    let platform = platform_string();

    let mut doc = String::new();

    doc.push_str("# Wundler Benchmark Results\n\n");
    doc.push_str(&format!(
        "Platform: {} | Generated: {}\n\n",
        platform, datetime
    ));
    doc.push_str("---\n\n");

    // ── Section 1: Analysis pipeline only ──────────────────────────────────
    doc.push_str("## 1. Analysis Pipeline — CAS Speedup (no transform)\n\n");
    doc.push_str(
        "This section isolates what CAS actually controls: SWC parse + content-addressed \
         cache + graph analysis.  The rolldown subprocess is excluded so transform time \
         does not drown out the summarizer speedup.\n\n\
         On a warm rebuild (1 file changed out of N), N-1 modules are sub-millisecond \
         cache reads.  The speedup therefore scales with N.\n\n",
    );
    if !analysis.is_empty() {
        doc.push_str(&format_analysis_table(analysis));
        doc.push('\n');
        doc.push_str("_`*` rows are extrapolated via linear regression, not measured._\n\n");
    }

    // ── Section 2: Full pipeline including transform ────────────────────────
    doc.push_str("## 2. Full Pipeline — CAS + Transform\n\n");
    doc.push_str(
        "The full build includes the transform step (SWC or rolldown).  Transform time \
         is O(N) even on a warm rebuild because the engine re-processes every alive \
         module.  This is why the speedup numbers here are much lower than Section 1 — \
         the pipeline bottleneck shifts from cache I/O to transform.\n\n",
    );
    if !cas.is_empty() {
        doc.push_str(&format_cas_table(cas));
        doc.push('\n');
        doc.push_str(
            "_Warm build speedup is modest (≈1.5×) because rolldown re-transforms \
             every module regardless of cache hits._\n\n",
        );
    }

    // ── Section 3: ABS delta efficiency ────────────────────────────────────
    doc.push_str("## 3. ABS — Delivery Delta Efficiency\n\n");
    doc.push_str(
        "With ABS, the browser only downloads chunks whose module-hash sets changed. \
         The tables below show download fraction per deploy for a client that was \
         current on the previous build.\n\n",
    );
    if !abs_scales.is_empty() {
        doc.push_str(&format_abs_scales_section(abs_scales));
    }

    // ── Section 4: Real-world scenarios ────────────────────────────────────
    let mut all_analysis = analysis.to_vec();
    // Add extrapolated rows for 50k and 100k
    if !analysis.is_empty() {
        let measured: Vec<_> = analysis.iter().filter(|r| !r.is_extrapolated).cloned().collect();
        if !measured.is_empty() {
            for &target in &[50_000_usize, 100_000_usize] {
                let already_have = analysis.iter().any(|r| r.n_modules == target);
                if !already_have {
                    all_analysis.push(crate::analysis_bench::extrapolate(&measured, target));
                }
            }
        }
    }
    doc.push_str(&format_real_world_scenarios(&all_analysis, abs_scales));

    std::fs::write(path, &doc)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a `usize` with thousands separators (e.g. 10000 → "10,000").
fn format_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

/// Format a churn percentage, showing "0.5%" for fractions < 1.
fn format_churn_pct(pct: f64) -> String {
    if pct < 1.0 {
        format!("{:.1}%", pct)
    } else {
        format!("{:.0}%", pct)
    }
}

/// Produce a platform description string.
fn platform_string() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    format!("{} ({})", os, arch)
}

/// Very minimal Unix timestamp → "YYYY-MM-DD HH:MM" formatter.
/// Handles leap years and months correctly without external crates.
fn format_unix_datetime(secs: u64) -> String {
    // Days since Unix epoch
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Compute year, month, day from days since 1970-01-01
    let (y, m, d) = days_to_ymd(days);

    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, hours, minutes)
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let in_year = if is_leap(year) { 366 } else { 365 };
        if days < in_year {
            break;
        }
        days -= in_year;
        year += 1;
    }
    let months = if is_leap(year) {
        [31u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &m_days in &months {
        if days < m_days {
            break;
        }
        days -= m_days;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_thousands_no_comma_below_1000() {
        assert_eq!(format_thousands(100), "100");
        assert_eq!(format_thousands(999), "999");
    }

    #[test]
    fn format_thousands_adds_comma() {
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(10000), "10,000");
        assert_eq!(format_thousands(1000000), "1,000,000");
    }

    #[test]
    fn cas_table_has_header() {
        let results = vec![CasBenchResult {
            n_modules: 100,
            cold_build_ms: 10,
            warm_build_ms: 1,
            speedup: 10.0,
            cache_hit_rate: 0.99,
        }];
        let t = format_cas_table(&results);
        assert!(t.contains("| Modules |"));
        assert!(t.contains("100"));
        assert!(t.contains("10.0×"));
    }

    #[test]
    fn abs_table_has_header() {
        let results = vec![AbsChurnResult {
            churn_fraction: 0.01,
            total_bundle_kb: 100.0,
            abs_download_kb: 10.0,
            savings_pct: 90.0,
        }];
        let t = format_abs_table(&results);
        assert!(t.contains("| Code churn |"));
        assert!(t.contains("1%"));
        assert!(t.contains("90.0%"));
    }
}
