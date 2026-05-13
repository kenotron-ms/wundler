use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;
use wundler_core::{cache::local::LocalCache, validation::run_validate_scale, ModuleSummarizer};
use wundler_graph::GraphAnalyzer;

const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

// ---------------------------------------------------------------------------
// CLI structure
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "wundler", about = "Wundler module bundler tools", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Summarize a single JS/TS module and print its JSON summary.
    Summarize {
        /// Path to the JS/TS source file to summarize.
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Optional directory to use as the local summary cache root.
        /// Defaults to ~/.wundler/cache/summaries/.
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,
    },

    /// Validate a directory of JS/TS modules at scale and report Phase 1 gate metrics.
    ValidateScale {
        /// Root directory containing JS/TS source files to analyse.
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Optional directory to use as the local summary cache root.
        /// Defaults to ~/.wundler/cache/summaries/.
        #[arg(long, value_name = "DIR")]
        cache_dir: Option<PathBuf>,
    },

    /// Analyse a directory and produce a chunk manifest JSON.
    Analyze {
        /// Root directory containing JS/TS source files.
        #[arg(value_name = "PATH")]
        path: PathBuf,

        /// Entry point(s) in ROUTE=PATH format (e.g. /=src/index.ts).
        /// PATH is relative to the directory argument.
        /// Repeat this flag for multiple entry points.
        #[arg(long = "entry", value_name = "ROUTE=PATH", required = true)]
        entry: Vec<String>,

        /// Minimum number of chunks a module must appear in to be promoted to
        /// the shared commons chunk. Defaults to 2.
        #[arg(long = "commons-threshold", default_value_t = 2)]
        commons_threshold: usize,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open (or create) a [`LocalCache`] from an optional path argument.
fn open_cache(cache_dir: Option<PathBuf>) -> Result<LocalCache> {
    match cache_dir {
        Some(dir) => LocalCache::new(dir).context("failed to open cache at specified directory"),
        None => LocalCache::with_default_root().context("failed to open default cache"),
    }
}

/// Count JS/TS files under `dir` (non-recursively filtered by extension).
fn count_js_ts_files(dir: &std::path::Path) -> usize {
    WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| JS_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .count()
}

/// Parse a single `--entry` argument of the form `ROUTE=PATH`.
///
/// Returns `(route, path)` where `path` is the raw (possibly relative) path
/// string from the argument.
///
/// # Errors
///
/// Returns an error (mentioning "entry") if the argument has no `=`, or if
/// either the route or path component is empty.
fn parse_entry_arg(s: &str) -> Result<(String, PathBuf)> {
    let eq_pos = s.find('=').ok_or_else(|| {
        anyhow::anyhow!(
            "invalid --entry value {s:?}: expected ROUTE=PATH format (no '=' found); \
             use --entry <route>=<path>"
        )
    })?;

    let route = &s[..eq_pos];
    let path = &s[eq_pos + 1..];

    if route.is_empty() {
        anyhow::bail!(
            "invalid --entry value {s:?}: route (left of '=') cannot be empty; \
             use --entry <route>=<path>"
        );
    }
    if path.is_empty() {
        anyhow::bail!(
            "invalid --entry value {s:?}: path (right of '=') cannot be empty; \
             use --entry <route>=<path>"
        );
    }

    Ok((route.to_string(), PathBuf::from(path)))
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn cmd_summarize(path: PathBuf, cache_dir: Option<PathBuf>) -> Result<()> {
    let cache = open_cache(cache_dir)?;
    let summarizer = ModuleSummarizer::new();
    let node = summarizer
        .summarize(&path)
        .with_context(|| format!("failed to summarize {}", path.display()))?;
    // Persist to cache.
    cache
        .put(&node.id, &node.summary)
        .context("failed to write summary to cache")?;
    // Print pretty JSON.
    let json = serde_json::to_string_pretty(&node).context("failed to serialize summary")?;
    println!("{json}");
    Ok(())
}

fn cmd_validate_scale(path: PathBuf, cache_dir: Option<PathBuf>) -> Result<()> {
    let cache = open_cache(cache_dir)?;

    // Count files first so we can size the progress bar.
    let file_count = count_js_ts_files(&path) as u64;

    // Build and show the progress bar.
    let pb = ProgressBar::new(file_count);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} modules  {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    // Run the scale validation (processing happens internally).
    let stats = run_validate_scale(&path, &cache)
        .with_context(|| format!("validation failed for {}", path.display()))?;

    // Advance the progress bar to completion and finish.
    pb.set_position(file_count);
    pb.finish_with_message("done");

    // -----------------------------------------------------------------------
    // Print the Phase 1 Validation Gate table.
    // -----------------------------------------------------------------------
    let total_mb = stats.total_cache_bytes as f64 / 1_048_576.0;
    let avg_bytes = if stats.module_count > 0 {
        stats.total_cache_bytes / stats.module_count as u64
    } else {
        0
    };

    println!();
    println!("┌──────────────────────────────────────────────────────────────┐");
    println!("│ Wundler Phase 1 Validation Gate                              │");
    println!("├──────────────────────────────────────┬───────────────────────┤");
    println!(
        "│ Modules                              │ {:<21} │",
        stats.module_count
    );
    println!(
        "│ Total cache size                     │ {:<21} │",
        format!("{:.2} MB  (target: ~100 MB/50k)", total_mb)
    );
    println!(
        "│ Avg bytes / module                   │ {:<21} │",
        format!("{avg_bytes} B")
    );
    println!(
        "│ p50 latency                          │ {:<21} │",
        format!("{} µs", stats.p50_micros)
    );
    println!(
        "│ p95 latency                          │ {:<21} │",
        format!("{} µs", stats.p95_micros)
    );
    println!(
        "│ Dead-code estimate                   │ {:<21} │",
        format!("{:.1} %", stats.dead_code_estimate_pct)
    );
    println!("└──────────────────────────────────────┴───────────────────────┘");
    println!();

    // -----------------------------------------------------------------------
    // Gate check: per-module bytes must be ≤ 3 KB.
    // -----------------------------------------------------------------------
    let per_module_bytes = if stats.module_count > 0 {
        stats.total_cache_bytes / stats.module_count as u64
    } else {
        0
    };

    if per_module_bytes <= 3000 {
        println!("✓ PASS — {per_module_bytes}/module (target: ≤3 KB)");
    } else {
        println!("✗ FAIL — {per_module_bytes}/module exceeds 3 KB target.");
        std::process::exit(1);
    }

    Ok(())
}

/// Run the full bundle analysis on `path`, using the given entry points and
/// commons threshold, then print the resulting [`ChunkManifest`] as JSON.
fn run_analyze(path: PathBuf, entry_args: Vec<String>, commons_threshold: usize) -> Result<()> {
    // Parse every --entry argument.
    //
    // Entry paths are interpreted **relative to the current working directory**,
    // NOT relative to the scan root.  This prevents the "path-doubling" bug:
    // if the caller writes `--entry main=test-app/src/main.tsx` while scanning
    // `test-app/src`, the old `path.join(rel_path)` produced
    // `"test-app/src/test-app/src/main.tsx"` which does not exist.
    //
    // Using `PathBuf::from(rel_path)` keeps the path exactly as the user wrote
    // it, which matches the node paths that WalkDir records (e.g.
    // `"test-app/src/main.tsx"` when scan root is `"test-app/src"`).
    let entry_points: HashMap<String, PathBuf> = entry_args
        .iter()
        .map(|s| {
            let (route, rel_path) = parse_entry_arg(s)?;
            Ok((route, rel_path))
        })
        .collect::<Result<_>>()?;

    // Walk the directory and summarize every JS/TS file.
    let summarizer = ModuleSummarizer;
    let nodes = WalkDir::new(&path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let p = e.path().to_path_buf();
            let ext = p.extension()?.to_str()?.to_lowercase();
            if JS_EXTENSIONS.contains(&ext.as_str()) {
                Some(p)
            } else {
                None
            }
        })
        .map(|p| summarizer.summarize(&p))
        .collect::<Result<Vec<_>>>()
        .with_context(|| format!("failed to summarize directory {}", path.display()))?;

    // Configure and run the graph analyzer.
    let mut analyzer = GraphAnalyzer::new(entry_points);
    analyzer.commons_threshold = commons_threshold;
    let result = analyzer.analyze(nodes)?;

    // Print the manifest as pretty JSON.
    println!("{}", result.manifest.to_json()?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Summarize { path, cache_dir } => cmd_summarize(path, cache_dir),
        Commands::ValidateScale { path, cache_dir } => cmd_validate_scale(path, cache_dir),
        Commands::Analyze {
            path,
            entry,
            commons_threshold,
        } => run_analyze(path, entry, commons_threshold),
    }
}
