use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;
use wundler_abs::state::AppState;
use wundler_core::types::ContentHash;

/// A valid ChunkManifest JSON fixture used across multiple tests.
fn valid_manifest_json() -> &'static str {
    r#"{
        "build_id": "b8f3a1c2",
        "chunks": [
            {
                "id": "chunk-main",
                "modules": ["abc123"],
                "hash": "deadbeef",
                "load_condition": "Initial",
                "co_request_score": null,
                "median_load_order": null,
                "suggested_merge": null
            }
        ],
        "entry_chunks": {
            "home": ["chunk-main"]
        },
        "module_index": {
            "abc123": "chunk-main"
        }
    }"#
}

#[tokio::test]
async fn loads_manifest_from_disk() {
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(valid_manifest_json().as_bytes())
        .expect("write manifest JSON");

    let state = AppState::load_from_disk(tmp.path(), "https://cdn.example.com".to_string(), 3600)
        .await
        .expect("load_from_disk should succeed for valid JSON");

    let manifest = state.manifest.read().await;
    assert_eq!(manifest.build_id, "b8f3a1c2");
    assert_eq!(manifest.chunks.len(), 1);
    assert_eq!(manifest.chunks[0].id, "chunk-main");
    assert_eq!(
        manifest.entry_chunks.get("home").expect("home entry"),
        &vec!["chunk-main".to_string()]
    );
}

#[tokio::test]
async fn missing_manifest_file_returns_error() {
    let result = AppState::load_from_disk(
        Path::new("/nonexistent/path/that/does/not/exist/manifest.json"),
        "https://cdn.example.com".to_string(),
        3600,
    )
    .await;

    assert!(result.is_err(), "expected error for missing file");
}

#[tokio::test]
async fn malformed_manifest_returns_error() {
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(b"this is not valid json {{{")
        .expect("write bad bytes");

    let result =
        AppState::load_from_disk(tmp.path(), "https://cdn.example.com".to_string(), 3600).await;

    assert!(result.is_err(), "expected error for malformed JSON");
}

#[tokio::test]
async fn state_exposes_module_lookup() {
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(valid_manifest_json().as_bytes())
        .expect("write manifest JSON");

    let state = AppState::load_from_disk(tmp.path(), "https://cdn.example.com".to_string(), 3600)
        .await
        .expect("load_from_disk should succeed");

    let manifest = state.manifest.read().await;
    let hash = ContentHash("abc123".to_string());
    let chunk_id = manifest
        .module_index
        .get(&hash)
        .expect("module_index should contain abc123");
    assert_eq!(chunk_id, "chunk-main");
}

// ---------------------------------------------------------------------------
// reload_manifest tests
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

fn make_minimal_manifest(build_id: &str) -> ChunkManifest {
    ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

fn make_app_state(manifest: ChunkManifest) -> wundler_abs::state::AppState {
    wundler_abs::state::AppState {
        manifest: Arc::new(RwLock::new(manifest)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    }
}

/// reload_manifest must swap the manifest and return the new build_id.
#[tokio::test]
async fn reload_manifest_swaps_and_returns_build_id() {
    let state = make_app_state(make_minimal_manifest("original-id"));

    let new_manifest = make_minimal_manifest("new-id-after-reload");
    let returned_id = state.reload_manifest(new_manifest).await;

    assert_eq!(
        returned_id, "new-id-after-reload",
        "reload_manifest must return the new manifest's build_id"
    );

    let guard = state.manifest.read().await;
    assert_eq!(
        guard.build_id, "new-id-after-reload",
        "AppState.manifest must reflect the swapped manifest"
    );
}

/// reload_manifest must replace the chunk list, not merge it.
#[tokio::test]
async fn reload_manifest_replaces_chunks_completely() {
    let initial = make_minimal_manifest("v1");
    let state = make_app_state(initial);

    use wundler_core::types::ContentHash;
    let new_chunk = Chunk {
        id: "chunk-new".to_string(),
        modules: vec![ContentHash("abc123".to_string())],
        hash: ContentHash("def456".to_string()),
        load_condition: LoadCondition::Initial,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };
    let new_manifest = ChunkManifest {
        build_id: "v2".to_string(),
        chunks: vec![new_chunk],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    };

    state.reload_manifest(new_manifest).await;

    let guard = state.manifest.read().await;
    assert_eq!(guard.chunks.len(), 1, "reloaded manifest must have exactly 1 chunk");
    assert_eq!(guard.chunks[0].id, "chunk-new");
}
