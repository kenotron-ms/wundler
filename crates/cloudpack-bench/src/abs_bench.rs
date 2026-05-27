
//! ABS (Adaptive Bundle Service) delta-efficiency benchmark.
//!
//! Measures how many bytes a browser must download after a deploy, comparing:
//! - **Traditional CDN**: full bundle re-download on every deploy.
//! - **ABS delta**: only chunks whose module-hash sets changed.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use cloudpack_graph::types::ChunkManifest;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A snapshot of a single chunk's identity and size after a build.
#[derive(Debug, Clone)]
pub struct ChunkSnapshot {
    /// Stable logical chunk identifier (e.g. `"commons"`, `"initial_root"`).
    pub chunk_id: String,
    /// Content hashes of all modules in this chunk (used to detect changes).
    pub module_hashes: HashSet<String>,
    /// Size of the compiled chunk file in bytes.
    pub size_bytes: u64,
}

/// Result row for one churn level in the ABS benchmark.
#[derive(Debug, Clone)]
pub struct AbsChurnResult {
    /// Fraction of modules changed (e.g. `0.01` = 1 %).
    pub churn_fraction: f64,
    /// Sum of all chunk sizes from the initial (v1) build, in KB.
    pub total_bundle_kb: f64,
    /// Bytes the client must download after the v2 deploy, in KB.
    pub abs_download_kb: f64,
    /// Percentage saved: `1 - (abs_download_kb / total_bundle_kb)` × 100.
    pub savings_pct: f64,
}

/// ABS results for a single module-count scale.
#[derive(Debug, Clone)]
pub struct AbsScaleResult {
    /// Number of modules in this measurement.
    pub n_modules: usize,
    /// One row per churn level.
    pub churns: Vec<AbsChurnResult>,
}

// ---------------------------------------------------------------------------
// Snapshot helpers
// ---------------------------------------------------------------------------

/// Read `<out_dir>/manifest.json` and return a `ChunkSnapshot` per chunk.
///
/// The manifest written by the pipeline has **output hashes** in `chunk.hash`
/// (the file name component), so chunk file sizes can be resolved directly at
/// `<out_dir>/chunks/<chunk.hash>.js`.
pub fn snapshot_from_disk(out_dir: &Path) -> Result<Vec<ChunkSnapshot>> {
    let manifest_path = out_dir.join("manifest.json");
    let json = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read manifest at {}", manifest_path.display()))?;

    let manifest: ChunkManifest = ChunkManifest::from_json(&json)
        .context("failed to parse manifest.json")?;

    let mut snapshots = Vec::new();

    for chunk in &manifest.chunks {
        // `chunk.hash` in the on-disk manifest is the **output hash** (the
        // actual .js file name), set by `output::write_manifest`.
        let chunk_file = out_dir
            .join("chunks")
            .join(format!("{}.js", chunk.hash.as_str()));

        let size_bytes = std::fs::metadata(&chunk_file)
            .map(|m| m.len())
            .unwrap_or(0);

        let module_hashes: HashSet<String> =
            chunk.modules.iter().map(|h| h.as_str().to_string()).collect();

        snapshots.push(ChunkSnapshot {
            chunk_id: chunk.id.clone(),
            module_hashes,
            size_bytes,
        });
    }

    Ok(snapshots)
}

// ---------------------------------------------------------------------------
// Delta computation
// ---------------------------------------------------------------------------

