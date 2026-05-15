
//! `wundler-bench` — CAS and ABS benchmark suite CLI.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "wundler-bench",
    about = "Wundler benchmark suite: CAS and ABS performance measurement",
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
    ///   wundler-bench repo-scale --path ~/workspace/office-bohemia --output markdown
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

    // ── Early exit: persist a synthetic app and stop ───────────────────────
    if let Some(dest) = &args.persist_to {
        let n = args.persist_modules;
        eprintln!(
            "wundler-bench: generating {n}-module synthetic app → {}",
            dest.display()
        );
        wundler_bench::synthetic::SyntheticApp::generate_at(n, dest)?;
        eprintln!(
            "wundler-bench: done ✓  ({} files in {}/src/)",
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
        "wundler-bench: analysis benchmark at scales {:?}",
        analysis_scales
    );
    let analysis_results = wundler_bench::analysis_bench::run(&analysis_scales)?;

    // ── Full-pipeline CAS benchmark ────────────────────────────────────────
    let cas_results = if !args.analysis {
        eprintln!(
            "wundler-bench: full-pipeline CAS benchmark at scales {:?}",
            cas_scales
        );
        wundler_bench::cas_bench::run(&cas_scales)?
    } else {
        eprintln!("wundler-bench: --analysis flag set, skipping full-pipeline CAS bench");
        Vec::new()
    };

    // ── ABS benchmark ─────────────────────────────────────────────────────
    eprintln!(
        "wundler-bench: ABS benchmark at scales {:?} …",
        abs_scale_list
    );
    let abs_scale_results =
        wundler_bench::abs_bench::run_scales(&abs_scale_list, &churn_levels)?;

    // ── Write report ──────────────────────────────────────────────────────
    eprintln!("wundler-bench: writing report to {:?}", args.output);
    wundler_bench::report::write_report(
        &analysis_results,
        &cas_results,
        &abs_scale_results,
        &args.output,
    )?;

    eprintln!("wundler-bench: done ✓");
    print_summary(&analysis_results, &cas_results, &abs_scale_results);

    Ok(())
}

// ---------------------------------------------------------------------------
// Console summary
// ---------------------------------------------------------------------------

fn print_summary(
    analysis: &[wundler_bench::analysis_bench::AnalysisBenchResult],
    cas: &[wundler_bench::cas_bench::CasBenchResult],
    abs_scales: &[wundler_bench::abs_bench::AbsScaleResult],
) {
    println!();
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│  Wundler Benchmark Summary                                  │");
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
    use wundler_bench::repo_scale::{measure_repo, render_json, render_markdown, RepoScaleOptions};

    let opts = RepoScaleOptions {
        path,
        top_dirs,
        top_extensions,
        include_untracked,
    };

    eprintln!("wundler-bench repo-scale: measuring {} …", opts.path.display());
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
            eprintln!("wundler-bench repo-scale: wrote report to {}", file_path.display());
        }
        None => {
            println!("{rendered}");
        }
    }

    eprintln!("wundler-bench repo-scale: done ✓");
    Ok(())
}
