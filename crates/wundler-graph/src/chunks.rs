//! Initial chunk assignment for the Wundler bundle graph.
//!
//! This module assigns modules to *initial* chunks — the chunks that are part
//! of the entry-point HTML payload and loaded on first page render.
//!
//! # Algorithm
//!
//! For each entry route (sorted for determinism):
//! 1. Derive a `chunk_id` from the route name via [`chunk_id_slug`].
//! 2. Run a static-import-only BFS from the entry module to collect all
//!    transitively reachable modules via [`static_bfs_chunk`].
//! 3. Use a `module_index` map with first-seen-wins semantics so that a module
//!    claimed by an earlier route is not re-assigned to a later one.
//! 4. Emit a [`Chunk`] with [`LoadCondition::Initial`] and a placeholder
//!    content hash (`ContentHash("")`); real hashing is performed in a later
//!    pass.

use std::collections::{HashMap, HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash, ImportKind};

use crate::types::{Chunk, ChunkId, LoadCondition};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Assign modules to initial chunks, one per alive entry point.
///
/// # Parameters
///
/// * `nodes` — complete set of bundle graph nodes.
/// * `alive` — set of module hashes that survived the reachability pass.
/// * `entry_hashes` — map from route name (e.g. `"main"`) to its entry
///   module `ContentHash`.
/// * `_commons_threshold` — reserved for the commons-chunk pass (Task 11);
///   unused here.
///
/// # Returns
///
/// A tuple of:
/// * `Vec<Chunk>` — one chunk per alive entry route, in sorted-route order.
/// * `HashMap<ContentHash, ChunkId>` — reverse index mapping each assigned
///   module hash to the chunk it was first assigned to.
pub fn assign_chunks(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
    entry_hashes: &HashMap<String, ContentHash>,
    _commons_threshold: usize,
) -> (Vec<Chunk>, HashMap<ContentHash, ChunkId>) {
    // -----------------------------------------------------------------------
    // Build lookup indices.
    // -----------------------------------------------------------------------

    // ContentHash → node reference.
    let by_hash: HashMap<ContentHash, &BundleGraphNode> =
        nodes.iter().map(|n| (n.id.clone(), n)).collect();

    // Path string → ContentHash reference.
    let path_to_hash: HashMap<&str, &ContentHash> =
        nodes.iter().map(|n| (n.path.as_str(), &n.id)).collect();

    // -----------------------------------------------------------------------
    // Sort routes for deterministic chunk-id assignment.
    // -----------------------------------------------------------------------

    let mut routes: Vec<&String> = entry_hashes.keys().collect();
    routes.sort();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut module_index: HashMap<ContentHash, ChunkId> = HashMap::new();

    // -----------------------------------------------------------------------
    // One chunk per alive entry route.
    // -----------------------------------------------------------------------

    for route in routes {
        // Unwrap is safe: `route` came from `entry_hashes.keys()`.
        let entry_hash = entry_hashes.get(route).expect("route key must exist");

        // Skip routes whose entry module is not alive.
        if !alive.contains(entry_hash) {
            continue;
        }

        let chunk_id = format!("initial_{}", chunk_id_slug(route));

        // BFS over static imports only, collecting all reachable alive modules.
        let bfs_modules =
            static_bfs_chunk(entry_hash, &by_hash, &path_to_hash, alive);

        // First-seen-wins: assign each module to this chunk only if it has not
        // yet been claimed by an earlier chunk.
        let mut chunk_modules: Vec<ContentHash> = Vec::new();
        for m in bfs_modules {
            let assigned_to = module_index
                .entry(m.clone())
                .or_insert_with(|| chunk_id.clone());
            if assigned_to == &chunk_id {
                chunk_modules.push(m);
            }
        }

        chunks.push(Chunk {
            id: chunk_id,
            modules: chunk_modules,
            // Placeholder — real content hash is computed in Task 11.
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Initial,
            // PGO fields — not yet computed.
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    (chunks, module_index)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// BFS over **static** imports only, starting from `root`.
///
/// Returns a `Vec<ContentHash>` of all modules reachable via non-dynamic
/// import edges whose targets are in the `alive` set.  `root` itself is
/// included as the first element if it is alive.
///
/// # Static vs dynamic
///
/// An import is considered *static* if `import.kind != ImportKind::Dynamic`.
/// Dynamic `import()` boundaries create separate lazy chunks and are therefore
/// not traversed here.
///
/// # Dead modules
///
/// Targets not present in `alive` (or whose paths cannot be resolved via
/// `path_to_hash`) are silently skipped.
fn static_bfs_chunk<'a>(
    root: &ContentHash,
    by_hash: &HashMap<ContentHash, &'a BundleGraphNode>,
    path_to_hash: &HashMap<&str, &'a ContentHash>,
    alive: &HashSet<ContentHash>,
) -> Vec<ContentHash> {
    let mut visited: HashSet<ContentHash> = HashSet::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();
    let mut result: Vec<ContentHash> = Vec::new();

    // Seed the BFS with the root (only if alive).
    if alive.contains(root) && visited.insert(root.clone()) {
        queue.push_back(root.clone());
    }

    while let Some(current) = queue.pop_front() {
        result.push(current.clone());

        let node = match by_hash.get(&current) {
            Some(n) => n,
            None => continue, // Unknown hash — skip.
        };

        for import in &node.summary.imports {
            // Skip dynamic imports — they do not belong in an INITIAL chunk.
            if import.kind == ImportKind::Dynamic {
                continue;
            }

            // Resolve the specifier to a content hash via the path index.
            let target_hash = match path_to_hash.get(import.specifier.as_str()) {
                Some(h) => *h,
                None => continue, // Unresolved specifier (e.g. npm package) — skip.
            };

            // Skip dead targets.
            if !alive.contains(target_hash) {
                continue;
            }

            if visited.insert(target_hash.clone()) {
                queue.push_back(target_hash.clone());
            }
        }
    }

    result
}

/// Convert a route string into a safe chunk-id slug.
///
/// Replaces every non-alphanumeric character with `_`.  If the result is
/// empty or consists entirely of underscores, returns `"root"` instead.
///
/// # Examples
///
/// ```text
/// "main"        → "main"
/// "admin/index" → "admin_index"
/// "/"           → "root"
/// ""            → "root"
/// ```
fn chunk_id_slug(route: &str) -> String {
    let slug: String = route
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    if slug.is_empty() || slug.chars().all(|c| c == '_') {
        "root".to_string()
    } else {
        slug
    }
}

// ---------------------------------------------------------------------------
// Unit tests for slug helper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::chunk_id_slug;

    #[test]
    fn slug_alphanumeric_unchanged() {
        assert_eq!(chunk_id_slug("main"), "main");
        assert_eq!(chunk_id_slug("admin123"), "admin123");
    }

    #[test]
    fn slug_replaces_non_alphanumeric_with_underscore() {
        assert_eq!(chunk_id_slug("admin/index"), "admin_index");
        assert_eq!(chunk_id_slug("my-route"), "my_route");
    }

    #[test]
    fn slug_all_underscores_returns_root() {
        assert_eq!(chunk_id_slug("/"), "root");
        assert_eq!(chunk_id_slug("//"), "root");
        assert_eq!(chunk_id_slug(""), "root");
    }
}
