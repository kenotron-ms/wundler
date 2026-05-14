//! Output writer for the Wundler build pipeline.
//!
//! Provides three public functions:
//!
//! * [`write_chunk`]      — writes a `ChunkOutput` to `<out_dir>/chunks/<hash>.js`
//!                          (and an optional `.js.map` alongside it).
//! * [`write_manifest`]   — serialises a `ChunkManifest` as pretty JSON to
//!                          `<out_dir>/manifest.json`, with chunk hashes
//!                          updated to the actual output-file hashes.
//! * [`write_index_html`] — generates an `<out_dir>/index.html` stub that
//!                          loads the initial chunks for a named entry point,
//!                          referencing the actual output-file hashes.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use wundler_core::types::ContentHash;
use wundler_graph::types::ChunkManifest;
use wundler_transform::engine::ChunkOutput;

// ---------------------------------------------------------------------------
// write_chunk
// ---------------------------------------------------------------------------

/// Write a single chunk's JavaScript (and optional source map) to disk.
///
/// The output file is placed at `<out_dir>/chunks/<hash>.js`, where `hash` is
/// the hex string of `output.hash`.  If `output.source_map` is `Some(_)`, a
/// companion `<hash>.js.map` file is also written and a `//# sourceMappingURL=`
/// comment is appended to the JS body.
///
/// Returns the path of the created `.js` file.
pub fn write_chunk(out_dir: &Path, output: &ChunkOutput) -> Result<PathBuf> {
    let chunks_dir = out_dir.join("chunks");
    fs::create_dir_all(&chunks_dir)?;

    let hash_hex = output.hash.as_str();
    let file_name = format!("{hash_hex}.js");
    let path = chunks_dir.join(&file_name);

    let mut body = output.code.clone();

    if let Some(ref map) = output.source_map {
        // Ensure the comment is on its own line.
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!("//# sourceMappingURL={hash_hex}.js.map\n"));

        // Write the companion source-map file.
        let map_path = chunks_dir.join(format!("{hash_hex}.js.map"));
        fs::write(map_path, map)?;
    }

    fs::write(&path, &body)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// write_manifest
// ---------------------------------------------------------------------------

/// Serialise `manifest` as pretty-printed JSON and write it to
/// `<out_dir>/manifest.json`.
///
/// `id_to_hash` maps each chunk's logical ID to the content hash of its
/// actual output file (the hash used in the `.js` filename).  The manifest
/// is cloned and each chunk's `hash` field is overwritten with the
/// corresponding output hash before serialisation, so the on-disk
/// `manifest.json` always reflects real file names.
pub fn write_manifest(
    out_dir: &Path,
    manifest: &ChunkManifest,
    id_to_hash: &HashMap<String, ContentHash>,
) -> Result<()> {
    fs::create_dir_all(out_dir)?;

    // Clone the manifest and update each chunk hash to the actual output hash.
    let mut updated = manifest.clone();
    for chunk in &mut updated.chunks {
        if let Some(output_hash) = id_to_hash.get(&chunk.id) {
            chunk.hash = output_hash.clone();
        }
    }

    let json = serde_json::to_string_pretty(&updated)?;
    fs::write(out_dir.join("manifest.json"), json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// write_index_html
// ---------------------------------------------------------------------------

/// Generate an `index.html` stub for the named `entry` point.
///
/// The stub includes one `<script type="module">` tag per chunk listed in
/// `manifest.entry_chunks[entry]`, with `src` pointing to
/// `chunks/<hash>.js`.
///
/// `id_to_hash` maps each chunk's logical ID to the content hash of its
/// actual output file.  This hash — not the analysis-phase hash stored in
/// `chunk.hash` — is used for the `src` attribute, ensuring the referenced
/// filename matches the file written by [`write_chunk`].
///
/// The file is written to `<out_dir>/index.html`.
pub fn write_index_html(
    out_dir: &Path,
    manifest: &ChunkManifest,
    entry: &str,
    id_to_hash: &HashMap<String, ContentHash>,
) -> Result<()> {
    fs::create_dir_all(out_dir)?;

    // Collect script tags for each initial chunk of this entry.
    let mut script_tags = String::new();

    if let Some(chunk_ids) = manifest.entry_chunks.get(entry) {
        for chunk_id in chunk_ids {
            // Look up the actual output hash for this chunk (not the
            // analysis-phase hash stored in the manifest).
            if let Some(hash) = id_to_hash.get(chunk_id) {
                let hash_hex = hash.as_str();
                script_tags.push_str(&format!(
                    "  <script src=\"chunks/{hash_hex}.js\" type=\"module\"></script>\n"
                ));
            }
        }
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{entry}</title>
</head>
<body>
  <div id="root"></div>
{script_tags}</body>
</html>
"#
    );

    fs::write(out_dir.join("index.html"), html)?;
    Ok(())
}
