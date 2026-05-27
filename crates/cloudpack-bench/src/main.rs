
//! `cloudpack-bench` — CAS and ABS benchmark suite CLI.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "cloudpack-bench",
    about = "Cloudpack benchmark suite: CAS and ABS performance measurement",
    version
)]
struct Args {
    /// Subcommand (optional — if absent, runs the full benchmark suite).
    #[command(subcommand)]
    command: Option<Commands>,

    /// Output file path for the Markdown report.
    #[arg(long, default_value = "BENCHMARK_RESULTS.md", global = false)]
    output: PathBuf,

    /// Module counts for the full-pipeline CAS benchmark (comma-separated).
    #[arg(long, default_value = "100,500,1000,5000,10000")]
    scales: String,

    /// Module counts for the analysis-only benchmark (comma-separated).
    /// Defaults to the same list as --scales.
    #[arg(long)]
    analysis_scales: Option<String>,

    /// Module counts for the ABS benchmark (comma-separated).
    #[arg(long, default_value = "1000,5000,10000")]
    abs_scales: String,

    /// Run ONLY the analysis-only benchmark (skip full-pipeline CAS bench).
    #[arg(long)]
    analysis: bool,

    /// Skip slow benchmarks — caps all scale lists at N=500.
    #[arg(long)]
    fast: bool,

    /// Write a persistent synthetic TypeScript app to this directory and exit
    /// (skips all benchmark runs).  Use together with `--persist-modules` to
    /// control the module count (default: 10 000).
    #[arg(long, value_name = "PATH")]
    persist_to: Option<PathBuf>,

