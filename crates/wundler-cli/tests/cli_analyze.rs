use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn wundler_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_wundler"))
}

/// Test 1: minimal project — src/index.ts importing src/util.ts —
/// with `--entry /=src/index.ts` produces success status, stdout is valid JSON
/// with build_id/chunks/entry_chunks keys, and at least one chunk.
#[test]
fn test_analyze_minimal_project() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("util.ts"),
        "export function util(): number { return 42; }",
    )
    .unwrap();
    fs::write(
        src.join("index.ts"),
        "import { util } from './util';\nconsole.log(util());",
    )
    .unwrap();

    let output = Command::new(wundler_bin())
        .arg("analyze")
        .arg(dir.path())
        .arg("--entry")
        .arg("/=src/index.ts")
        .output()
        .expect("failed to spawn wundler binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\nstderr: {stderr}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert!(json["build_id"].is_string(), "expected build_id key in JSON");
    assert!(json["chunks"].is_array(), "expected chunks key in JSON");
    assert!(
        json["entry_chunks"].is_object(),
        "expected entry_chunks key in JSON"
    );

    let chunks = json["chunks"].as_array().unwrap();
    assert!(!chunks.is_empty(), "expected at least one chunk");
}

/// Test 2: multiple entries `--entry /a=...` and `--entry /b=...` yield
/// entry_chunks containing both routes.
#[test]
fn test_analyze_multiple_entries() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("a.ts"),
        "export function a(): number { return 1; }",
    )
    .unwrap();
    fs::write(
        src.join("b.ts"),
        "export function b(): number { return 2; }",
    )
    .unwrap();

    let output = Command::new(wundler_bin())
        .arg("analyze")
        .arg(dir.path())
        .arg("--entry")
        .arg("/a=src/a.ts")
        .arg("--entry")
        .arg("/b=src/b.ts")
        .output()
        .expect("failed to spawn wundler binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\nstderr: {stderr}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    let entry_chunks = json["entry_chunks"]
        .as_object()
        .expect("entry_chunks should be an object");

    assert!(
        entry_chunks.contains_key("/a"),
        "expected route /a in entry_chunks, got keys: {:?}",
        entry_chunks.keys().collect::<Vec<_>>()
    );
    assert!(
        entry_chunks.contains_key("/b"),
        "expected route /b in entry_chunks, got keys: {:?}",
        entry_chunks.keys().collect::<Vec<_>>()
    );
}

/// Test 3: malformed entry (no '=' sign) fails with non-zero exit and
/// stderr containing the word "entry".
#[test]
fn test_analyze_malformed_entry() {
    let dir = TempDir::new().unwrap();

    let output = Command::new(wundler_bin())
        .arg("analyze")
        .arg(dir.path())
        .arg("--entry")
        .arg("no_equals_sign_here")
        .output()
        .expect("failed to spawn wundler binary");

    assert!(
        !output.status.success(),
        "expected non-zero exit for malformed entry, got: {:?}",
        output.status
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.to_lowercase().contains("entry"),
        "expected 'entry' in stderr, got: {stderr}"
    );
}
