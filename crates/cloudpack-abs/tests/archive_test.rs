//! Unit tests for the on-disk `ManifestArchive`.

use std::collections::HashMap;

use tempfile::TempDir;
use cloudpack_abs::archive::ManifestArchive;
use cloudpack_graph::types::ChunkManifest;

fn manifest(build_id: &str) -> ChunkManifest {
    ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

#[test]
fn open_creates_archive_dir_when_missing() {
    let tmp = TempDir::new().expect("create temp dir");
    let dir = tmp.path().join("nested").join(".archive");
    assert!(!dir.exists());
    let archive = ManifestArchive::open(&dir, 10).expect("open should create dir");
    assert!(dir.is_dir());
    assert_eq!(archive.archive_dir, dir);
    assert_eq!(archive.retention, 10);
}

#[test]
fn open_accepts_existing_dir() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 5).expect("open on existing dir must succeed");
    assert_eq!(archive.retention, 5);
}

#[test]
fn install_writes_file_named_after_build_id() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open archive");
    let entry = archive.install(&manifest("abc123")).expect("install");
    assert_eq!(entry.build_id, "abc123");
    assert!(entry.bytes > 0);
    assert!(!entry.is_current);
    assert!(tmp.path().join("abc123.json").is_file());
}

#[test]
fn list_returns_entries_newest_first() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open archive");
    archive.install(&manifest("oldest")).expect("install 1");
    std::thread::sleep(std::time::Duration::from_millis(5));
    archive.install(&manifest("middle")).expect("install 2");
    std::thread::sleep(std::time::Duration::from_millis(5));
    archive.install(&manifest("newest")).expect("install 3");
    let entries = archive.list().expect("list");
    let ids: Vec<&str> = entries.iter().map(|e| e.build_id.as_str()).collect();
    assert_eq!(ids, vec!["newest", "middle", "oldest"]);
}

#[test]
fn list_on_empty_archive_returns_empty_vec() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open archive");
    let entries = archive.list().expect("list");
    assert!(entries.is_empty());
}

#[test]
fn install_prunes_oldest_when_over_retention() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open archive");
    for i in 0..15 {
        archive.install(&manifest(&format!("b{:02}", i))).expect("install");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let entries = archive.list().expect("list");
    assert_eq!(entries.len(), 10, "retention=10 + 15 installs must leave exactly 10 entries");
    let ids: Vec<String> = entries.iter().map(|e| e.build_id.clone()).collect();
    for i in 5..15 {
        assert!(ids.contains(&format!("b{:02}", i)), "expected b{:02} to be retained", i);
    }
    for i in 0..5 {
        assert!(!ids.contains(&format!("b{:02}", i)), "expected b{:02} to be pruned", i);
    }
}

// ---------------------------------------------------------------------------
// load()
// ---------------------------------------------------------------------------

#[test]
fn load_roundtrips_installed_manifest() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");

    let mut m = manifest("roundtrip");
    m.entry_chunks.insert("home".to_string(), vec!["chunk-a".to_string()]);
    archive.install(&m).expect("install");

    let loaded = archive.load("roundtrip").expect("load");
    assert_eq!(loaded.build_id, "roundtrip");
    assert_eq!(
        loaded.entry_chunks.get("home").map(|v| v.as_slice()),
        Some(["chunk-a".to_string()].as_slice()),
    );
}

#[test]
fn load_missing_build_id_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");
    let result = archive.load("never-installed");
    assert!(result.is_err(), "load() must fail for missing build_id");
}

// ---------------------------------------------------------------------------
// set_current() + current()
// ---------------------------------------------------------------------------

#[test]
fn set_current_creates_symlink_and_current_reads_it_back() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");
    archive.install(&manifest("first")).expect("install");
    assert_eq!(archive.current().expect("current"), None);
    archive.set_current("first").expect("set_current");
    assert_eq!(archive.current().expect("current"), Some("first".to_string()));
    let entries = archive.list().expect("list");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_current);
}

#[test]
fn set_current_swaps_atomically_to_another_build() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");
    archive.install(&manifest("v1")).expect("install v1");
    archive.install(&manifest("v2")).expect("install v2");
    archive.set_current("v1").expect("point at v1");
    assert_eq!(archive.current().expect("current"), Some("v1".to_string()));
    archive.set_current("v2").expect("swap to v2");
    assert_eq!(archive.current().expect("current"), Some("v2".to_string()));
}

#[test]
fn set_current_rejects_unknown_build_id() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");
    let err = archive.set_current("does-not-exist").expect_err("must fail");
    assert!(err.to_string().contains("does-not-exist"), "got: {err}");
}

// ---------------------------------------------------------------------------
// Idempotent install
// ---------------------------------------------------------------------------

#[test]
fn install_is_idempotent_for_same_build_id() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 10).expect("open");
    let entry_a = archive.install(&manifest("dup")).expect("install 1");
    std::thread::sleep(std::time::Duration::from_millis(15));
    let entry_b = archive.install(&manifest("dup")).expect("install 2");
    assert_eq!(entry_a.build_id, entry_b.build_id);
    assert_eq!(
        entry_a.installed_at_ms, entry_b.installed_at_ms,
        "second install must not rewrite the file (mtime preserved)"
    );
    let entries = archive.list().expect("list");
    assert_eq!(entries.len(), 1, "duplicate install must not double-store");
}

#[test]
fn current_entry_is_never_pruned() {
    let tmp = TempDir::new().expect("create temp dir");
    let archive = ManifestArchive::open(tmp.path(), 3).expect("open");
    archive.install(&manifest("v0")).expect("install v0");
    archive.set_current("v0").expect("point at v0");
    for i in 1..=5 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        archive.install(&manifest(&format!("v{i}"))).expect("install vN");
    }
    let ids: Vec<String> = archive
        .list().expect("list")
        .into_iter().map(|e| e.build_id).collect();
    assert!(ids.contains(&"v0".to_string()),
        "current entry must survive prune; got {ids:?}");
}
