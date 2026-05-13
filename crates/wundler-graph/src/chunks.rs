//! Chunk assignment for the Wundler bundle graph.
//!
//! This module assigns modules to chunks:
//!
//! * **Initial chunks** — one per entry route, loaded on first page render.
//! * **Lazy chunks** — one per `import()` boundary, fetched on demand.
//! * **Commons chunk** — shared modules extracted when they appear in ≥
//!   `commons_threshold` candidate chunks, emitted first with id `"commons"`.
//!
//! # Algorithm (3-phase)
//!
//! ## Phase 1 — Collect candidates (WITHOUT de-duplication)
//!
//! For each entry route (sorted for determinism):
//! 1. Derive a `chunk_id` from the route name via [`chunk_id_slug`].
//! 2. Run a static-import-only BFS from the entry module via
//!    [`static_bfs_chunk_with_dynamic_capture`].  Dynamic `import()` targets
//!    discovered during BFS are pushed onto a `lazy_queue` (deduped by
//!    `lazy_seen`).
//! 3. Emit one [`Candidate`] per alive route with the **full** BFS result
//!    (no first-seen-wins at this stage — shared modules may appear in
//!    multiple candidates).
//!
//! After all entry routes are processed, drain the `lazy_queue`:
//! * Skip if the BFS result is empty (empty-skip).
//! * Otherwise create a `lazy_N` [`Candidate`].  Nested dynamic imports
//!   discovered during the lazy BFS are also captured into the queue.
//!
//! ## Phase 2 — Count appearances
//!
//! Build `appearance: HashMap<ContentHash, usize>` counting in how many
//! candidate chunks each module appears.  A per-candidate `seen_in_this`
//! `HashSet` prevents double-counting the same module within one candidate.
//!
//! ## Phase 3 — Emit chunks
//!
//! * `commons_set` = modules with `appearance` count ≥ `commons_threshold`.
//! * Emit commons chunk **first** (id `"commons"`, `LoadCondition::Initial`,
//!   members sorted by `ContentHash.0` lexicographically, placeholder hash).
//! * Emit each per-route / lazy candidate with commons members filtered out
//!   and any remaining duplicates resolved by first-seen-wins
//!   (`if module_index.contains_key(&m) { continue }`).
//! * Skip candidates that become empty after filtering.

use std::collections::{HashMap, HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash, ImportKind};

