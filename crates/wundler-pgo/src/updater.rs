//! Apply [`PgoHints`] back into a [`ChunkManifest`] on disk.
//!
//! Writes are atomic: the manifest is serialised to a temporary file
//! `{manifest_path}.tmp` and then renamed into place.

use std::path::Path;

use anyhow::{Context, Result};
use wundler_graph::types::ChunkManifest;

use crate::types::PgoHints;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary of a single `apply_hints` run.
#[derive(Debug)]
pub struct UpdateStats {
    /// Number of chunks whose PGO fields were updated.
    pub chunks_updated: usize,
    /// Number of chunks that received a non-None `suggested_merge`.
    pub merge_suggestions: usize,
    /// The `build_id` of the manifest that was updated.
    pub manifest_build_id: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read `manifest_path`, apply `hints`, and write back atomically.
///
/// # Errors
///
/// Returns an error if:
/// * The manifest file cannot be read or parsed.
/// * `manifest.build_id != hints.build_id`.
/// * The atomic write fails.
pub fn apply_hints(manifest_path: &Path, hints: &PgoHints) -> Result<UpdateStats> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read manifest at {:?}", manifest_path))?;

    let mut manifest = ChunkManifest::from_json(&content)
        .with_context(|| format!("failed to parse manifest JSON at {:?}", manifest_path))?;

    if manifest.build_id != hints.build_id {
        anyhow::bail!(
            "build_id mismatch: manifest has {:?} but hints have {:?}",
            manifest.build_id,
            hints.build_id
        );
    }

    let mut chunks_updated = 0usize;
    let mut merge_suggestions = 0usize;

    for chunk in &mut manifest.chunks {
        if let Some(hint) = hints.chunk_hints.get(&chunk.id) {
            chunk.co_request_score = Some(hint.co_request_score);
            chunk.median_load_order = Some(hint.median_load_order);
            chunk.suggested_merge = hint.suggested_merge.clone();
            chunks_updated += 1;
            if hint.suggested_merge.is_some() {
                merge_suggestions += 1;
            }
        }
    }

    // Atomic write: serialise to .tmp then rename.
    let tmp_path = {
        let mut name = manifest_path
            .file_name()
            .expect("manifest_path must be a file")
            .to_owned();
        name.push(".tmp");
        manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(name)
    };

    let json = manifest
        .to_json()
        .context("failed to serialise updated manifest")?;

    std::fs::write(&tmp_path, &json)
        .with_context(|| format!("failed to write temp manifest to {:?}", tmp_path))?;

    std::fs::rename(&tmp_path, manifest_path).with_context(|| {
        format!(
            "failed to rename {:?} → {:?}",
            tmp_path, manifest_path
        )
    })?;

    Ok(UpdateStats {
        chunks_updated,
        merge_suggestions,
        manifest_build_id: hints.build_id.clone(),
    })
}
