//! Path-indexed adjacency map and SCC analysis for the Wundler bundle dependency graph.
//!
//! This module provides:
//! * [`build_adjacency`] — converts a flat slice of [`BundleGraphNode`]s into a
//!   `HashMap<ContentHash, Vec<ContentHash>>` where each key is a source module
//!   and each value is the list of modules it directly imports.
//! * [`tarjan_sccs`] — detects strongly-connected components (circular import
//!   groups) in the dependency graph using Tarjan's algorithm via `petgraph`.

use std::collections::{HashMap, HashSet};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
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

/// Detect strongly-connected components (circular import groups) in the
/// dependency graph using Tarjan's algorithm.
///
/// Builds a directed graph from `nodes` and `adj`, then delegates to
/// [`petgraph::algo::tarjan_scc`].  Each returned inner `Vec` is one SCC:
/// * Modules involved in a circular import chain appear together in the same
///   `Vec` (size ≥ 2).
/// * Modules with no cycle produce singleton `Vec`s (size 1).
///
/// # Ordering
///
/// The outer `Vec` follows petgraph's reverse-topological order (dependencies
/// before dependents).  Order within each SCC is implementation-defined but
/// stable for a given input.
///
/// # Edge cases
///
/// * An empty `nodes` slice returns an empty `Vec`.
/// * Edges in `adj` whose target hash is not present in `nodes` are silently
///   ignored (consistent with [`build_adjacency`]).
pub fn tarjan_sccs(
    nodes: &[BundleGraphNode],
    adj: &HashMap<ContentHash, Vec<ContentHash>>,
) -> Vec<Vec<ContentHash>> {
    if nodes.is_empty() {
        return Vec::new();
    }

    // Build a directed petgraph graph with ContentHash as node weight.
    let mut graph: DiGraph<ContentHash, ()> = DiGraph::new();

    // Add one node per BundleGraphNode and record its NodeIndex.
    let mut idx_of: HashMap<ContentHash, NodeIndex> = HashMap::new();
    for node in nodes {
        let ix = graph.add_node(node.id.clone());
        idx_of.insert(node.id.clone(), ix);
    }

    // Add directed edges for every adjacency entry whose target exists in the
    // graph.  Targets that are not in `idx_of` (external / unresolved) are
    // silently skipped.
    for (src_hash, targets) in adj {
        if let Some(&src_ix) = idx_of.get(src_hash) {
            for tgt_hash in targets {
                if let Some(&tgt_ix) = idx_of.get(tgt_hash) {
                    graph.add_edge(src_ix, tgt_ix, ());
                }
            }
        }
    }

    // Run Tarjan's SCC algorithm and map NodeIndex back to ContentHash.
    tarjan_scc(&graph)
        .into_iter()
        .map(|component| {
            component
                .into_iter()
                .map(|ix| graph[ix].clone())
                .collect()
        })
        .collect()
}
