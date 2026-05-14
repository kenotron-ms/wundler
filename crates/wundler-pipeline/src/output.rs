//! Output writer for the Wundler build pipeline.
//!
//! Provides three public functions:
//!
//! * [`write_chunk`]      — writes a `ChunkOutput` to `<out_dir>/chunks/<hash>.js`
//!                          (and an optional `.js.map` alongside it).
//! * [`write_manifest`]   — serialises a `ChunkManifest` as pretty JSON to
//!                          `<out_dir>/manifest.json`.
//! * [`write_index_html`] — generates an `<out_dir>/index.html` stub that
//!                          loads the initial chunks for a named entry point.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
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
pub fn write_manifest(out_dir: &Path, manifest: &ChunkManifest) -> Result<()> {
    fs::create_dir_all(out_dir)?;
    let json = serde_json::to_string_pretty(manifest)?;
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
/// The file is written to `<out_dir>/index.html`.
pub fn write_index_html(out_dir: &Path, manifest: &ChunkManifest, entry: &str) -> Result<()> {
    fs::create_dir_all(out_dir)?;

    // Collect script tags for each initial chunk of this entry.
    let mut script_tags = String::new();

    if let Some(chunk_ids) = manifest.entry_chunks.get(entry) {
        for chunk_id in chunk_ids {
            // Find the chunk in the manifest to obtain its content hash.
            if let Some(chunk) = manifest.chunks.iter().find(|c| &c.id == chunk_id) {
                let hash_hex = chunk.hash.as_str();
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
