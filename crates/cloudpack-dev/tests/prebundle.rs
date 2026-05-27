//! Integration tests for the dependency pre-bundling cache.

use std::path::Path;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use cloudpack_dev::{compute_fingerprint, DepPrebundler};

fn write(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

fn make_project(tmp: &Path) {
    write(tmp, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
    write(tmp, "package-lock.json", r#"{"lockfileVersion":1}"#);
}

#[test]
fn cache_miss_creates_dir_with_index_json() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert!(r.cache_dir.is_dir());
    assert_eq!(r.cache_dir.file_name().unwrap().to_str().unwrap(), r.fingerprint);

    // PrebundleResult carries pre-warm stats.
    assert_eq!(r.new_entries, 0, "no node_modules in this fixture");
    assert_eq!(r.cached_entries, 0, "no node_modules in this fixture");

    let index_path = r.cache_dir.join("index.json");
    assert!(index_path.is_file());
    let body = std::fs::read_to_string(&index_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["fingerprint"], serde_json::Value::String(r.fingerprint.clone()));
    assert_eq!(parsed["new_entries"], serde_json::Value::from(0u64));
    assert_eq!(parsed["cached_entries"], serde_json::Value::from(0u64));
    assert_eq!(parsed["total_modules"], serde_json::Value::from(0u64));
}

#[test]
fn cache_hit_returns_instantly_without_touching_index() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project.path()).unwrap();
    assert!(!r1.from_cache);

    let sentinel = r1.cache_dir.join("index.json");
    std::fs::write(&sentinel, "SENTINEL").unwrap();

    let r2 = pre.ensure_fresh(project.path()).unwrap();
    assert!(r2.from_cache);
    assert_eq!(r1.fingerprint, r2.fingerprint);
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "SENTINEL");
}

#[test]
fn fingerprint_changes_invalidate_cache() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project.path()).unwrap();

    write(project.path(), "package.json", r#"{"name":"app","version":"2.0.0"}"#);
    let r2 = pre.ensure_fresh(project.path()).unwrap();

    assert_ne!(r1.fingerprint, r2.fingerprint);
    assert!(!r2.from_cache);
    assert!(r1.cache_dir.is_dir());
    assert!(r2.cache_dir.is_dir());
}

#[test]
fn ensure_fresh_leaves_no_tmp_dirs_behind() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    pre.ensure_fresh(project.path()).unwrap();

    for entry in std::fs::read_dir(cache.path()).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(!name.starts_with(".tmp-"), "tempdir {name:?} should have been renamed away");
    }
}

#[test]
fn concurrent_ensure_fresh_does_not_error() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let proj = project.path().to_path_buf();
    let cache_root = cache.path().to_path_buf();
    let cas_root = cas.path().to_path_buf();

    let t1 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        let cas_root = cas_root.clone();
        thread::spawn(move || {
            DepPrebundler::new(cache_root, cas_root, 14)
                .ensure_fresh(&proj)
                .unwrap()
        })
    };
    let t2 = {
        let proj = proj.clone();
        let cache_root = cache_root.clone();
        let cas_root = cas_root.clone();
        thread::spawn(move || {
            DepPrebundler::new(cache_root, cas_root, 14)
                .ensure_fresh(&proj)
                .unwrap()
        })
    };

    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    assert_eq!(r1.fingerprint, r2.fingerprint);
    assert_eq!(r1.cache_dir, r2.cache_dir);
    assert!(r1.cache_dir.join("index.json").is_file());
}

#[test]
fn gc_deletes_stale_dirs_and_keeps_fresh_ones() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 0);
    let r = pre.ensure_fresh(project.path()).unwrap();
    assert!(r.cache_dir.is_dir());

    thread::sleep(Duration::from_millis(1100));
    pre.gc().unwrap();

    assert!(!r.cache_dir.exists(), "ttl_days=0 must evict the directory after 1s");
}

#[test]
fn gc_keeps_fresh_dirs_under_default_ttl() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();
    pre.gc().unwrap();
    assert!(r.cache_dir.is_dir());
}

#[test]
fn gc_on_missing_root_is_not_an_error() {
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    let missing = cache.path().join("never-created");
    let pre = DepPrebundler::new(missing, cas.path().to_path_buf(), 14);
    pre.gc().unwrap();
}

