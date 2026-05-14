//! `BuildPipeline` — orchestrates the full build from summarize → analyze → transform → emit.

use std::sync::Arc;

use anyhow::{Context, Result};

use wundler_core::cache::local::LocalCache;
use wundler_core::summarizer::summarize_directory;
use wundler_core::types::BundleGraphNode;
use wundler_graph::analyzer::{AnalysisResult, GraphAnalyzer};
use wundler_transform::engine::TransformEngine;
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
}
