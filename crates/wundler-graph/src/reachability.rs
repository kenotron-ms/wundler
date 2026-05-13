//! BFS reachability analysis over the bundle dependency graph.
//!
//! This module computes the set of modules that are transitively reachable
//! from one or more entry-point hashes, following both static and dynamic
//! imports (both are already encoded as edges by [`crate::graph::build_adjacency`]).

use std::collections::{HashMap, HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash};

use crate::graph::{build_adjacency, tarjan_sccs};

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

/// Compute the set of all modules reachable from the given entry hashes,
/// treating strongly-connected components as atomic units.
///
/// # Algorithm
///
/// 1. Build an adjacency map from `nodes` via [`build_adjacency`].
/// 2. Compute all SCCs via [`tarjan_sccs`].
/// 3. Build `scc_of: HashMap<ContentHash, usize>` mapping each node to its
///    SCC index.
/// 4. Seed the BFS from every entry hash that corresponds to a known node,
///    using [`activate_scc`] so that all SCC members are activated together.
/// 5. Standard BFS: for each node dequeued, look up each neighbour's SCC
///    index and call [`activate_scc`]; the `alive_scc` guard prevents
///    re-processing an SCC that is already fully enqueued.
///
/// # SCC liveness invariant
///
/// * If **any** member of an SCC is reached, **every** member becomes alive.
/// * If an SCC is never reached, all its members remain dead.
/// * On a graph with no cycles (all SCCs are singletons) the result is
///   identical to [`compute_reachability`].
pub fn compute_reachability_with_sccs(
    nodes: &[BundleGraphNode],
    entry_hashes: &HashSet<ContentHash>,
) -> HashSet<ContentHash> {
    if entry_hashes.is_empty() {
        return HashSet::new();
    }

    // Step 1 — build the edge map.
    let adj = build_adjacency(nodes);

    // Step 2 — detect SCCs; each element of `sccs` is one component.
    let sccs = tarjan_sccs(nodes, &adj);

    // Step 3 — build scc_of: ContentHash → SCC index.
    let mut scc_of: HashMap<ContentHash, usize> = HashMap::new();
    for (idx, component) in sccs.iter().enumerate() {
        for hash in component {
            scc_of.insert(hash.clone(), idx);
        }
    }

    // Step 4 — BFS state.
    let mut alive: HashSet<ContentHash> = HashSet::new();
    let mut alive_scc: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<ContentHash> = VecDeque::new();

    // Seed: activate the SCC of every entry that is present in the graph.
    let known: HashSet<&ContentHash> = nodes.iter().map(|n| &n.id).collect();
    for entry in entry_hashes {
        if known.contains(entry) {
            if let Some(&scc_idx) = scc_of.get(entry) {
                activate_scc(scc_idx, &sccs, &mut alive, &mut queue, &mut alive_scc);
            }
        }
    }

    // Step 5 — BFS; the alive_scc guard prevents re-expanding SCCs.
    while let Some(current) = queue.pop_front() {
        if let Some(targets) = adj.get(&current) {
            for target in targets {
                if let Some(&scc_idx) = scc_of.get(target) {
                    activate_scc(scc_idx, &sccs, &mut alive, &mut queue, &mut alive_scc);
                }
            }
        }
    }

    alive
}

/// Activate every member of SCC `scc_idx` if it has not been activated before.
///
/// On the first call for a given `scc_idx`:
/// * Sets `alive_scc.insert(scc_idx)` to mark the SCC as processed.
/// * For every member of that SCC: inserts into `alive` and enqueues it if it
///   was not already present (the `alive.insert` guard handles idempotency
///   within a single SCC as well as across SCC boundaries).
///
/// Subsequent calls with the same `scc_idx` are no-ops.
fn activate_scc(
    scc_idx: usize,
    scc_members: &[Vec<ContentHash>],
    alive: &mut HashSet<ContentHash>,
    queue: &mut VecDeque<ContentHash>,
    alive_scc: &mut HashSet<usize>,
) {
    if alive_scc.insert(scc_idx) {
        for member in &scc_members[scc_idx] {
            if alive.insert(member.clone()) {
                queue.push_back(member.clone());
            }
        }
    }
}
