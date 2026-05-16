//! Unit tests for the on-disk `ManifestArchive`.

use std::collections::HashMap;

use tempfile::TempDir;
use wundler_abs::archive::ManifestArchive;
use wundler_graph::types::ChunkManifest;

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
