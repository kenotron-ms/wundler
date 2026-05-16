//! Integration test: end-to-end 5-module TypeScript project build (Level 0 gate).
//!
//! Acceptance criteria:
//! * `cargo test -p wundler-pipeline --test integration_five_module` reports both
//!   tests passing.
//! * `cargo test --workspace` reports zero failures across every crate.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

// Serialize the two integration tests to prevent a cache-write race:
// both tests process the same source files (identical content hashes) and
// the LocalCache uses an atomic-write pattern (write tmp → rename) that can
// race when two threads try to write the same cache entry concurrently.
static PIPELINE_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Absolute path to the five-module-app fixture directory.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("five-module-app")
}

/// Copy every `.ts` file from `<fixture_dir>/src/` into `<dest>/src/`.
fn copy_fixture_sources(dest: &Path) {
    let src_dir = fixture_dir().join("src");
    let dest_src = dest.join("src");
    fs::create_dir_all(&dest_src).expect("failed to create dest/src dir");

    for entry in fs::read_dir(&src_dir).expect("fixture src/ not found — fixtures missing?") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "ts").unwrap_or(false) {
            let dest_file = dest_src.join(path.file_name().unwrap());
            fs::copy(&path, &dest_file).unwrap();
        }
    }
}

/// Build a `BuildConfig` for the five-module fixture project.
///
/// Entry points: "/" → index.ts, "/dashboard" → dashboard.ts, "/settings" → settings.ts.
/// source_maps: true, commons_threshold: 2.
fn make_five_module_config(project_dir: &Path, out_dir: &Path) -> BuildConfig {
    let mut entry_points = HashMap::new();
    entry_points.insert("/".to_string(), PathBuf::from("src/index.ts"));
    entry_points.insert("/dashboard".to_string(), PathBuf::from("src/dashboard.ts"));
    entry_points.insert("/settings".to_string(), PathBuf::from("src/settings.ts"));

    BuildConfig {
        root: project_dir.to_path_buf(),
        out_dir: out_dir.to_path_buf(),
        source_maps: true,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points,
        budget: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: full end-to-end build of the 5-module project
// ---------------------------------------------------------------------------

#[test]
fn five_module_project_builds_end_to_end() {
    let _guard = PIPELINE_LOCK.lock().unwrap();

    let project_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();

    // Copy fixture TypeScript sources into the temp project.
    copy_fixture_sources(project_dir.path());

    let config = make_five_module_config(project_dir.path(), out_dir.path());
    let pipeline = BuildPipeline::new(config);

    let result = pipeline.build().expect("build() failed");
    let stats = &result.stats;

    // ---- Module counts ----

    // Five .ts files → five modules discovered.
    assert_eq!(
        stats.total_modules, 5,
        "expected total_modules = 5, got {}",
        stats.total_modules
    );

    // orphan.ts is not reachable from any entry point → at least 1 dead module.
    assert!(
        stats.dead_modules >= 1,
        "expected dead_modules ≥ 1 (orphan.ts is dead), got {}",
        stats.dead_modules
    );

    // alive + dead must always equal total.
    assert_eq!(
        stats.alive_modules + stats.dead_modules,
        stats.total_modules,
        "alive ({}) + dead ({}) must equal total ({})",
        stats.alive_modules,
        stats.dead_modules,
        stats.total_modules
    );

    // ---- Output files ----

    // manifest.json must exist.
    let manifest_path = out_dir.path().join("manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest.json must exist at {manifest_path:?}"
    );

    // index.html must exist (written for the last entry point processed).
    let html_path = out_dir.path().join("index.html");
    assert!(
        html_path.exists(),
        "dist/index.html must exist at {html_path:?}"
    );

    // ---- Chunks directory ----

    let chunks_dir = out_dir.path().join("chunks");
    assert!(
        chunks_dir.exists(),
        "chunks/ directory must exist at {chunks_dir:?}"
    );

    // The number of .js files must match chunks_written.
    let js_files: Vec<_> = fs::read_dir(&chunks_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "js")
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        js_files.len(),
        stats.chunks_written,
        "expected {} .js files in chunks/ (= chunks_written), got {}",
        stats.chunks_written,
        js_files.len()
    );

    // Every chunk file must be non-empty.
    for f in &js_files {
        let size = f.metadata().unwrap().len();
        assert!(
            size > 0,
            "chunk file {:?} must not be empty",
            f.path()
        );
    }

    // ---- Manifest JSON validity ----

    let manifest_text = fs::read_to_string(&manifest_path).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("manifest.json must be valid JSON");

    assert!(
        parsed.is_object(),
        "manifest.json must be a JSON object — got: {parsed}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: two successive builds of the same sources must produce identical
//         chunk-hash sets (determinism gate).
// ---------------------------------------------------------------------------

#[test]
fn integration_build_is_deterministic() {
    let _guard = PIPELINE_LOCK.lock().unwrap();

    let project_dir = TempDir::new().unwrap();
    let out_dir1 = TempDir::new().unwrap();
    let out_dir2 = TempDir::new().unwrap();

    // Copy fixture sources once; both builds share the same project tree.
    copy_fixture_sources(project_dir.path());

    // --- Build 1 ---
    let config1 = make_five_module_config(project_dir.path(), out_dir1.path());
    let result1 = BuildPipeline::new(config1)
        .build()
        .expect("first build failed");

    // --- Build 2 ---
    let config2 = make_five_module_config(project_dir.path(), out_dir2.path());
    let result2 = BuildPipeline::new(config2)
        .build()
        .expect("second build failed");

    // Sort chunk hashes from both runs and compare.
    let mut hashes1: Vec<String> = result1
        .manifest
        .chunks
        .iter()
        .map(|c| c.hash.as_str().to_string())
        .collect();
    hashes1.sort();

    let mut hashes2: Vec<String> = result2
        .manifest
        .chunks
        .iter()
        .map(|c| c.hash.as_str().to_string())
        .collect();
    hashes2.sort();

    assert_eq!(
        hashes1, hashes2,
        "build must be deterministic: sorted chunk hashes must be identical across two runs"
    );
}
