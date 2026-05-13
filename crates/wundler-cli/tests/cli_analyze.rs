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

    assert!(
        json["build_id"].is_string(),
        "expected build_id key in JSON"
    );
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

/// Test 4: scan root is a subdirectory and the entry path is given as a path
/// relative to CWD (i.e. it includes the subdirectory prefix).
///
/// Before the Bug-1 fix the CLI joined the scan-root with the entry path,
/// producing a double-prefix like `"sub/sub/a.ts"` which does not exist.
/// After the fix the entry path is used directly from CWD so it matches the
/// WalkDir path that starts with the subdirectory name.
#[test]
fn test_analyze_entry_does_not_double_path_when_scan_root_is_subdir() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir_all(&sub).unwrap();

    // Write a simple TypeScript module with no imports.
    fs::write(sub.join("a.ts"), "export const a = 1;\n").unwrap();

    // Run the binary from the *parent* directory so that "sub/a.ts" is the
    // correct CWD-relative path.  The scan root is "sub" (a subdirectory), and
    // the entry path "sub/a.ts" already contains the subdirectory prefix.
    //
    // The buggy behaviour joins scan_root with entry:
    //   "sub".join("sub/a.ts") = "sub/sub/a.ts"  → "not found in graph"
    //
    // The fixed behaviour uses the entry path directly, which matches the
    // WalkDir path produced by WalkDir::new("sub"):
    //   "sub/a.ts"  → found ✓
    let output = Command::new(wundler_bin())
        .arg("analyze")
        .arg("sub")
        .arg("--entry")
        .arg("/=sub/a.ts")
        .current_dir(dir.path())
        .output()
        .expect("failed to spawn wundler binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected exit 0 — entry should resolve without path-doubling, got:\n{stderr}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    let chunks = json["chunks"].as_array().unwrap();
    assert!(
        !chunks.is_empty(),
        "expected at least one chunk in the output"
    );
}

/// Test 5: scan root is "." and the entry is given as a CWD-relative path
/// (no "./" prefix).  This is the standard invocation from the project root
/// and the entry must resolve without any mangling.
///
/// This test acts as a regression guard: if the entry-path logic is ever
/// accidentally changed so that it adds an extra path component, the lookup
/// will fail with "not found in graph" and the binary will exit non-zero.
#[test]
fn test_analyze_entry_resolves_with_dot_scan_root() {
    use std::fs;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("app.ts"), "export const app = 1;\n").unwrap();

    // Run from within the project directory so "." really is the project root.
    let output = Command::new(wundler_bin())
        .arg("analyze")
        .arg(".")
        .arg("--entry")
        .arg("/=src/app.ts")
        .current_dir(dir.path())
        .output()
        .expect("failed to spawn wundler binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected exit 0 — entry should resolve with scan root '.', got:\n{stderr}"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    let chunks = json["chunks"].as_array().unwrap();
    assert!(
        !chunks.is_empty(),
        "expected at least one chunk in the output"
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
