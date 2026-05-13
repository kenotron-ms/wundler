//! Path-indexed adjacency map and SCC analysis for the Wundler bundle dependency graph.
//!
//! This module provides:
//! * [`build_adjacency`] — converts a flat slice of [`BundleGraphNode`]s into a
//!   `HashMap<ContentHash, Vec<ContentHash>>` where each key is a source module
//!   and each value is the list of modules it directly imports.
//! * [`tarjan_sccs`] — detects strongly-connected components (circular import
//!   groups) in the dependency graph using Tarjan's algorithm via `petgraph`.
//! * [`resolve_specifier`] — resolves a relative import specifier (e.g. `"./App"`)
//!   to the matching node path (e.g. `"src/App.tsx"`) by trying TypeScript/JS
//!   extensions in order.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use wundler_core::types::{BundleGraphNode, ContentHash};

/// Build a path-indexed adjacency map from a slice of bundle graph nodes.
///
/// Each node's `path` is mapped to its `id` (`ContentHash`).  For every import
/// in a node's `summary.imports`, the import specifier is resolved against the
/// importer's directory via [`resolve_specifier`] and looked up in the path map.
/// If a match is found, the target hash is added to the source node's adjacency
/// entry.
///
/// # Specifier resolution
///
/// Import specifiers are resolved in two passes:
/// 1. **Exact match** — if the specifier is literally identical to a known node
///    path (e.g. synthetic test fixtures that use `"src/b.ts"` as a specifier).
/// 2. **Relative resolution** — specifiers starting with `"./"` or `"../"` are
///    joined with the importer's directory, normalised lexically, and then tried
///    with each TypeScript/JS extension in order (`.ts`, `.tsx`, `.js`, `.jsx`,
///    `/index.ts`, `/index.tsx`).
///
/// Bare package specifiers (e.g. `"react"`, `"lodash"`) are silently ignored.
///
/// # Path normalisation
///
/// Node paths that start with `"./"` (produced by `WalkDir::new(".")`) are
/// normalised by stripping that prefix before building the lookup table.  This
/// ensures that a node stored as `"./src/App.tsx"` can be found by the
/// resolved path `"src/App.tsx"`.
///
/// # Deduplication
///
/// A [`HashSet`] is used per source node so that multiple imports referencing
/// the same target path produce exactly one edge in the output.
///
/// # Unresolved imports
///
/// Imports whose specifier does not resolve to a known node (e.g. npm packages
/// or specifiers with no matching file) are silently ignored.
///
/// # Edge cases
///
/// * An empty `nodes` slice returns an empty `HashMap`.
/// * Self-imports (a node importing its own path) are preserved.
pub fn build_adjacency(nodes: &[BundleGraphNode]) -> HashMap<ContentHash, Vec<ContentHash>> {
    // Build a normalised path → ContentHash lookup.
    // Stripping the leading "./" ensures that WalkDir paths ("./src/App.tsx")
    // are matched by resolved specifiers ("src/App.tsx").
    let normalised_paths: Vec<String> = nodes
        .iter()
        .map(|n| strip_dot_slash(&n.path).to_owned())
        .collect();

    let hash_by_norm_path: HashMap<&str, &ContentHash> = normalised_paths
        .iter()
        .zip(nodes.iter())
        .map(|(norm, node)| (norm.as_str(), &node.id))
        .collect();

    let all_paths: HashSet<&str> = hash_by_norm_path.keys().copied().collect();

    let mut adjacency: HashMap<ContentHash, Vec<ContentHash>> = HashMap::new();

    for node in nodes {
        // Use a HashSet to deduplicate (source, target) pairs.  Multiple
        // Import rows may reference the same target with different ImportKinds
        // (e.g. static Named + Dynamic) — the chunker reads ImportKind directly
        // from node.summary.imports; this map only encodes reachability.
        let mut seen: HashSet<&ContentHash> = HashSet::new();
        let mut targets: Vec<ContentHash> = Vec::new();

        for import in &node.summary.imports {
            if let Some(target_path) = resolve_specifier(&node.path, &import.specifier, &all_paths)
            {
                if let Some(target_hash) = hash_by_norm_path.get(target_path) {
                    if seen.insert(target_hash) {
                        targets.push((*target_hash).clone());
                    }
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

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Strip a leading `"./"` from a path string.  This normalises WalkDir entries
/// produced when the scan root is `"."` (e.g. `"./src/App.tsx"` → `"src/App.tsx"`).
/// Absolute paths and paths that do not start with `"./"` are returned unchanged.
pub(crate) fn strip_dot_slash(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

/// Lexically normalise a path string by resolving `.` and `..` components
/// without touching the filesystem.
///
/// * Empty components (consecutive `/`) and `.` components are skipped.
/// * `..` components pop the last accumulated component.
/// * A leading `/` is preserved so that absolute paths remain absolute.
///
/// # Examples
///
/// ```text
/// "src/./App"           → "src/App"
/// "./test-app/src/./B"  → "test-app/src/B"
/// "/tmp/foo/../bar"     → "/tmp/bar"
/// "/abs/./path"         → "/abs/path"
/// ```
pub(crate) fn normalize_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "." | "" => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    let joined = components.join("/");
    if is_absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Resolve a relative import specifier to a node path.
///
/// Given an importer path and a relative specifier (e.g. `"./App"`), this
/// function returns the matching entry from `all_paths` by:
///
/// 1. **Exact match** — if the specifier string is already a member of
///    `all_paths`, it is returned as-is.  This handles synthetic test fixtures
///    where the specifier equals the full node path (e.g. `"src/b.ts"`).
///
/// 2. **Relative resolution** — the specifier (must start with `"."`) is
///    joined with the importer's directory, lexically normalised, and then
///    tried with these extensions in order:
///    - no extension (bare match)
///    - `.ts`, `.tsx`, `.js`, `.jsx`
///    - `/index.ts`, `/index.tsx`, `/index.js`, `/index.jsx`
///
/// Bare package specifiers that do not start with `"."` return `None` (they
/// are external modules like `"react"` or `"lodash"`).
///
/// # Arguments
///
/// * `importer_path` — path of the module containing the import statement.
/// * `specifier` — the raw import specifier string (e.g. `"./App"`, `"react"`).
/// * `all_paths` — set of normalised (leading `"./"` stripped) node paths to
///   search within.
///
/// # Returns
///
/// `Some(&str)` pointing to the matched entry in `all_paths`, or `None`.
pub(crate) fn resolve_specifier<'a>(
    importer_path: &str,
    specifier: &str,
    all_paths: &HashSet<&'a str>,
) -> Option<&'a str> {
    // 1. Exact match — handles synthetic fixtures where specifier == node path.
    if let Some(&p) = all_paths.get(specifier) {
        return Some(p);
    }

    // 2. Only relative specifiers can be resolved further.
    if !specifier.starts_with('.') {
        return None;
    }

    // Resolve relative to the importer's directory.
    let importer_dir = Path::new(importer_path).parent()?;
    let raw = importer_dir.join(specifier);
    let raw_str = raw.to_string_lossy();
    let normalized = normalize_path(&raw_str);

    // Try the normalised path without extension.
    if let Some(&p) = all_paths.get(normalized.as_str()) {
        return Some(p);
    }

    // Try with each JS/TS extension.
    for ext in &[".ts", ".tsx", ".js", ".jsx"] {
        let candidate = format!("{normalized}{ext}");
        if let Some(&p) = all_paths.get(candidate.as_str()) {
            return Some(p);
        }
    }

    // Try as directory index.
    for ext in &["/index.ts", "/index.tsx", "/index.js", "/index.jsx"] {
        let candidate = format!("{normalized}{ext}");
        if let Some(&p) = all_paths.get(candidate.as_str()) {
            return Some(p);
        }
    }

    None
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
        .map(|component| component.into_iter().map(|ix| graph[ix].clone()).collect())
        .collect()
}
