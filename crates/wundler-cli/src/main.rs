use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use walkdir::WalkDir;
use wundler_abs::server::AbsConfig;
use wundler_abs::signing::{generate_keypair, ManifestSigner};
use wundler_core::{cache::local::LocalCache, validation::run_validate_scale, ModuleSummarizer};
use wundler_graph::GraphAnalyzer;
use wundler_pipeline::{BuildConfig, BuildPipeline, DevServer, EngineChoice};

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

    /// Build the project using wundler.toml configuration.
    Build {
        /// Path to the wundler.toml configuration file.
        #[arg(long, default_value = "wundler.toml")]
        config: PathBuf,

        /// Override the bundling engine (swc|rolldown|rspack).
        #[arg(long)]
        engine: Option<String>,

        /// Sign the output manifest with an Ed25519 private key.
        #[arg(long)]
        sign: bool,

        /// Path to the Ed25519 signing key PEM file (required when --sign is set).
        #[arg(long)]
        key: Option<PathBuf>,
    },

    /// Start the development server.
    Dev {
        /// Path to the wundler.toml configuration file.
        #[arg(long, default_value = "wundler.toml")]
        config: PathBuf,

        /// Port to bind on.
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },

    /// Asset Bundling Server commands.
    Abs(AbsArgs),
}

// ---------------------------------------------------------------------------
// ABS subcommand types
// ---------------------------------------------------------------------------

/// Arguments for the `abs` subcommand.
#[derive(Parser)]
struct AbsArgs {
    #[command(subcommand)]
    command: AbsCmd,
}

/// Subcommands nested under `abs`.
#[derive(Subcommand)]
enum AbsCmd {
    /// Start the ABS HTTP server.
    Serve(AbsServeArgs),
    /// Generate an Ed25519 key pair for manifest signing.
    Keygen(AbsKeygenArgs),
}

/// Arguments for `wundler abs serve`.
#[derive(Parser)]
struct AbsServeArgs {
    /// Path to the ABS TOML configuration file.
    #[arg(long, default_value = "abs.toml")]
    config: PathBuf,
}

/// Arguments for `wundler abs keygen`.
#[derive(Parser)]
struct AbsKeygenArgs {
    /// Output path for the Ed25519 signing (private) key PEM file.
    #[arg(long, default_value = "signing.pem")]
    signing_out: PathBuf,

    /// Output path for the Ed25519 verifying (public) key PEM file.
    #[arg(long, default_value = "verifying.pem")]
    verifying_out: PathBuf,
}

// ---------------------------------------------------------------------------
// ABS TOML configuration schema
// ---------------------------------------------------------------------------

/// Schema for an `abs.toml` configuration file.
///
/// This is the on-disk representation; it is deserialized from TOML and then
/// converted into [`AbsConfig`] for the server.
#[derive(Deserialize)]
struct AbsTomlConfig {
    /// Path to the `ChunkManifest` JSON file on disk.
    manifest_path: PathBuf,
    /// Base URL of the CDN that serves chunk assets.
    cdn_base_url: String,
    /// Path to the append-only JSONL telemetry log.
    telemetry_log: PathBuf,
    /// Path to an Ed25519 signing key PEM file, or absent to disable signing.
    signing_key_pem: Option<PathBuf>,
    /// TCP port the server listens on (default: 8080).
    #[serde(default = "default_abs_port")]
    port: u16,
    /// How long clients should cache a manifest response, in seconds (default: 300).
    #[serde(default = "default_abs_ttl")]
    ttl_seconds: u64,
}

fn default_abs_port() -> u16 {
    8080
}

