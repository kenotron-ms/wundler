//! `GraphAnalyzer` — wires reachability, DCE, chunk assignment, and manifest
//! assembly into a single end-to-end analysis pipeline.
//!
//! # Pipeline
//!
//! 1. Resolve every entry-point path to a [`ContentHash`].
//! 2. Compute alive modules with [`compute_reachability_with_sccs`].
//! 3. Compute dead exports with [`compute_dead_exports`].
//! 4. Annotate each node with its alive flag; trim dead Named exports from alive
//!    nodes so the transform layer does not have to recompute them.
//! 5. Assign modules to chunks via [`assign_chunks`].
//! 6. Annotate each node with its `chunk_id`.
//! 7. Sort chunks by `id` for a stable manifest.
//! 8. Assemble the [`ChunkManifest`] via [`build_manifest`].
//! 9. Compute [`AnalysisStats`] and return [`AnalysisResult`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use wundler_core::types::{BundleGraphNode, ContentHash};

use crate::chunks::assign_chunks;
use crate::dce::compute_dead_exports;
use crate::manifest::build_manifest;
use crate::reachability::compute_reachability_with_sccs;
use crate::types::ChunkManifest;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for the analysis pipeline.
///
/// `entry_points` maps route names (e.g. `"/a"`) to the filesystem path of
/// the corresponding entry module (e.g. `PathBuf::from("a")`).  The path
/// must match the `path` field of a [`BundleGraphNode`] in the node slice
/// passed to [`GraphAnalyzer::analyze`].
///
/// `commons_threshold` is the minimum number of candidate chunks a module
/// must appear in to be hoisted into the shared commons chunk.  Defaults to
/// `2` when constructed via [`GraphAnalyzer::new`].
pub struct GraphAnalyzer {
    pub entry_points: HashMap<String, PathBuf>,
    pub commons_threshold: usize,
}

/// The complete result of an analysis run.
pub struct AnalysisResult {
    /// Nodes with `alive` and `chunk_id` fields populated by the pipeline.
    pub nodes: Vec<BundleGraphNode>,
    /// The assembled chunk manifest.
    pub manifest: ChunkManifest,
    /// High-level statistics derived from the run.
    pub stats: AnalysisStats,
}

/// High-level statistics produced by a single analysis run.
pub struct AnalysisStats {
    /// Total number of nodes in the input graph.
    pub total: usize,
    /// Number of nodes that are reachable from at least one entry point.
    pub alive: usize,
    /// Number of nodes that are NOT reachable from any entry point.
    pub dead: usize,
    /// Total number of output chunks in the manifest.
    pub chunks: usize,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl GraphAnalyzer {
    /// Create a new [`GraphAnalyzer`] with `commons_threshold = 2`.
    pub fn new(entry_points: HashMap<String, PathBuf>) -> Self {
        Self {
            entry_points,
            commons_threshold: 2,
        }
    }

    /// Run the full analysis pipeline on a flat list of bundle graph nodes.
    ///
    /// # Errors
    ///
    /// Returns an error if any entry-point path cannot be resolved to a node
    /// in the provided `nodes` slice.
    pub fn analyze(&self, mut nodes: Vec<BundleGraphNode>) -> Result<AnalysisResult> {
        // ── Step 1: Resolve entry-point paths → hashes ───────────────────────
        //
        // Build path_to_hash inside a block so the shared borrow of `nodes`
        // ends before we need a mutable borrow below.
        let (entry_hashes, entry_hash_set) = {
            let path_to_hash: HashMap<&str, ContentHash> = nodes
                .iter()
                .map(|n| (n.path.as_str(), n.id.clone()))
                .collect();

            let mut entry_hashes: HashMap<String, ContentHash> = HashMap::new();
            let mut entry_hash_set: HashSet<ContentHash> = HashSet::new();

            for (route, path) in &self.entry_points {
                let path_str = path
                    .to_str()
                    .ok_or_else(|| anyhow!("route {route:?}: path is not valid UTF-8"))?;

                // Try to find the entry hash using multiple path formats.
                //
                // After the CLI Bug-1 fix, entry paths are no longer joined
                // with the scan root: they are stored exactly as the user
                // wrote them (e.g. `"src/index.ts"`).  WalkDir, however,
                // prepends `"./"` when the scan root is `"."`, producing
                // `"./src/index.ts"`.  For absolute scan roots it produces
                // absolute paths like `"/tmp/xxx/src/index.ts"`.  We therefore
                // try several fallback strategies in order.
                let hash = path_to_hash
                    .get(path_str)
                    // 1. With "./" prefix (WalkDir from scan root ".").
                    .or_else(|| path_to_hash.get(format!("./{path_str}").as_str()))
                    // 2. Suffix match (entry is relative to scan root, but
                    //    WalkDir produced an absolute or longer-prefix path).
                    .or_else(|| {
                        let suffix = format!("/{path_str}");
                        nodes
                            .iter()
                            .find(|n| n.path.ends_with(&suffix))
                            .and_then(|n| path_to_hash.get(n.path.as_str()))
                    })
                    .ok_or_else(|| {
                        anyhow!("route {route:?}: path {path_str:?} not found in graph")
                    })?;

                entry_hashes.insert(route.clone(), hash.clone());
                entry_hash_set.insert(hash.clone());
            }

            (entry_hashes, entry_hash_set)
        };
        // `path_to_hash` is dropped here; we no longer hold a shared borrow of
        // `nodes`.

        // ── Step 2: Reachability + dead-export analysis ───────────────────────
        let alive = compute_reachability_with_sccs(&nodes, &entry_hash_set);
        let dead_exports = compute_dead_exports(&nodes, &alive);

        // ── Step 3: Annotate nodes; trim dead exports from alive nodes ────────
        for n in &mut nodes {
            n.alive = alive.contains(&n.id);

            if n.alive {
                if let Some(dead_set) = dead_exports.get(&n.id) {
                    if !dead_set.is_empty() {
                        // Retain only exports that are NOT in the dead set.
                        n.summary.exports.retain(|e| !dead_set.contains(&e.name));
                    }
                }
            }
        }

        // ── Step 4: Assign modules to chunks ─────────────────────────────────
        let (mut chunks, module_index) =
            assign_chunks(&nodes, &alive, &entry_hashes, self.commons_threshold);

        // ── Step 5: Annotate each node with its chunk_id ──────────────────────
        for n in &mut nodes {
            n.chunk_id = module_index.get(&n.id).cloned();
        }

        // ── Step 6: Sort chunks by id for stable manifests ────────────────────
        chunks.sort_by(|a, b| a.id.cmp(&b.id));

        // ── Step 7: Build the manifest ────────────────────────────────────────
        let manifest = build_manifest(chunks, &entry_hashes, module_index);

        // ── Step 8: Compute stats ─────────────────────────────────────────────
        let total = nodes.len();
        let alive_count = nodes.iter().filter(|n| n.alive).count();
        let dead_count = total - alive_count;
        let chunks_count = manifest.chunks.len();

        let stats = AnalysisStats {
            total,
            alive: alive_count,
            dead: dead_count,
            chunks: chunks_count,
        };

        Ok(AnalysisResult {
            nodes,
            manifest,
            stats,
        })
    }
}
