use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;
use cloudpack_abs::state::AppState;
use cloudpack_core::types::ContentHash;

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

    let archive_dir = tempfile::TempDir::new().expect("archive tempdir");
    let archive = cloudpack_abs::archive::ManifestArchive::open(archive_dir.path(), 10)
        .expect("open archive");
    let state = AppState::load_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        archive,
    )
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
    let archive_dir = tempfile::TempDir::new().expect("archive tempdir");
    let archive = cloudpack_abs::archive::ManifestArchive::open(archive_dir.path(), 10)
        .expect("open archive");
    let result = AppState::load_from_disk(
        Path::new("/nonexistent/path/that/does/not/exist/manifest.json"),
        "https://cdn.example.com".to_string(),
        3600,
        archive,
    )
    .await;

    assert!(result.is_err(), "expected error for missing file");
}

#[tokio::test]
async fn malformed_manifest_returns_error() {
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(b"this is not valid json {{{")
        .expect("write bad bytes");

    let archive_dir = tempfile::TempDir::new().expect("archive tempdir");
    let archive = cloudpack_abs::archive::ManifestArchive::open(archive_dir.path(), 10)
        .expect("open archive");
    let result = AppState::load_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        archive,
    )
    .await;

    assert!(result.is_err(), "expected error for malformed JSON");
}

#[tokio::test]
async fn state_exposes_module_lookup() {
    let mut tmp = NamedTempFile::new().expect("create temp file");
    tmp.write_all(valid_manifest_json().as_bytes())
        .expect("write manifest JSON");

    let archive_dir = tempfile::TempDir::new().expect("archive tempdir");
    let archive = cloudpack_abs::archive::ManifestArchive::open(archive_dir.path(), 10)
        .expect("open archive");
    let state = AppState::load_from_disk(
        tmp.path(),
        "https://cdn.example.com".to_string(),
        3600,
        archive,
    )
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
use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

fn make_minimal_manifest(build_id: &str) -> ChunkManifest {
    ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

fn make_app_state(manifest: ChunkManifest) -> cloudpack_abs::state::AppState {
    // Leak the TempDir so the archive directory outlives this helper function.
    let tmp = Box::leak(Box::new(tempfile::TempDir::new().expect("make_app_state tempdir")));
    let archive = cloudpack_abs::archive::ManifestArchive::open(tmp.path(), 10)
        .expect("open archive");
    cloudpack_abs::state::AppState {
        manifest: Arc::new(RwLock::new(Arc::new(manifest))),
        archive: Arc::new(archive),
        reload_lock: Arc::new(tokio::sync::Mutex::new(())),
        signature: Arc::new(RwLock::new(None)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    }
}

/// Direct manifest write must swap and be reflected on the next read.
#[tokio::test]
async fn reload_manifest_swaps_and_returns_build_id() {
    let state = make_app_state(make_minimal_manifest("original-id"));

    let new_manifest = make_minimal_manifest("new-id-after-reload");
    let expected_id = new_manifest.build_id.clone();
    {
        let mut guard = state.manifest.write().await;
        *guard = Arc::new(new_manifest);
    }

    assert_eq!(
        expected_id, "new-id-after-reload",
        "direct write must swap to the new manifest's build_id"
    );

    let guard = state.manifest.read().await;
    assert_eq!(
        guard.build_id, "new-id-after-reload",
        "AppState.manifest must reflect the swapped manifest"
    );
}

/// Direct manifest write must replace the chunk list completely, not merge it.
#[tokio::test]
async fn reload_manifest_replaces_chunks_completely() {
    let initial = make_minimal_manifest("v1");
    let state = make_app_state(initial);

    use cloudpack_core::types::ContentHash;
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

    {
        let mut guard = state.manifest.write().await;
        *guard = Arc::new(new_manifest);
    }

    let guard = state.manifest.read().await;
    assert_eq!(guard.chunks.len(), 1, "reloaded manifest must have exactly 1 chunk");
    assert_eq!(guard.chunks[0].id, "chunk-new");
}

// ---------------------------------------------------------------------------
// VRC C2: snapshot() and swap_to()
// ---------------------------------------------------------------------------

use tempfile::TempDir;
use cloudpack_abs::archive::ManifestArchive;

fn make_app_state_with_archive(initial: ChunkManifest, dir: &std::path::Path) -> AppState {
    let archive = ManifestArchive::open(dir, 10).expect("open archive");
    archive.install(&initial).expect("seed archive");
    archive.set_current(&initial.build_id).expect("point current at seed");
    AppState {
        manifest: Arc::new(RwLock::new(Arc::new(initial))),
        archive: Arc::new(archive),
        reload_lock: Arc::new(tokio::sync::Mutex::new(())),
        signature: Arc::new(RwLock::new(None)),
        cdn_base_url: Arc::new("https://cdn.example.com".to_string()),
        ttl_seconds: 300,
    }
}

#[tokio::test]
async fn snapshot_clones_inner_arc_without_blocking_writer() {
    let tmp = TempDir::new().expect("tempdir");
    let state = make_app_state_with_archive(make_minimal_manifest("v1"), tmp.path());
    let snap = state.snapshot().await;
    assert_eq!(snap.build_id, "v1");

    let v2 = make_minimal_manifest("v2");
    state.archive.install(&v2).expect("install v2");
    let report = state.swap_to("v2").await.expect("swap");
    assert_eq!(report.previous, "v1");
    assert_eq!(report.current, "v2");

    assert_eq!(snap.build_id, "v1", "in-flight snapshot must be unaffected");
    let fresh = state.snapshot().await;
    assert_eq!(fresh.build_id, "v2");
}

#[tokio::test]
async fn swap_to_unknown_build_id_returns_error() {
    let tmp = TempDir::new().expect("tempdir");
    let state = make_app_state_with_archive(make_minimal_manifest("v1"), tmp.path());
    let err = state.swap_to("never-installed").await.expect_err("must fail");
    assert!(err.to_string().contains("never-installed"), "error: {err}");
}

#[tokio::test]
async fn swap_to_updates_archive_current_pointer() {
    let tmp = TempDir::new().expect("tempdir");
    let state = make_app_state_with_archive(make_minimal_manifest("v1"), tmp.path());
    let v2 = make_minimal_manifest("v2");
    state.archive.install(&v2).expect("install v2");
    state.swap_to("v2").await.expect("swap");
    assert_eq!(state.archive.current().expect("current"), Some("v2".to_string()));
}