fn default_abs_ttl() -> u64 {
    300
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
// Build / Dev handlers
// ---------------------------------------------------------------------------

fn run_build(
    config_path: &Path,
    engine_override: Option<String>,
    sign: bool,
    key: Option<PathBuf>,
) -> Result<()> {
    let mut cfg = BuildConfig::load(config_path)?;
    let out_dir = cfg.out_dir.clone();

    if let Some(name) = engine_override {
        cfg.engine = match name.as_str() {
            "swc" => EngineChoice::Swc,
            "rolldown" => EngineChoice::Rolldown,
            "rspack" => EngineChoice::Rspack,
            other => anyhow::bail!(
                "unknown engine {:?}; expected one of: swc, rolldown, rspack",
                other
            ),
        };
    }

    let bar = ProgressBar::new_spinner();
    bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    bar.enable_steady_tick(std::time::Duration::from_millis(100));
    bar.set_message("building\u{2026}");

    let pipeline = BuildPipeline::new(cfg);
    let out = pipeline.build()?;

    bar.finish_and_clear();
    println!(
        "built {} chunks ({} modules alive, {} dead) in {} ms \u{2014} largest chunk {} bytes",
        out.stats.chunks_written,
        out.stats.alive_modules,
        out.stats.dead_modules,
        out.stats.build_time_ms,
        out.stats.largest_chunk_bytes,
    );

    // Optionally sign the manifest.
    if sign {
        let key_path = key
            .ok_or_else(|| anyhow::anyhow!("--sign requires --key <path> to the signing PEM"))?;
        let pem = std::fs::read_to_string(&key_path)
            .with_context(|| format!("failed to read signing key: {}", key_path.display()))?;
        let signer = ManifestSigner::from_pem(&pem)?;
        let sig = signer.sign_manifest(&out.manifest);
        let sig_path = out_dir.join("manifest.sig");
        std::fs::write(&sig_path, sig.to_bytes())
            .with_context(|| format!("failed to write manifest.sig to {}", sig_path.display()))?;
        eprintln!("signed manifest → {}", sig_path.display());
    }

    Ok(())
}

async fn run_dev(config_path: &Path, port: u16) -> Result<()> {
    let cfg = BuildConfig::load(config_path)?;
    println!(
        "wundler dev: serving {} on http://127.0.0.1:{}",
        cfg.root.display(),
        port
    );
    DevServer { root: cfg.root, port }.start().await
}

// ---------------------------------------------------------------------------
// ABS handlers
// ---------------------------------------------------------------------------

async fn run_abs_serve(config_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read ABS config: {}", config_path.display()))?;
    let toml_cfg: AbsTomlConfig =
        toml::from_str(&content).context("failed to parse ABS TOML config")?;

    let config = AbsConfig {
        manifest_path: toml_cfg.manifest_path,
        cdn_base_url: toml_cfg.cdn_base_url,
        telemetry_log: toml_cfg.telemetry_log,
        signing_key_pem: toml_cfg.signing_key_pem,
        port: toml_cfg.port,
        ttl_seconds: toml_cfg.ttl_seconds,
    };

    wundler_abs::server::run(config).await
}

fn run_abs_keygen(signing_out: &Path, verifying_out: &Path) -> Result<()> {
    let keypair = generate_keypair();

    // Create parent directories if needed. Guard against the empty-string
    // parent that `Path::parent()` returns for a bare filename like "key.pem".
    fn ensure_parent(p: &Path) -> Result<()> {
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create directory {}", parent.display())
                })?;
            }
        }
        Ok(())
    }

    ensure_parent(signing_out)?;
    ensure_parent(verifying_out)?;

    std::fs::write(signing_out, &keypair.signing_key_pem).with_context(|| {
        format!(
            "failed to write signing key to {}",
            signing_out.display()
        )
    })?;
    std::fs::write(verifying_out, &keypair.verifying_key_pem).with_context(|| {
        format!(
            "failed to write verifying key to {}",
            verifying_out.display()
        )
    })?;

    eprintln!("wrote signing key → {}", signing_out.display());
    eprintln!("wrote verifying key → {}", verifying_out.display());

    Ok(())
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
    println!("├──────────────────────────────────────────┬───────────────────────┤");
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
    println!("└──────────────────────────────────────────┴───────────────────────┘");
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
        Commands::Build {
            config,
            engine,
            sign,
            key,
        } => run_build(&config, engine, sign, key),
        Commands::Dev { config, port } => {
            tokio::runtime::Runtime::new()?.block_on(run_dev(&config, port))
        }
        Commands::Abs(abs_args) => match abs_args.command {
            AbsCmd::Serve(args) => {
                tokio::runtime::Runtime::new()?.block_on(run_abs_serve(&args.config))
            }
            AbsCmd::Keygen(args) => run_abs_keygen(&args.signing_out, &args.verifying_out),
        },
    }
}
