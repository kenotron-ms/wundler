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