    /// Number of synthetic modules to generate when `--persist-to` is set.
    #[arg(long, value_name = "N", default_value = "10000")]
    persist_modules: usize,
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum Commands {
    /// Measure structural scale of a git repository using git metadata and
    /// filesystem byte/line counts.  Outputs a bounded summary — not file lists.
    ///
    /// Example:
    ///   cloudpack-bench repo-scale --path ~/workspace/office-bohemia --output markdown
    RepoScale {
        /// Path to the git repository to measure.
        #[arg(long)]
        path: PathBuf,

        /// Number of top-level directories to include in the report.
        #[arg(long, value_name = "N", default_value = "20")]
        top_dirs: usize,

        /// Number of extensions to include in the report.
        #[arg(long, value_name = "N", default_value = "30")]
        top_extensions: usize,

        /// Also walk the filesystem for files not tracked by git (ignores
        /// node_modules, .git, target, dist, build, etc.).
        #[arg(long)]
        include_untracked: bool,

        /// Output format.
        #[arg(long, value_name = "FORMAT", default_value = "markdown")]
        output: RepoScaleOutputFormat,

        /// Write the report to this file instead of stdout.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },

    /// Generate a synthetic corpus from a benchmark profile JSON.
    GenerateCorpus {
        /// Path to the benchmark profile JSON file.
        #[arg(long)]
        profile: std::path::PathBuf,

        /// Output directory for the generated corpus.
        #[arg(long)]
        out: std::path::PathBuf,
    },

    /// Verify a corpus directory against a benchmark profile JSON.
    VerifyCorpus {
        /// Path to the corpus directory to verify.
        #[arg(long)]
        corpus: std::path::PathBuf,

        /// Path to the benchmark profile JSON file.
        #[arg(long)]
        profile: std::path::PathBuf,

        /// Output verification report as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Run the analysis-only benchmark at a specific scale.
    AnalysisBench {
        /// Number of synthetic modules to analyse.
        #[arg(long, default_value = "1000")]
        modules: usize,

        /// Number of repeated cold-run measurements (for CV gate).
        #[arg(long, default_value_t = 1)]
        repeat: u32,

        /// Fail if coefficient of variation exceeds this threshold (0.0–1.0).
        #[arg(long)]
        check_cv: Option<f64>,
    },
}

/// Output format for the `repo-scale` subcommand.
#[derive(clap::ValueEnum, Clone, Debug)]
enum RepoScaleOutputFormat {
    Json,
    Markdown,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args = Args::parse();

    // ── repo-scale subcommand ─────────────────────────────────────────────────
    if let Some(Commands::RepoScale {
        path,
        top_dirs,
        top_extensions,
        include_untracked,
        output: fmt,
        out,
    }) = args.command
    {
        return run_repo_scale(path, top_dirs, top_extensions, include_untracked, fmt, out);
    }

    // ── generate-corpus subcommand ─────────────────────────────────────────────
    if let Some(Commands::GenerateCorpus { profile, out }) = args.command.as_ref() {
        let p = cloudpack_bench::profile::load_profile(profile)?;
        cloudpack_bench::corpus_gen::generate_corpus(&p, out)?;
        eprintln!(
            "generated corpus at {} (seed={}, target_files={})",
            out.display(),
            p.gen.seed,
            p.target.workspace_stats.total_files
        );
        return Ok(());
    }

    // ── verify-corpus subcommand ─────────────────────────────────────────────
    if let Some(Commands::VerifyCorpus {
        corpus,
        profile,
        json,
    }) = args.command.as_ref()
    {
        let p = cloudpack_bench::profile::load_profile(profile)?;
        let report = cloudpack_bench::corpus_verify::verify_corpus(&p, corpus)?;
        if *json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            let fails: Vec<_> = report
                .checks
                .iter()
                .filter(|c| c.status == cloudpack_bench::profile::CheckStatus::Fail)
                .collect();
            eprintln!(
                "verify: {} checks, passed={} (fails: {})",
                report.checks.len(),
                report.passed,
                fails.len()
            );
            for c in &fails {
                eprintln!(
                    "  FAIL {}: target={}, actual={}, tol={}%",
                    c.name,
                    c.target,
                    c.actual,
                    c.tolerance_pct * 100.0
                );
            }
        }
        if !report.passed {
            std::process::exit(1);
        }
        return Ok(());
    }

    // ── analysis-bench subcommand ────────────────────────────────────────────
    if let Some(Commands::AnalysisBench {
        modules,
        repeat,
        check_cv,
    }) = args.command
    {
        let scales = vec![modules];
        let mut cold_ms_values: Vec<f64> = Vec::new();
        for _ in 0..repeat {
            let run_results = cloudpack_bench::analysis_bench::run(&scales)?;
            if let Some(r) = run_results.first() {
                cold_ms_values.push(r.cold_ms as f64);
            }
        }
        // Final run for display
        let results = cloudpack_bench::analysis_bench::run(&scales)?;
        if let Some(r) = results.first() {
            eprintln!(
                "analysis-bench: N={} cold={}ms warm={}ms speedup={:.0}x",
                modules, r.cold_ms, r.warm_ms, r.speedup
            );
        }
        if let Some(cv_threshold) = check_cv {
            let cv = coefficient_of_variation(&cold_ms_values);
            eprintln!("analysis-bench: CV={:.4} (threshold={:.4})", cv, cv_threshold);
            if cv > cv_threshold {
                anyhow::bail!(
                    "CV {:.4} exceeds threshold {:.4} — measurement is too noisy",
                    cv,
                    cv_threshold
                );
            }
        }
        return Ok(());
    }

    // ── Early exit: persist a synthetic app and stop ─────────────────────────
    if let Some(dest) = &args.persist_to {
        let n = args.persist_modules;
        eprintln!(
            "cloudpack-bench: generating {n}-module synthetic app → {}",
            dest.display()
        );
        cloudpack_bench::synthetic::SyntheticApp::generate_at(n, dest)?;
        eprintln!(
            "cloudpack-bench: done ✓  ({} files in {}/src/)",
            n + 1,          // modules + main.tsx
            dest.display()
        );
        return Ok(());
    }

    // ── Parse scale lists ──────────────────────────────────────────────────
    let mut cas_scales: Vec<usize> = parse_scales(&args.scales);
    let mut analysis_scales: Vec<usize> = args
        .analysis_scales
        .as_deref()
        .map(parse_scales)
        .unwrap_or_else(|| cas_scales.clone());
    let mut abs_scale_list: Vec<usize> = parse_scales(&args.abs_scales);

    if args.fast {
        cap_scales(&mut cas_scales, 500);
        cap_scales(&mut analysis_scales, 500);
        cap_scales(&mut abs_scale_list, 500);
    }

    let churn_levels = vec![0.005, 0.01, 0.05, 0.10, 0.25, 0.50];

    // ── Analysis-only benchmark ────────────────────────────────────────────
    eprintln!(
        "cloudpack-bench: analysis benchmark at scales {:?}",
        analysis_scales
    );
    let analysis_results = cloudpack_bench::analysis_bench::run(&analysis_scales)?;

    // ── Full-pipeline CAS benchmark ────────────────────────────────────────
    let cas_results = if !args.analysis {
        eprintln!(
            "cloudpack-bench: full-pipeline CAS benchmark at scales {:?}",
            cas_scales
        );
        cloudpack_bench::cas_bench::run(&cas_scales)?
    } else {
        eprintln!("cloudpack-bench: --analysis flag set, skipping full-pipeline CAS bench");
        Vec::new()
    };

    // ── ABS benchmark ─────────────────────────────────────────────────────
    eprintln!(
        "cloudpack-bench: ABS benchmark at scales {:?} …",
        abs_scale_list
    );
    let abs_scale_results =
        cloudpack_bench::abs_bench::run_scales(&abs_scale_list, &churn_levels)?;

    // ── Write report ──────────────────────────────────────────────────────
    eprintln!("cloudpack-bench: writing report to {:?}", args.output);
    cloudpack_bench::report::write_report(
        &analysis_results,
        &cas_results,
        &abs_scale_results,
        &args.output,
    )?;

    eprintln!("cloudpack-bench: done ✓");
    print_summary(&analysis_results, &cas_results, &abs_scale_results);

    Ok(())
}

// ---------------------------------------------------------------------------
// Console summary
// ---------------------------------------------------------------------------

fn print_summary(
    analysis: &[cloudpack_bench::analysis_bench::AnalysisBenchResult],
    cas: &[cloudpack_bench::cas_bench::CasBenchResult],
    abs_scales: &[cloudpack_bench::abs_bench::AbsScaleResult],
) {
    println!();
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│  Cloudpack Benchmark Summary                                  │");
    println!("├─────────────────────────────────────────────────────────────┤");
    println!("│  Analysis-only CAS (no transform)                           │");
    for r in analysis {
        let flag = if r.is_extrapolated { "*" } else { " " };
        println!(
            "│  N={:>7}  cold={:>6}ms  warm={:>5}ms  speedup={:>6.0}×{}  │",
            format_n(r.n_modules),
            r.cold_ms,
            r.warm_ms,
            r.speedup,
            flag,
        );
    }
    if !cas.is_empty() {
        println!("├─────────────────────────────────────────────────────────────┤");
        println!("│  Full pipeline (incl. transform)                            │");
        for r in cas {
            println!(
                "│  N={:>7}  cold={:>6}ms  warm={:>5}ms  speedup={:>5.1}×  │",
                format_n(r.n_modules),
                r.cold_build_ms,
                r.warm_build_ms,
                r.speedup,
            );
        }
    }
    println!("├─────────────────────────────────────────────────────────────┤");
    println!("│  ABS — Delta Efficiency                                     │");
    for sr in abs_scales {
        println!(
            "│  N={:>7}:                                                │",
            format_n(sr.n_modules)
        );
        for r in &sr.churns {
            println!(
                "│    churn={:>5.1}%  delta={:>7.1}KB  saved={:>5.1}%          │",
                r.churn_fraction * 100.0,
                r.abs_download_kb,
                r.savings_pct,
            );
        }
    }
    println!("└─────────────────────────────────────────────────────────────┘");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_scales(s: &str) -> Vec<usize> {
    s.split(',')
        .filter_map(|tok| tok.trim().parse::<usize>().ok())
        .collect()
}

fn cap_scales(scales: &mut Vec<usize>, max: usize) {
    scales.retain(|&n| n <= max);
    if scales.is_empty() {
        scales.push(max);
    }
}

fn format_n(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

// ---------------------------------------------------------------------------
// CV helper
// ---------------------------------------------------------------------------

fn coefficient_of_variation(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    if mean == 0.0 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() / mean
}

#[cfg(test)]
mod cv_tests {
    use super::coefficient_of_variation;

    #[test]
    fn zero_for_single_sample() {
        assert_eq!(coefficient_of_variation(&[42.0]), 0.0);
    }

    #[test]
    fn zero_for_constant_samples() {
        assert_eq!(coefficient_of_variation(&[10.0, 10.0, 10.0]), 0.0);
    }

    #[test]
    fn positive_for_varying_samples() {
        let cv = coefficient_of_variation(&[10.0, 12.0, 8.0]);
        assert!(cv > 0.0 && cv < 1.0);
    }
}

// ---------------------------------------------------------------------------
// repo-scale subcommand handler
// ---------------------------------------------------------------------------

fn run_repo_scale(
    path: PathBuf,
    top_dirs: usize,
    top_extensions: usize,
    include_untracked: bool,
    fmt: RepoScaleOutputFormat,
    out: Option<PathBuf>,
) -> Result<()> {
    use cloudpack_bench::repo_scale::{measure_repo, render_json, render_markdown, RepoScaleOptions};

    let opts = RepoScaleOptions {
        path,
        top_dirs,
        top_extensions,
        include_untracked,
    };

    eprintln!("cloudpack-bench repo-scale: measuring {} …", opts.path.display());
    let report = measure_repo(&opts)
        .map_err(|e| anyhow::anyhow!("repo-scale measurement failed: {e}"))?;

    let rendered = match fmt {
        RepoScaleOutputFormat::Json => render_json(&report),
        RepoScaleOutputFormat::Markdown => render_markdown(&report),
    };

    match out {
        Some(ref file_path) => {
            std::fs::write(file_path, &rendered)
                .map_err(|e| anyhow::anyhow!("failed to write {}: {e}", file_path.display()))?;
            eprintln!("cloudpack-bench repo-scale: wrote report to {}", file_path.display());
        }
        None => {
            println!("{rendered}");
        }
    }

    eprintln!("cloudpack-bench repo-scale: done ✓");
    Ok(())
}
