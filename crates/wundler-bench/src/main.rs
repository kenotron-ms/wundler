
//! `wundler-bench` — CAS and ABS benchmark suite CLI.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

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
    /// Output file path for the Markdown report.
    #[arg(long, default_value = "BENCHMARK_RESULTS.md")]
    output: PathBuf,

    /// Module counts for the CAS benchmark (comma-separated).
    #[arg(long, default_value = "100,500,1000,5000,10000")]
    scales: String,

    /// Module count for the ABS benchmark.
    #[arg(long, default_value_t = 1000)]
    abs_modules: usize,

    /// Skip slow benchmarks — runs only N=100 and N=500 for CAS.
    #[arg(long)]
    fast: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let args = Args::parse();

    // Parse scale list
    let mut scales: Vec<usize> = args
        .scales
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect();

    if args.fast {
        scales.retain(|&n| n <= 500);
        if scales.is_empty() {
            scales = vec![100, 500];
        }
    }

    eprintln!("wundler-bench: CAS benchmark at scales {:?}", scales);
    let cas_results = wundler_bench::cas_bench::run(&scales)?;

    eprintln!(
        "wundler-bench: ABS benchmark at N={} …",
        args.abs_modules
    );
    let churn_levels = vec![0.005, 0.01, 0.05, 0.10, 0.25, 0.50];
    let abs_results = wundler_bench::abs_bench::run(args.abs_modules, &churn_levels)?;

    eprintln!("wundler-bench: writing report to {:?}", args.output);
    wundler_bench::report::write_report(&cas_results, &abs_results, &args.output)?;

    eprintln!("wundler-bench: done ✓");
    print_summary(&cas_results, &abs_results);

    Ok(())
}

// ---------------------------------------------------------------------------
// Console summary
// ---------------------------------------------------------------------------

fn print_summary(
    cas: &[wundler_bench::cas_bench::CasBenchResult],
    abs: &[wundler_bench::abs_bench::AbsChurnResult],
) {
    println!();
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│  Wundler Benchmark Summary                                  │");
    println!("├─────────────────────────────────────────────────────────────┤");
    println!("│  CAS — Summarizer Cache                                     │");
    for r in cas {
        println!(
            "│  N={:>7}  cold={:>6}ms  warm={:>5}ms  speedup={:>5.1}×  │",
            format_n(r.n_modules),
            r.cold_build_ms,
            r.warm_build_ms,
            r.speedup,
        );
    }
    println!("├─────────────────────────────────────────────────────────────┤");
    println!("│  ABS — Delta Efficiency                                     │");
    for r in abs {
        println!(
            "│  churn={:>5.1}%  total={:>7.1}KB  delta={:>7.1}KB  saved={:>5.1}%  │",
            r.churn_fraction * 100.0,
            r.total_bundle_kb,
            r.abs_download_kb,
            r.savings_pct,
        );
    }
    println!("└─────────────────────────────────────────────────────────────┘");
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
