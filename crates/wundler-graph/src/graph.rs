//! Path-indexed adjacency map for the Wundler bundle dependency graph.
//!
//! This module provides [`build_adjacency`], which converts a flat slice of
//! [`BundleGraphNode`]s into a `HashMap<ContentHash, Vec<ContentHash>>`
//! where each key is a source module and each value is the list of modules
//! it directly imports (by path resolution within the graph).

use std::collections::{HashMap, HashSet};

use wundler_core::types::{BundleGraphNode, ContentHash};

/// Build a path-indexed adjacency map from a slice of bundle graph nodes.
///
/// Each node's `path` is mapped to its `id` (`ContentHash`).  For every import
/// in a node's `summary.imports`, the import specifier is looked up in that
/// path map.  If a match is found, the target hash is added to the source
/// node's adjacency entry.
///
/// # Deduplication
///
/// A [`HashSet`] is used per source node so that multiple imports referencing
/// the same target path (e.g. one static `Named` import and one `Dynamic`
/// import of the same module) produce exactly one edge in the output.
///
/// # Unresolved imports
///
/// Imports whose specifier does not resolve to a node in the provided slice
/// (e.g. bare npm package specifiers such as `"react"`) are silently ignored.
///
/// # Edge cases
///
/// * An empty `nodes` slice returns an empty `HashMap`.
/// * Self-imports (a node importing its own path) are preserved.
pub fn build_adjacency(nodes: &[BundleGraphNode]) -> HashMap<ContentHash, Vec<ContentHash>> {
    // Build path → ContentHash lookup for all nodes in the graph.
    let path_to_hash: HashMap<&str, &ContentHash> = nodes
        .iter()
        .map(|node| (node.path.as_str(), &node.id))
        .collect();

    let mut adjacency: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();

    for node in nodes {
        // Use a HashSet to deduplicate (source, target) pairs.  Multiple
        // Import rows may reference the same target with different ImportKinds
        // (e.g. static Named + Dynamic) — the chunker reads ImportKind directly
        // from node.summary.imports; this map only encodes reachability.
        let mut seen: HashSet<&ContentHash> = HashSet::new();
        let mut targets: Vec<ContentHash> = Vec::new();

        for import in &node.summary.imports {
            if let Some(target_hash) = path_to_hash.get(import.specifier.as_str()) {
                if seen.insert(target_hash) {
                    targets.push((*target_hash).clone());
                }
            }
            // Unresolved specifier (e.g. npm package) — silently dropped.
        }

        if !targets.is_empty() {
            adjacency.insert(node.id.clone(), targets);
        }
    }

    adjacency
}
