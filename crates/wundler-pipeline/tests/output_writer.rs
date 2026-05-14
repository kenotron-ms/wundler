//! Integration tests for `output.rs` — write_chunk, write_manifest, write_index_html.
//!
//! Acceptance criteria: `cargo test -p wundler-pipeline --test output_writer`
//! reports `test result: ok. 4 passed; 0 failed`.

use std::collections::HashMap;
use std::fs;

use tempfile::TempDir;
use wundler_core::types::ContentHash;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};
use wundler_pipeline::output::{write_chunk, write_index_html, write_manifest};
use wundler_transform::engine::ChunkOutput;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_chunk_output(code: &str, source_map: Option<String>) -> ChunkOutput {
    let hash = ContentHash::from_source(code);
    ChunkOutput {
        chunk_id: "chunk-0".to_string(),
        hash,
        code: code.to_string(),
        source_map,
    }
}

fn make_manifest_with_entry(entry: &str, chunk_id: &str, hash: ContentHash) -> ChunkManifest {
    let chunk = Chunk {
        id: chunk_id.to_string(),
        modules: vec![],
        hash,
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let mut entry_chunks = HashMap::new();
    entry_chunks.insert(entry.to_string(), vec![chunk_id.to_string()]);
    ChunkManifest {
        build_id: "test-build-id".to_string(),
        chunks: vec![chunk],
        entry_chunks,
        module_index: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Test 1: write_chunk creates a content-hashed JS file under chunks/
// ---------------------------------------------------------------------------

#[test]
fn write_chunk_creates_content_hashed_file() {
    let dir = TempDir::new().unwrap();
    let output = make_chunk_output("console.log('hello');", None);
    let hash_hex = output.hash.as_str().to_string();

    let path = write_chunk(dir.path(), &output).expect("write_chunk failed");

    let path_str = path.to_string_lossy();
    assert!(
        path_str.contains("chunks/") || path_str.contains("chunks\\"),
        "path must contain 'chunks/' — got: {path_str}"
    );
    assert!(
        path_str.ends_with(".js"),
        "path must end with '.js' — got: {path_str}"
    );
    assert!(
        path_str.contains(&hash_hex[..16]),
        "path must contain the content hash — got: {path_str}"
    );
    assert!(path.exists(), "chunk file must exist on disk");
}

// ---------------------------------------------------------------------------
// Test 2: write_chunk emits source-map file + sourceMappingURL comment
// ---------------------------------------------------------------------------

#[test]
fn write_chunk_emits_source_map_when_present() {
    let dir = TempDir::new().unwrap();
    let source_map_json = r#"{"version":3,"sources":[],"mappings":""}"#;
    let output = make_chunk_output("export const x = 1;", Some(source_map_json.to_string()));
    let hash_hex = output.hash.as_str().to_string();

    let js_path = write_chunk(dir.path(), &output).expect("write_chunk failed");

    // The .js file body must end with a sourceMappingURL comment.
    let body = fs::read_to_string(&js_path).expect("failed to read JS file");
    assert!(
        body.contains("//# sourceMappingURL="),
        "JS file must contain sourceMappingURL comment — got: {body}"
    );

    // A corresponding .js.map file must exist.
    let map_path = dir.path().join("chunks").join(format!("{hash_hex}.js.map"));
    assert!(
        map_path.exists(),
        "source-map file must exist at {map_path:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: write_manifest emits valid JSON at out_dir/manifest.json
// ---------------------------------------------------------------------------

#[test]
fn write_manifest_emits_valid_json() {
    let dir = TempDir::new().unwrap();
    let hash = ContentHash::from_source("test module source");
    let manifest = make_manifest_with_entry("main", "chunk-0", hash.clone());

    // id_to_hash maps the chunk's logical ID to its actual output-file hash.
    let mut id_to_hash = HashMap::new();
    id_to_hash.insert("chunk-0".to_string(), hash);

    write_manifest(dir.path(), &manifest, &id_to_hash).expect("write_manifest failed");

    let manifest_path = dir.path().join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json must exist");

    let contents = fs::read_to_string(&manifest_path).expect("failed to read manifest.json");
    let parsed: serde_json::Value =
        serde_json::from_str(&contents).expect("manifest.json must be valid JSON");

    assert!(
        parsed.is_object(),
        "manifest.json must be a JSON object — got: {parsed}"
    );
    assert!(
        parsed.get("build_id").is_some(),
        "manifest.json must contain 'build_id' — got: {parsed}"
    );
    assert!(
        parsed.get("chunks").is_some(),
        "manifest.json must contain 'chunks' — got: {parsed}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: write_index_html references initial chunks with <script> tags
// ---------------------------------------------------------------------------

#[test]
fn write_index_html_references_initial_chunks() {
    let dir = TempDir::new().unwrap();
    let hash = ContentHash::from_source("initial chunk code");
    let manifest = make_manifest_with_entry("main", "chunk-0", hash.clone());

    // id_to_hash maps the chunk's logical ID to its actual output-file hash.
    let mut id_to_hash = HashMap::new();
    id_to_hash.insert("chunk-0".to_string(), hash);

    write_index_html(dir.path(), &manifest, "main", &id_to_hash).expect("write_index_html failed");

    // Find the generated HTML file (index.html).
    let html_path = dir.path().join("index.html");
    assert!(html_path.exists(), "index.html must exist at {html_path:?}");

    let body = fs::read_to_string(&html_path).expect("failed to read index.html");

    assert!(
        body.contains("<script"),
        "index.html must contain a <script tag — got: {body}"
    );
    assert!(
        body.contains("chunks/"),
        "index.html must reference the chunks/ directory — got: {body}"
    );
    assert!(
        body.contains(".js"),
        "index.html must reference a .js file — got: {body}"
    );
}