use crate::types::{Chunk, ChunkId, LoadCondition};

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A pre-deduplication snapshot of the modules reachable from one route or
/// lazy-import root.  Used in Phases 1 and 2 before commons extraction.
struct Candidate {
    id: String,
    load_condition: LoadCondition,
    /// Full BFS result — may overlap with other candidates (intentional).
    modules: Vec<ContentHash>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Assign modules to initial, lazy, and commons chunks.
///
/// # Parameters
///
/// * `nodes` — complete set of bundle graph nodes.
/// * `alive` — set of module hashes that survived the reachability pass.
/// * `entry_hashes` — map from route name (e.g. `"main"`) to its entry
///   module `ContentHash`.
/// * `commons_threshold` — minimum number of candidate chunks a module must
///   appear in to be hoisted into the commons chunk.  Pass `usize::MAX` (or
///   any value larger than the number of routes) to disable commons
///   extraction.
///
/// # Returns
///
/// A tuple of:
/// * `Vec<Chunk>` — commons chunk (if any) first, then initial chunks (one
///   per alive entry route, sorted) followed by lazy chunks in
///   BFS-discovery order.
/// * `HashMap<ContentHash, ChunkId>` — reverse index mapping each assigned
///   module hash to the chunk it belongs to.
pub fn assign_chunks(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
    entry_hashes: &HashMap<String, ContentHash>,
    commons_threshold: usize,
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

    // Lazy chunk bookkeeping (shared across all BFS passes).
    let mut lazy_seen: HashSet<ContentHash> = HashSet::new();
    let mut lazy_queue: VecDeque<(String, ContentHash)> = VecDeque::new();
    let mut lazy_counter: usize = 0;

    // -----------------------------------------------------------------------
    // Phase 1 — Collect candidates WITHOUT de-duplication.
    // -----------------------------------------------------------------------

    let mut candidates: Vec<Candidate> = Vec::new();

    // One INITIAL candidate per alive entry route.
    for route in routes {
        // Unwrap is safe: `route` came from `entry_hashes.keys()`.
        let entry_hash = entry_hashes.get(route).expect("route key must exist");

        // Skip routes whose entry module is not alive.
        if !alive.contains(entry_hash) {
            continue;
        }

        let chunk_id = format!("initial_{}", chunk_id_slug(route));

        // BFS over static imports only; dynamic import targets captured into queue.
        // No first-seen-wins filtering — full BFS result goes into the candidate.
        let bfs_modules = static_bfs_chunk_with_dynamic_capture(
            entry_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );

        candidates.push(Candidate {
            id: chunk_id,
            load_condition: LoadCondition::Initial,
            modules: bfs_modules,
        });
    }

    // Drain the lazy queue to create LAZY candidates.
    //
    // Note: `static_bfs_chunk_with_dynamic_capture` may push new entries into
    // `lazy_queue` during BFS of a lazy root (nested dynamic imports), so the
    // loop continues until the queue is fully drained.
    while let Some((_, root_hash)) = lazy_queue.pop_front() {
        // BFS from the lazy root; nested dynamic imports captured into the queue.
        let bfs_modules = static_bfs_chunk_with_dynamic_capture(
            &root_hash,
            &by_hash,
            &path_to_hash,
            alive,
            &mut lazy_seen,
            &mut lazy_queue,
        );

        // Empty-skip: if the root is not alive or has no alive descendants,
        // discard this queue entry without creating a candidate.
        if bfs_modules.is_empty() {
            continue;
        }

        lazy_counter += 1;
        let chunk_id = format!("lazy_{lazy_counter}");

        candidates.push(Candidate {
            id: chunk_id,
            load_condition: LoadCondition::Lazy,
            modules: bfs_modules,
        });
    }

    // -----------------------------------------------------------------------
    // Phase 2 — Count appearances.
    //
    // `appearance[m]` = number of candidate chunks that contain module `m`.
    // A per-candidate `seen_in_this` HashSet prevents double-counting if the
    // same module appears more than once in a single candidate's BFS result.
    // -----------------------------------------------------------------------

    let mut appearance: HashMap<ContentHash, usize> = HashMap::new();
    for candidate in &candidates {
        let mut seen_in_this: HashSet<ContentHash> = HashSet::new();
        for m in &candidate.modules {
            if seen_in_this.insert(m.clone()) {
                *appearance.entry(m.clone()).or_insert(0) += 1;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Phase 3 — Emit chunks.
    // -----------------------------------------------------------------------

    // Modules that appear in ≥ commons_threshold candidates are hoisted to commons.
    let commons_set: HashSet<ContentHash> = appearance
        .iter()
        .filter(|(_, &count)| count >= commons_threshold)
        .map(|(h, _)| h.clone())
        .collect();

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut module_index: HashMap<ContentHash, ChunkId> = HashMap::new();

    // Emit the commons chunk FIRST (deterministic id "commons").
    if !commons_set.is_empty() {
        // Sort members by ContentHash.0 for determinism.
        let mut commons_modules: Vec<ContentHash> = commons_set.iter().cloned().collect();
        commons_modules.sort_by(|a, b| a.0.cmp(&b.0));

        // Claim all commons modules in module_index before per-route processing.
        for m in &commons_modules {
            module_index.insert(m.clone(), "commons".to_string());
        }

        chunks.push(Chunk {
            id: "commons".to_string(),
            modules: commons_modules,
            hash: ContentHash("".to_string()),
            load_condition: LoadCondition::Initial,
            co_request_score: None,
            median_load_order: None,
            suggested_merge: None,
        });
    }

    // Emit per-route and lazy chunks with commons filtered out and
    // first-seen-wins applied for any remaining cross-chunk duplicates.
    for candidate in candidates {
        let mut chunk_modules: Vec<ContentHash> = Vec::new();
        for m in candidate.modules {
            // Commons members are already in module_index; skip them.
            if commons_set.contains(&m) {
                continue;
            }
            // First-seen-wins: skip if already claimed by an earlier chunk.
            if module_index.contains_key(&m) {
                continue;
            }
            // Claim this module for the current chunk.
            module_index.insert(m.clone(), candidate.id.clone());
            chunk_modules.push(m);
        }

        // Skip chunks that become empty after commons filtering.
        if chunk_modules.is_empty() {
            continue;
        }

        chunks.push(Chunk {
            id: candidate.id,
            modules: chunk_modules,
            hash: ContentHash("".to_string()),
            load_condition: candidate.load_condition,
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
