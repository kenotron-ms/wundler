//! Compute [`PgoHints`] from a populated store for a given manifest build.

use std::collections::HashMap;

use crate::clustering::{compute_clusters, C3Config};
use crate::store::PgoStore;
use crate::types::{ChunkHint, PgoHints};

/// Compute hints for every chunk in `chunk_ids`, backed by `store`.
///
/// * `co_request_score` = fraction of sessions that loaded the chunk within
///   the first three positions (`load_order < 3`).  Zero when the store is
///   empty.
/// * `median_load_order` = median position in the load waterfall.  Zero when
///   the chunk has never been seen.
/// * `suggested_merge` = result of [`compute_clusters`] for this manifest.
pub fn compute_hints(
    store: &PgoStore,
    manifest_build_id: &str,
    chunk_ids: &[String],
    cluster_config: &C3Config,
) -> anyhow::Result<PgoHints> {
    let session_count = store.session_count()?;
    let clusters = compute_clusters(store, chunk_ids, cluster_config)?;

    let mut chunk_hints: HashMap<String, ChunkHint> = HashMap::new();

    for chunk_id in chunk_ids {
        let co_request_score = if session_count == 0 {
            0.0
        } else {
            store.initial_load_count(chunk_id, 3)? as f64 / session_count as f64
        };

        let median_load_order = store.median_load_order(chunk_id)?;
        let suggested_merge = clusters.get(chunk_id).and_then(|v| v.clone());

        chunk_hints.insert(
            chunk_id.clone(),
            ChunkHint {
                co_request_score,
                median_load_order,
                suggested_merge,
            },
        );
    }

    Ok(PgoHints {
        build_id: manifest_build_id.to_string(),
        chunk_hints,
    })
}
