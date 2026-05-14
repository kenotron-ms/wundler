//! `BuildPipeline` — orchestrates the full build from summarize → analyze → transform → emit.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use wundler_core::cache::local::LocalCache;
use wundler_core::summarizer::summarize_directory;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use wundler_graph::types::ChunkManifest;
use wundler_transform::engine::{ChunkOutput, TransformDecisions, TransformEngine};
use wundler_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
use wundler_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

use crate::config::{BuildConfig, EngineChoice};
use crate::output;

// ---------------------------------------------------------------------------
// Build output types
// ---------------------------------------------------------------------------

/// Statistics produced by a complete [`BuildPipeline::build()`] run.
#[derive(Debug, Clone)]
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

    /// Step 3: Run the configured [`TransformEngine`] over every chunk in parallel.
    ///
    /// Derives [`TransformDecisions`] from per-node export aliveness (if the
    /// `Export` type carries an `alive` flag; otherwise the decisions map is
    /// empty and tree-shaking degrades gracefully to module-level only).
    ///
    /// Chunks are processed in parallel via Rayon, returning one [`ChunkOutput`]
    /// per chunk in the manifest.
    pub fn run_transform(&self, analysis: &AnalysisResult) -> Result<Vec<ChunkOutput>> {
        // Build a fast lookup from content-hash → node reference.
        let node_index: HashMap<&ContentHash, &BundleGraphNode> =
            analysis.nodes.iter().map(|n| (&n.id, n)).collect();

        // Build dead_exports from per-export aliveness.
        // NOTE: The `Export` type does not currently carry an `alive: bool` field
        // (that requires a future plan).  Until then the map stays empty and
        // tree-shaking is module-level only — alive nodes are included whole,
        // dead nodes are excluded from the chunk's module list entirely.
        let dead_exports: HashMap<ContentHash, HashSet<String>> = {
            let mut map: HashMap<ContentHash, HashSet<String>> = HashMap::new();
            for node in &analysis.nodes {
                if !node.alive {
                    continue;
                }
                // Collect any export names whose `alive` flag is false.
                // Currently `Export` has no such field, so this block never
                // inserts anything — see the note above.
                let dead_names: HashSet<String> = node
                    .summary
                    .exports
                    .iter()
                    .filter(|_e| {
                        // Export::alive does not exist yet; always false here.
                        false
                    })
                    .map(|e| e.name.clone())
                    .collect();
                if !dead_names.is_empty() {
                    map.insert(node.id.clone(), dead_names);
                }
            }
            map
        };

        let decisions = TransformDecisions { dead_exports };
        let engine = Arc::clone(&self.engine);
        let chunks = &analysis.manifest.chunks;

        let outputs: Result<Vec<ChunkOutput>> = chunks
            .par_iter()
            .map(|chunk| -> Result<ChunkOutput> {
                // Collect the alive modules that belong to this chunk.
                let modules: Vec<BundleGraphNode> = chunk
                    .modules
                    .iter()
                    .filter_map(|h| node_index.get(h).map(|n| (*n).clone()))
                    .collect();

                engine
                    .transform_chunk(&modules, chunk, &decisions)
                    .map_err(|e| {
                        anyhow::anyhow!("transform_chunk({}): {}", chunk.id, e)
                    })
            })
            .collect();

        outputs
    }

    // -----------------------------------------------------------------------
    // Step 4 — End-to-end build
    // -----------------------------------------------------------------------

    /// Run all four pipeline steps and write every artifact to `config.out_dir`.
    ///
    /// 1. Summarize modules under `config.root`.
    /// 2. Load source text for every discovered module.
    /// 3. Analyze the dependency graph.
    /// 4. Transform each chunk in parallel.
    /// 5. Write each `ChunkOutput` to `<out_dir>/chunks/<hash>.js`.
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

        // ----- Step 4a: Write chunks -----
        let out_dir = &self.config.out_dir;
        let mut chunk_files: Vec<PathBuf> = Vec::with_capacity(outputs.len());
        let mut largest_chunk_bytes: usize = 0;

        for chunk_output in &outputs {
            let path = output::write_chunk(out_dir, chunk_output)
                .with_context(|| format!("write_chunk failed for chunk {}", chunk_output.chunk_id))?;
            largest_chunk_bytes = largest_chunk_bytes.max(chunk_output.code.len());
            chunk_files.push(path);
        }

        let chunks_written = chunk_files.len();

        // ----- Step 4b: Write manifest -----
        output::write_manifest(out_dir, &analysis.manifest)
            .context("write_manifest failed")?;

        // ----- Step 4c: Write index.html for every entry point -----
        for (entry, _) in &self.config.entry_points {
            output::write_index_html(out_dir, &analysis.manifest, entry)
                .with_context(|| format!("write_index_html failed for entry '{entry}'"))?;
        }

        // ----- Compute stats -----
        let total_modules = analysis.nodes.len();
        let alive_modules = analysis.nodes.iter().filter(|n| n.alive).count();
        let dead_modules = total_modules - alive_modules;
        let build_time_ms = build_start.elapsed().as_millis();

        Ok(BuildOutput {
            manifest: analysis.manifest,
            chunk_files,
            stats: BuildStats {
                total_modules,
                alive_modules,
                dead_modules,
                chunks_written,
                build_time_ms,
                largest_chunk_bytes,
            },
        })
    }
}
