//! C³ — Conditional Co-request Clustering.
//!
//! Computes bidirectional merge suggestions for chunk pairs whose
//! conditional co-request probability exceeds a configurable threshold.

use std::collections::{HashMap, HashSet};

use crate::store::PgoStore;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Tuning knobs for the C³ algorithm.
#[derive(Debug, Clone)]
pub struct C3Config {
    /// P(b|a) (and P(a|b)) must both exceed this value for a merge suggestion.
    /// Default: `0.70`.
    pub merge_threshold: f64,
    /// Skip a merge if the combined module count would exceed this.
    /// Default: `500`.
    pub max_merge_modules: usize,
    /// Skip clustering entirely if the store has fewer sessions than this.
    /// Default: `100`.
    pub min_sessions: usize,
}

impl Default for C3Config {
    fn default() -> Self {
        C3Config {
            merge_threshold: 0.70,
            max_merge_modules: 500,
            min_sessions: 100,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute bidirectional merge suggestions for `chunk_ids`.
///
/// Returns a map from each chunk ID to its suggested merge target (or `None`
/// if no merge candidate was identified).
///
/// # Algorithm
///
/// 1. Return all-None immediately when `session_count < config.min_sessions`.
/// 2. For every pair `(a, b)` (sorted so `a < b` for determinism), compute
///    `P(b|a)` and `P(a|b)` using the store's co-request and load counts.
/// 3. Collect pairs where **both** conditional probabilities exceed the
///    threshold.
/// 4. Sort candidates by `P(b|a) + P(a|b)` descending (alphabetical pair as
///    tiebreaker).
/// 5. Greedy assignment: for each candidate pair, skip if either chunk is
///    already assigned, or if the combined module count would exceed
///    `max_merge_modules`.  Otherwise assign both chunks to each other
///    (bidirectional).
pub fn compute_clusters(
    store: &PgoStore,
    chunk_ids: &[String],
    config: &C3Config,
) -> anyhow::Result<HashMap<String, Option<String>>> {
    // Initialise result with None for every supplied chunk.
    let mut result: HashMap<String, Option<String>> =
        chunk_ids.iter().map(|id| (id.clone(), None)).collect();

    // Gate: not enough data yet.
    if store.session_count()? < config.min_sessions {
        return Ok(result);
    }

    // -----------------------------------------------------------------------
    // Build candidate list.
    // -----------------------------------------------------------------------
    // Each entry: (combined_score, a, b) where a < b alphabetically.
    let mut candidates: Vec<(f64, String, String)> = Vec::new();

    let n = chunk_ids.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let raw_a = &chunk_ids[i];
            let raw_b = &chunk_ids[j];

            // Canonical ordering: a < b.
            let (a, b) = if raw_a <= raw_b {
                (raw_a, raw_b)
            } else {
                (raw_b, raw_a)
            };

            let co_count = store.co_load_count(a, b)?;
            let a_count = store.chunk_load_count(a)?;
            let b_count = store.chunk_load_count(b)?;

            if a_count == 0 || b_count == 0 {
                continue;
            }

            let p_ab = co_count as f64 / a_count as f64; // P(b|a)
            let p_ba = co_count as f64 / b_count as f64; // P(a|b)

            if p_ab > config.merge_threshold && p_ba > config.merge_threshold {
                candidates.push((p_ab + p_ba, a.clone(), b.clone()));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Sort by score descending; alphabetical pair as deterministic tiebreaker.
    // -----------------------------------------------------------------------
    candidates.sort_by(|x, y| {
        y.0.partial_cmp(&x.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.1.cmp(&y.1))
            .then_with(|| x.2.cmp(&y.2))
    });

    // -----------------------------------------------------------------------
    // Greedy assignment with naive module-count tracking (1 per chunk).
    // -----------------------------------------------------------------------
    let mut module_counts: HashMap<&str, usize> =
        chunk_ids.iter().map(|id| (id.as_str(), 1)).collect();
    let mut assigned: HashSet<String> = HashSet::new();

    for (_, a, b) in &candidates {
        if assigned.contains(a.as_str()) || assigned.contains(b.as_str()) {
            continue;
        }
        let combined = module_counts.get(a.as_str()).copied().unwrap_or(1)
            + module_counts.get(b.as_str()).copied().unwrap_or(1);
        if combined > config.max_merge_modules {
            continue;
        }

        // Bidirectional assignment: a → b, b → a.
        result.insert(a.clone(), Some(b.clone()));
        result.insert(b.clone(), Some(a.clone()));
        assigned.insert(a.clone());
        assigned.insert(b.clone());

        // Update combined module count so later pairs respect the limit.
        module_counts.insert(a.as_str(), combined);
        module_counts.insert(b.as_str(), combined);
    }

    Ok(result)
}
