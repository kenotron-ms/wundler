//! BFS reachability analysis over the bundle dependency graph.
//!
//! This module computes the set of modules that are transitively reachable
//! from one or more entry-point hashes, following both static and dynamic
//! imports (both are already encoded as edges by [`crate::graph::build_adjacency`]).

use std::collections::{HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash};

use crate::graph::build_adjacency;

/// Compute the set of all modules reachable from the given entry hashes.
///
/// # Algorithm
///
/// 1. Build an adjacency map from `nodes` via [`build_adjacency`].
/// 2. Seed the alive set and BFS queue with every entry hash that corresponds
///    to a known node.  Entry hashes with no outgoing imports are added to
///    `alive` but produce no further expansion (they are "alive alone").
/// 3. Standard BFS: for each node popped from the queue, all neighbours not
///    yet in `alive` are inserted and enqueued.
///
/// # Edge cases
///
/// * Empty `entry_hashes` → returns an empty `HashSet`.
/// * Entry hash with no imports → alive set contains only that entry.
/// * Cycles in the graph are handled correctly by the `alive` guard.
pub fn compute_reachability(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
) -> HashSet<ContentHash> {
    if entry_hashes.is_empty() {
        return HashSet::new();
    }

    let adj = build_adjacency(nodes);

    let mut alive: HashSet<ContentHash> = HashSet::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    // Seed: add each entry that exists in the node slice.
    // A node not present in `adj` has no outgoing edges but is still alive.
    let known: HashSet<&ContentHash> = nodes.iter().map(|n| &n.id).collect();
    for entry in entry_hashes {
        if known.contains(entry) && alive.insert(entry.clone()) {
            queue.push_back(entry.clone());
        }
    }

    // BFS — the `alive` guard prevents re-visiting and handles cycles.
    while let Some(current) = queue.pop_front() {
        if let Some(targets) = adj.get(&current) {
            for target in targets {
                if alive.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }
        }
    }

    alive
}