#[test]
fn ensure_fresh_does_not_call_gc_inline() {
    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    let stale = cache.path().join("deadbeef".repeat(8));
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("index.json"), "{}").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let _ = pre.ensure_fresh(project.path()).unwrap();

    assert!(stale.is_dir(), "ensure_fresh must not touch unrelated entries");
}

#[test]
fn compute_fingerprint_is_reexported() {
    let project = TempDir::new().unwrap();
    make_project(project.path());
    let _fp = compute_fingerprint(project.path()).unwrap();
}

#[test]
fn ensure_fresh_prewarms_cas_for_node_modules() {
    use walkdir::WalkDir;

    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    // Fake a single dependency module under node_modules.
    let dep = project.path().join("node_modules").join("acme");
    std::fs::create_dir_all(&dep).unwrap();
    std::fs::write(dep.join("index.ts"), "export const x = 1;\n").unwrap();
    // A non-JS file must be ignored.
    std::fs::write(dep.join("README.md"), "# acme\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert_eq!(r.new_entries, 1, "should have summarized one .ts file");
    assert_eq!(r.cached_entries, 0, "first run, nothing was cached yet");

    // The CAS should now contain exactly one summary JSON file.
    let cas_files: Vec<_> = WalkDir::new(cas.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    assert_eq!(cas_files.len(), 1, "expected 1 CAS entry, got: {cas_files:?}");
}

#[test]
fn ensure_fresh_prewarms_workspace_packages() {
    use walkdir::WalkDir;

    let project = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();
    make_project(project.path());

    // Top-level node_modules with one module
    let top_dep = project.path().join("node_modules").join("alpha");
    std::fs::create_dir_all(&top_dep).unwrap();
    std::fs::write(top_dep.join("a.ts"), "export const a = 1;\n").unwrap();

    // Workspace package node_modules with one module
    let pkg_dep = project.path().join("pkg-a").join("node_modules").join("beta");
    std::fs::create_dir_all(&pkg_dep).unwrap();
    std::fs::write(pkg_dep.join("b.ts"), "export const b = 2;\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r = pre.ensure_fresh(project.path()).unwrap();

    assert!(!r.from_cache);
    assert_eq!(r.new_entries, 2, "both node_modules trees should have been walked");
    assert_eq!(r.cached_entries, 0);

    let cas_files: Vec<_> = WalkDir::new(cas.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    assert_eq!(cas_files.len(), 2, "expected 2 CAS entries");
}

#[test]
fn ensure_fresh_records_hits_on_warm_cas() {
    // A second project with the same dep content should report cached_entries=1.
    let cache = TempDir::new().unwrap();
    let cas = TempDir::new().unwrap();

    // Project 1 — cold CAS.
    let project1 = TempDir::new().unwrap();
    make_project(project1.path());
    let dep1 = project1.path().join("node_modules").join("shared");
    std::fs::create_dir_all(&dep1).unwrap();
    std::fs::write(dep1.join("index.ts"), "export const shared = 42;\n").unwrap();

    let pre = DepPrebundler::new(cache.path().to_path_buf(), cas.path().to_path_buf(), 14);
    let r1 = pre.ensure_fresh(project1.path()).unwrap();
    assert_eq!(r1.new_entries, 1);
    assert_eq!(r1.cached_entries, 0);

    // Project 2 — different fingerprint, same dep content.
    let project2 = TempDir::new().unwrap();
    write(project2.path(), "package.json", r#"{"name":"other","version":"9.9.9"}"#);
    let dep2 = project2.path().join("node_modules").join("shared");
    std::fs::create_dir_all(&dep2).unwrap();
    std::fs::write(dep2.join("index.ts"), "export const shared = 42;\n").unwrap();

    let r2 = pre.ensure_fresh(project2.path()).unwrap();
    assert_ne!(r1.fingerprint, r2.fingerprint);
    assert!(!r2.from_cache);
    assert_eq!(r2.new_entries, 0, "content was already in the CAS");
    assert_eq!(r2.cached_entries, 1, "and we should have recorded the hit");
}
