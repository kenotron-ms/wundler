
//! Markdown report formatter for wundler-bench results.
//!
//! Produces GitHub-flavoured Markdown with ASCII tables suitable for
//! committing as `BENCHMARK_RESULTS.md`.

use std::path::Path;

use anyhow::Result;

use crate::abs_bench::AbsChurnResult;
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
pub fn write_report(cas: &[CasBenchResult], abs: &[AbsChurnResult], path: &Path) -> Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let ts_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // YYYY-MM-DD HH:MM from Unix timestamp (simple, no external crate)
    let datetime = format_unix_datetime(ts_secs);
    let platform = platform_string();

    let mut doc = String::new();

    doc.push_str("# Wundler Benchmark Results\n\n");
    doc.push_str(&format!(
        "Platform: {} | Generated: {}\n\n",
        platform, datetime
    ));
    doc.push_str("---\n\n");

    // ---- Section 1: CAS ----
    doc.push_str("## 1. CAS — Summarizer Cache: Build Speed at Scale\n\n");
    doc.push_str(
        "The wundler summarizer caches each module's analysis as a content-addressed \
         `ModuleSummary`. On incremental rebuilds, only changed files are re-summarized; \
         every other module is a sub-millisecond cache read.\n\n",
    );
    doc.push_str(&format_cas_table(cas));
    doc.push('\n');
    doc.push_str(
        "_Cold build scales linearly with module count. \
         Warm (incremental) build is near-constant — dominated by the graph analysis \
         pass, not summarization._\n\n",
    );

    // ---- Section 2: ABS ----
    doc.push_str("## 2. ABS — Delivery Delta Efficiency\n\n");
    doc.push_str(
        "With ABS, the browser only downloads chunks whose module-hash sets changed. \
         The table below shows download fraction per deploy for a client that was \
         current on the previous build.\n\n",
    );
    if !abs.is_empty() {
        let n = abs[0].total_bundle_kb * 1024.0 / 1.0; // rough module count
        let _ = n;
        doc.push_str(&format_abs_table(abs));
        doc.push('\n');
        doc.push_str(
            "_Savings plateau when churn stays within a single chunk. They collapse \
             once churn spans the chunk boundary — illustrating that ABS operates at \
             chunk granularity, not module granularity._\n\n",
        );
    }

    // ---- Section 3: Combined scenario ----
    doc.push_str(&format_combined_scenario(cas, abs));

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
        if i > 0 && (len - i) % 3 == 0 {
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
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
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
