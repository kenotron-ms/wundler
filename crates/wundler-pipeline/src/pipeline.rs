//! `BuildPipeline` — orchestrates the full build from summarize → analyze → transform → emit.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use wundler_core::cache::local::LocalCache;
use wundler_core::summarizer::summarize_directory;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use wundler_transform::engine::{ChunkOutput, TransformDecisions, TransformEngine};
use wundler_transform::rolldown_adapter::{RolldownAdapter, RolldownAdapterConfig};
use wundler_transform::swc_adapter::{SwcAdapterConfig, SwcTransformAdapter};

use crate::config::{BuildConfig, EngineChoice};

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
}
