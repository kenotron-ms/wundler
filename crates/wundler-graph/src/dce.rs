//! Call-edge transitive Dead Code Elimination (DCE) for the bundle graph.
//!
//! This module computes, for each alive module, the set of its Named exports
//! that are *dead* — i.e. not reachable from any public root via the module's
//! declared `call_edges`.
//!
//! # Algorithm
//!
//! 1. **Index** nodes by both [`ContentHash`] and path.
//! 2. **Seed** `live_exports` with every [`ExportKind::Default`] and
//!    [`ExportKind::StarExport`] ("Namespace") export of every alive module —
//!    these are always alive as long as their module is alive.
//! 3. **Transitive walk** via `call_edges`: for each `(mod_hash, export_name)`
//!    pair dequeued, iterate `node.summary.call_edges`; for every edge where
//!    `edge.caller == export_name`, resolve `edge.callee`:
//!    * If `callee` contains `"::"`, split into `(module_path, export_name)`
//!      and look up the module by path.
//!    * Otherwise the callee lives in the same module.
//!      Skip if the target module is not in `alive`.  Add to `live_exports`
//!      (and re-enqueue) if the pair is new.
//! 4. **Compute dead**: for each alive module, a [`ExportKind::Named`] export
//!    is dead if its `(module_hash, name)` pair is absent from `live_exports`.
//!    [`ExportKind::Default`] and [`ExportKind::StarExport`] exports are
//!    *never* reported as dead.
//!
//! Only alive modules appear as keys in the returned map.

use std::collections::{HashMap, HashSet, VecDeque};

use wundler_core::types::{BundleGraphNode, ContentHash, ExportKind};

/// Compute the set of dead Named exports for each alive module.
///
/// # Parameters
///
/// * `nodes`  — the full set of bundle graph nodes (alive and dead).
/// * `alive`  — the set of module hashes that are considered reachable /
///   alive by a prior reachability pass.
///
/// # Returns
///
/// A map from alive-module hash → set of dead export names.  The set contains
/// only [`ExportKind::Named`] exports that are not transitively reachable from
/// any public root.  [`ExportKind::Default`] and [`ExportKind::StarExport`]
/// exports are omitted (they are never dead while their module is alive).
///
/// All alive modules appear as keys; the set is empty for modules that have no
/// dead exports.
pub fn compute_dead_exports(
    nodes: &[BundleGraphNode],
    alive: &HashSet<ContentHash>,
) -> HashMap<ContentHash, HashSet<String>> {
    // -----------------------------------------------------------------------
    // Step 1 — build indexes.
    // -----------------------------------------------------------------------

    // Hash → node reference (for call-edge walks on alive modules).
    let hash_to_node: HashMap<ContentHash, &BundleGraphNode> =
        nodes.iter().map(|n| (n.id.clone(), n)).collect();

    // Path → hash (for resolving cross-module `"path::export"` callees).
    let path_to_hash: HashMap<&str, ContentHash> = nodes
        .iter()
        .map(|n| (n.path.as_str(), n.id.clone()))
        .collect();

    // -----------------------------------------------------------------------
    // Step 2 — seed live_exports with public roots.
    //
    // Default and StarExport ("Namespace") exports are always alive while their
    // module is alive.  They are the starting points for the transitive walk.
    // -----------------------------------------------------------------------

    // A `(module_hash, export_name)` pair is *live* if it is reachable from a
    // public root.
    let mut live_exports: HashSet<(ContentHash, String)> = HashSet::new();
    let mut queue: VecDeque<(ContentHash, String)> = VecDeque::new();

    for node in nodes {
        if !alive.contains(&node.id) {
            continue;
        }
        for export in &node.summary.exports {
            match export.kind {
                ExportKind::Default | ExportKind::StarExport => {
                    let pair = (node.id.clone(), export.name.clone());
                    if live_exports.insert(pair.clone()) {
                        queue.push_back(pair);
                    }
                }
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Step 3 — transitive walk via call edges.
    // -----------------------------------------------------------------------

    while let Some((mod_hash, export_name)) = queue.pop_front() {
        let node = match hash_to_node.get(&mod_hash) {
            Some(n) => n,
            None => continue,
        };

        for edge in &node.summary.call_edges {
            // Only follow edges *from* the export we are currently expanding.
            if edge.caller != export_name {
                continue;
            }

            // Resolve the callee to a (target_hash, target_export_name) pair.
            let (target_hash, target_export) = if edge.callee.contains("::") {
                // Cross-module call: "<module_path>::<export_name>"
                let (module_path, target_exp) = edge
                    .callee
                    .split_once("::")
                    .expect("contains '::' so split_once always succeeds");
                match path_to_hash.get(module_path) {
                    Some(hash) => (hash.clone(), target_exp.to_string()),
                    None => continue, // Unresolved module — skip.
                }
            } else {
                // Same-module call: "<export_name>"
                (mod_hash.clone(), edge.callee.clone())
            };

            // Skip calls into dead modules.
            if !alive.contains(&target_hash) {
                continue;
            }

            let pair = (target_hash, target_export);
            if live_exports.insert(pair.clone()) {
                queue.push_back(pair);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Step 4 — compute the dead export set for every alive module.
    // -----------------------------------------------------------------------

    let mut result: HashMap<ContentHash, HashSet<String>> = HashMap::new();

    for node in nodes {
        if !alive.contains(&node.id) {
            continue; // Only alive modules appear as keys.
        }

        let mut dead_set: HashSet<String> = HashSet::new();

        for export in &node.summary.exports {
            if export.kind == ExportKind::Named {
                // A Named export is dead if it never entered live_exports.
                if !live_exports.contains(&(node.id.clone(), export.name.clone())) {
                    dead_set.insert(export.name.clone());
                }
            }
            // Default / StarExport: never reported as dead.
        }

        result.insert(node.id.clone(), dead_set);
    }

    result
}
