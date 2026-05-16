//! `BuildPipeline` — orchestrates the full build from summarize → analyze → transform → emit.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use wundler_core::cache::local::LocalCache;
use wundler_core::summarizer::summarize_directory;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use wundler_graph::types::ChunkManifest;
use wundler_transform::engine::{BatchConfig, ChunkOutput, TransformEngine};
use wundler_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
use wundler_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

use crate::config::{BuildConfig, EngineChoice};
use crate::output;

// ---------------------------------------------------------------------------
// Build output types
// ---------------------------------------------------------------------------

/// Statistics produced by a complete [`BuildPipeline::build()`] run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildStats {
    /// Total number of modules discovered by the summarize step.
    pub total_modules: usize,
    /// Number of modules reachable from at least one entry point.
    pub alive_modules: usize,
    /// Number of modules NOT reachable from any entry point.
    pub dead_modules: usize,
    /// Number of chunk `.js` files written to disk.
    pub chunks_written: usize,
    /// Wall-clock build time in milliseconds.
    pub build_time_ms: u128,
    /// Size in bytes of the largest emitted chunk.
    pub largest_chunk_bytes: usize,
}

/// The result returned by [`BuildPipeline::build()`].
#[derive(Debug)]
pub struct BuildOutput {
    /// The fully-assembled chunk manifest.
    pub manifest: ChunkManifest,
    /// Paths of every `.js` chunk file written to disk.
    pub chunk_files: Vec<PathBuf>,
    /// High-level build statistics.
    pub stats: BuildStats,
}

/// Orchestrates the Wundler build pipeline: summarize → analyze → transform → emit.
pub struct BuildPipeline {
    pub config: BuildConfig,
    pub engine: Arc<dyn TransformEngine>,
}

impl BuildPipeline {
    /// Create a new `BuildPipeline`, selecting the `TransformEngine` based on `config.engine`.
    pub fn new(config: BuildConfig) -> Self {
        let engine: Arc<dyn TransformEngine> = match config.engine {
            EngineChoice::Swc => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
            EngineChoice::Rolldown => {
                Arc::new(RolldownAdapter::with_config(RolldownAdapterConfig::default()))
            }
            // Rspack interface only — fall back to SWC for now.
            EngineChoice::Rspack => Arc::new(SwcTransformAdapter::with_config(SwcAdapterConfig {
                source_maps: config.source_maps,
            })),
        };
        Self { config, engine }
    }

    /// Step 1: Walk `config.root` and summarize every JS/TS module found.
    ///
    /// Uses the default content-addressed local cache (`~/.wundler/cache/summaries/`).
    pub fn run_summarize(&self) -> Result<Vec<BundleGraphNode>> {
        let cache = LocalCache::with_default_root()
            .context("failed to initialise local summary cache")?;
        let nodes = summarize_directory(&self.config.root, &cache)
            .with_context(|| {
                format!(
                    "summarize_directory failed for root: {}",
                    self.config.root.display()
                )
            })?;
        Ok(nodes)
    }

    /// Step 2: Run the Plan 2 graph analyzer on a slice of bundle graph nodes.
    ///
    /// Resolves entry-point paths from `config.entry_points`, computes module
    /// reachability, assigns modules to chunks, and assembles the
    /// [`ChunkManifest`].
    pub fn run_analyze(&self, nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        let mut analyzer = GraphAnalyzer::new(self.config.entry_points.clone());
        analyzer.commons_threshold = self.config.commons_threshold;
        let result = analyzer
            .analyze(nodes)
            .with_context(|| "graph analysis failed")?;
        Ok(result)
    }