/// Compute which v2 chunks a browser must download given its cached v1 state.
///
/// A chunk must be downloaded if:
/// - It exists in v2 but not in v1 (new chunk), **or**
/// - Its module-hash set differs between v1 and v2 (content changed).
///
/// Chunks that exist in v2 with an identical module-hash set are cache hits.
pub fn compute_delta(v1: &[ChunkSnapshot], v2: &[ChunkSnapshot]) -> Vec<ChunkSnapshot> {
    let v1_by_id: HashMap<&str, &ChunkSnapshot> =
        v1.iter().map(|c| (c.chunk_id.as_str(), c)).collect();

    v2.iter()
        .filter(|c2| match v1_by_id.get(c2.chunk_id.as_str()) {
            None => true,                                   // new chunk
            Some(c1) => c1.module_hashes != c2.module_hashes, // changed
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Full benchmark runner
// ---------------------------------------------------------------------------

/// Run the ABS delta-efficiency benchmark at a single module scale.
///
/// 1. Generates a fresh `SyntheticApp` with `n_modules` modules.
/// 2. Builds v1 and snapshots chunk sizes + module-hash sets.
/// 3. For each churn fraction:
///    a. Applies churn (touches the last `fraction × n_modules` modules).
///    b. Builds v2 and snapshots.
///    c. Computes the ABS delta (bytes that changed).
///    d. Records the result.
pub fn run(
    n_modules: usize,
    churns: &[f64],
) -> Result<Vec<AbsChurnResult>> {
    use crate::synthetic::SyntheticApp;

    let mut results = Vec::new();

    for &fraction in churns {
        // Generate a fresh app for each churn level so runs don't compound.
        let app_c = SyntheticApp::generate(n_modules)?;

        // v1 for this run
        let v1c_out = tempfile::TempDir::new()?;
        std::fs::create_dir_all(v1c_out.path())?;
        build_app(&app_c, v1c_out.path())?;
        let v1c_snap = snapshot_from_disk(v1c_out.path())?;

        // Apply churn
        let changed = app_c.churn(fraction)?;
        let _ = changed; // informational; not used in computation

        // v2 build
        let v2c_out = tempfile::TempDir::new()?;
        std::fs::create_dir_all(v2c_out.path())?;
        build_app(&app_c, v2c_out.path())?;
        let v2c_snap = snapshot_from_disk(v2c_out.path())?;

        // Delta
        let delta = compute_delta(&v1c_snap, &v2c_snap);
        let delta_bytes: u64 = delta.iter().map(|c| c.size_bytes).sum();
        let v2_total: u64 = v2c_snap.iter().map(|c| c.size_bytes).sum();
        let abs_download_kb = delta_bytes as f64 / 1024.0;
        let bundle_kb = v2_total as f64 / 1024.0;
        let savings_pct = if bundle_kb > 0.0 {
            (1.0 - (abs_download_kb / bundle_kb)) * 100.0
        } else {
            0.0
        };

        results.push(AbsChurnResult {
            churn_fraction: fraction,
            total_bundle_kb: bundle_kb,
            abs_download_kb,
            savings_pct: savings_pct.max(0.0),
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Multi-scale runner
// ---------------------------------------------------------------------------

/// Run the ABS benchmark at every module count in `scales` and every churn
/// fraction in `churns`, returning one [`AbsScaleResult`] per scale.
///
/// This is a thin wrapper around [`run`] that makes it easy to compare ABS
/// delta efficiency across different app sizes.
pub fn run_scales(scales: &[usize], churns: &[f64]) -> Result<Vec<AbsScaleResult>> {
    let mut out = Vec::with_capacity(scales.len());
    for &n in scales {
        eprintln!("  ABS bench N={} …", n);
        let churn_results = run(n, churns)?;
        out.push(AbsScaleResult {
            n_modules: n,
            churns: churn_results,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Internal helper: run a cloudpack SWC build
// ---------------------------------------------------------------------------

pub(crate) fn build_app(
    app: &crate::synthetic::SyntheticApp,
    out_dir: &Path,
) -> Result<()> {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use cloudpack_pipeline::config::{BuildConfig, EngineChoice};
    use cloudpack_pipeline::pipeline::BuildPipeline;

    std::fs::create_dir_all(out_dir)?;

    let config = BuildConfig {
        root: app.dir.path().to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: {
            let mut m = HashMap::new();
            m.insert("/".to_string(), PathBuf::from("src/main.tsx"));
            m
        },
        budget: None,
        dev: None,
    };

    let pipeline = BuildPipeline::new(config);
    pipeline.build().context("BuildPipeline::build() failed")?;
    Ok(())
}
