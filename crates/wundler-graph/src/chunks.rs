//! Chunk assignment for the Wundler bundle graph.
//!
//! This module assigns modules to chunks:
//!
//! * **Initial chunks** — one per entry route, loaded on first page render.
//! * **Lazy chunks** — one per `import()` boundary, fetched on demand.
//!
//! # Algorithm
//!
//! For each entry route (sorted for determinism):
//! 1. Derive a `chunk_id` from the route name via [`chunk_id_slug`].
//! 2. Run a static-import-only BFS from the entry module via
//!    [`static_bfs_chunk_with_dynamic_capture`].  Dynamic `import()` targets
//!    discovered during BFS are pushed onto a `lazy_queue` (deduped by
//!    `lazy_seen`).
//! 3. Use a `module_index` map with first-seen-wins semantics so that a module
//!    claimed by an earlier route is not re-assigned to a later one.
//! 4. Emit a [`Chunk`] with [`LoadCondition::Initial`] and a placeholder
//!    content hash (`ContentHash("")`); real hashing is performed in a later
//!    pass.
//!
//! After all entry routes are processed, drain the `lazy_queue`:
//! * Skip if the root is already owned by an earlier chunk (first-seen-wins).
//! * Otherwise create a `lazy_N` chunk by running the same BFS helper,
//!   filtering out modules already in `module_index`, and pushing a
//!   [`LoadCondition::Lazy`] chunk.  Nested dynamic imports discovered during
//!   the lazy BFS are also captured into the queue.

use std::collections::{HashMap, HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash, ImportKind};

use crate::types::{Chunk, ChunkId, LoadCondition};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Assign modules to initial **and** lazy chunks.
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
/// * `Vec<Chunk>` — initial chunks (one per alive entry route, sorted) followed
///   by lazy chunks in BFS-discovery order.
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

    // Lazy chunk bookkeeping.
    let mut lazy_seen: HashSet<ContentHash> = HashSet::new();
    let mut lazy_queue: VecDeque<(String, ContentHash)> = VecDeque::new();
    let mut lazy_counter: usize = 0;

    // -----------------------------------------------------------------------
    // One INITIAL chunk per alive entry route.
    // -----------------------------------------------------------------------

    for route in routes {
        // Unwrap is safe: `route` came from `entry_hashes.keys()`.
        let entry_hash = entry_hashes.get(route).expect("route key must exist");

        // Skip routes whose entry module is not alive.
        if !alive.contains(entry_hash) {
            continue;
        }

        let chunk_id = format!("initial_{}", chunk_id_slug(route));

        // BFS over static imports only; dynamic import targets captured into queue.
        let bfs_modules = static_bfs_chunk_with_dynamic_capture(
            entry_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );

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

    // -----------------------------------------------------------------------
    // Drain the lazy queue to create LAZY chunks.
    //
    // Note: `static_bfs_chunk_with_dynamic_capture` may push new entries into
    // `lazy_queue` during BFS of a lazy root (nested dynamic imports), so the
    // loop continues until the queue is fully drained.
    // -----------------------------------------------------------------------

    while let Some((_, root_hash)) = lazy_queue.pop_front() {
        // Skip if the root is already owned by an earlier chunk.
        if module_index.contains_key(&root_hash) {
            continue;
        }

        lazy_counter += 1;
        let chunk_id = format!("lazy_{lazy_counter}");

        // BFS from the lazy root; nested dynamic imports are captured into the
        // queue for processing in subsequent iterations.
        let bfs_modules = static_bfs_chunk_with_dynamic_capture(
            &root_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );

        // Filter out modules already claimed by an earlier chunk.
        let chunk_modules: Vec<ContentHash> = bfs_modules
            .into_iter()
            .filter(|m| !module_index.contains_key(m))
            .collect();

        // Skip if all reachable modules are already owned elsewhere.
        if chunk_modules.is_empty() {
            continue;
        }

        // Claim the modules for this lazy chunk.
        for m in &chunk_modules {
            module_index.insert(m.clone(), chunk_id.clone());
        }

        chunks.push(Chunk {
            id: chunk_id,
            modules: chunk_modules,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Lazy,
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
/// Dynamic `import()` targets encountered during the traversal are captured
/// into `lazy_queue` (deduped via `lazy_seen`) instead of being traversed.
/// This keeps the returned module list limited to the static-import reachable
/// set, while ensuring every dynamic boundary is queued for lazy processing.
///
/// Returns a `Vec<ContentHash>` of all modules reachable via non-dynamic
/// import edges whose targets are in the `alive` set.  `root` itself is
/// included as the first element if it is alive.
///
/// # When there are no dynamic imports
///
/// The function behaves identically to the previous `static_bfs_chunk`
/// helper — `lazy_queue` remains unchanged and the return value is the pure
/// static-reachable set.
fn static_bfs_chunk_with_dynamic_capture<'a>(
    root: &ContentHash,
    by_hash: &HashMap<ContentHash, &'a BundleGraphNode>,
    path_to_hash: &HashMap<&str, &'a ContentHash>,
    alive: &HashSet<ContentHash>,
    lazy_seen: &mut HashSet<ContentHash>,
    lazy_queue: &mut VecDeque<(String, ContentHash)>,
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
            if import.kind == ImportKind::Dynamic {
                // Capture dynamic import target into the lazy queue (deduped).
                let target_hash = match path_to_hash.get(import.specifier.as_str()) {
                    Some(h) => *h,
                    None => continue, // Unresolved specifier — skip.
                };

                if alive.contains(target_hash) && lazy_seen.insert(target_hash.clone()) {
                    lazy_queue.push_back((import.specifier.clone(), target_hash.clone()));
                }

                // Do NOT traverse into the dynamic target from this chunk.
                continue;
            }

            // Static import: resolve and follow.
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