    /// Step 3: Run the configured [`TransformEngine`] in batch mode.
    ///
    /// Calls [`TransformEngine::batch_transform`] which, for the SWC engine,
    /// delegates to `transform_chunk` per chunk, and for the rolldown engine,
    /// invokes the rolldown CLI once for the whole project.
    pub fn run_transform(&self, analysis: &AnalysisResult) -> Result<Vec<ChunkOutput>> {
        let batch_config = BatchConfig {
            root: self.config.root.clone(),
            entry_points: self.config.entry_points.clone(),
            out_dir: self.config.out_dir.clone(),
        };
        self.engine
            .batch_transform(analysis, &batch_config)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    // -----------------------------------------------------------------------
    // Step 4 — End-to-end build
    // -----------------------------------------------------------------------

    /// Run all four pipeline steps and write every artifact to `config.out_dir`.
    ///
    /// 1. Summarize modules under `config.root`.
    /// 2. Load source text for every discovered module.
    /// 3. Analyze the dependency graph.
    /// 4. Transform each chunk (batch or per-chunk depending on engine).
    /// 5. For SWC: write each `ChunkOutput` to `<out_dir>/chunks/<hash>.js`.
    ///    For rolldown: files are already in `out_dir`; skip `write_chunk`.
    /// 6. Write `<out_dir>/manifest.json`.
    /// 7. Write `<out_dir>/index.html` for every entry point.
    ///
    /// Returns a [`BuildOutput`] with the assembled manifest, the paths of all
    /// written chunk files, and aggregate build statistics.
    pub fn build(&self) -> Result<BuildOutput> {
        let build_start = std::time::Instant::now();

        // ----- Step 1: Summarize -----
        let mut nodes = self.run_summarize()?;

        // ----- Load source for each node -----
        // The transform engine needs `node.source` to be populated; the
        // summarizer does not load it (it only produces the summary metadata).
        for node in &mut nodes {
            let source = std::fs::read_to_string(&node.path)
                .unwrap_or_else(|_| "// empty\n".to_string());
            node.source = Some(source);
        }

        // ----- Step 2: Analyze -----
        let analysis = self.run_analyze(nodes)?;

        // ----- Step 3: Transform -----
        let outputs = self.run_transform(&analysis)?;

        // Detect whether the engine wrote files directly (rolldown batch path).
        let already_written = outputs.iter().any(|o| o.already_written);

        // Build a mapping from chunk_id → actual output hash so that
        // write_index_html and write_manifest can reference real file names
        // rather than the analysis-phase hashes stored in the ChunkManifest.
        let id_to_output_hash: HashMap<String, ContentHash> = outputs
            .iter()
            .map(|o| (o.chunk_id.clone(), o.hash.clone()))
            .collect();

        let out_dir = &self.config.out_dir;
        let mut chunk_files: Vec<PathBuf> = Vec::with_capacity(outputs.len());
        let mut largest_chunk_bytes: usize = 0;

        if already_written {
            // Rolldown path: files are already in out_dir.
            // Enumerate them so we can report chunk_files / stats.
            for dir_entry in std::fs::read_dir(out_dir)
                .with_context(|| format!("reading out_dir {}", out_dir.display()))?
            {
                let dir_entry = dir_entry?;
                let path = dir_entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("js") {
                    let size = std::fs::metadata(&path).map(|m| m.len() as usize).unwrap_or(0);
                    largest_chunk_bytes = largest_chunk_bytes.max(size);
                    chunk_files.push(path);
                }
            }
        } else {
            // SWC path: write each ChunkOutput to chunks/<hash>.js.
            for chunk_output in &outputs {
                let path = output::write_chunk(out_dir, chunk_output)
                    .with_context(|| {
                        format!("write_chunk failed for chunk {}", chunk_output.chunk_id)
                    })?;
                largest_chunk_bytes = largest_chunk_bytes.max(chunk_output.code.len());
                chunk_files.push(path);
            }
        }

        let chunks_written = chunk_files.len();

        // ----- Step 4b: Write manifest -----
        output::write_manifest(out_dir, &analysis.manifest, &id_to_output_hash)
            .context("write_manifest failed")?;

        // ----- Step 4c: Write index.html for every entry point -----
        for entry in self.config.entry_points.keys() {
            if already_written {
                // Rolldown path: generate index.html that references rolldown's
                // actual output files (e.g., `root-abc123.js`) directly.
                output::write_rolldown_index_html(out_dir, entry)
                    .with_context(|| {
                        format!("write_rolldown_index_html failed for entry '{entry}'")
                    })?;
            } else {
                // SWC path: use the manifest-based HTML generator.
                output::write_index_html(out_dir, &analysis.manifest, entry, &id_to_output_hash)
                    .with_context(|| {
                        format!("write_index_html failed for entry '{entry}'")
                    })?;
            }
        }

        // ----- Compute stats -----
        let total_modules = analysis.nodes.len();
        let alive_modules = analysis.nodes.iter().filter(|n| n.alive).count();
        let dead_modules = total_modules - alive_modules;
        let build_time_ms = build_start.elapsed().as_millis();

        let stats = BuildStats {
            total_modules,
            alive_modules,
            dead_modules,
            chunks_written,
            build_time_ms,
            largest_chunk_bytes,
        };

        // ----- Step 4d: Write build-stats.json (non-fatal) -----
        let stats_path = out_dir.join("build-stats.json");
        match std::fs::File::create(&stats_path) {
            Ok(f) => {
                if let Err(e) = serde_json::to_writer_pretty(f, &stats) {
                    eprintln!("warning: failed to write build-stats.json: {e}");
                }
            }
            Err(e) => {
                eprintln!("warning: failed to create build-stats.json: {e}");
            }
        }

        Ok(BuildOutput {
            manifest: analysis.manifest,
            chunk_files,
            stats,
        })
    }
}
