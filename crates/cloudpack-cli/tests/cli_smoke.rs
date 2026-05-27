use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn cloudpack_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cloudpack"))
}

/// `cloudpack --help` must list both "build" and "dev" subcommands.
#[test]
fn cli_build_help_lists_subcommands() {
    let output = Command::new(cloudpack_bin())
        .arg("--help")
        .output()
        .expect("failed to spawn cloudpack binary");

    // clap prints help to stdout and exits 0 for --help.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "cloudpack --help returned non-zero: {:?}\n{combined}",
        output.status
    );
    assert!(
        combined.contains("build"),
        "expected 'build' in --help output, got:\n{combined}"
    );
    assert!(
        combined.contains("dev"),
        "expected 'dev' in --help output, got:\n{combined}"
    );
}

/// `cloudpack build --config <path>` must succeed end-to-end and produce
/// `dist/manifest.json` and `dist/chunks/` in the configured out_dir.
#[test]
fn cli_build_subcommand_runs_end_to_end() {
    let dir = TempDir::new().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // Write minimal TS source files.
    fs::write(
        src.join("util.ts"),
        "export function util(): number { return 42; }\n",
    )
    .unwrap();
    fs::write(
        src.join("index.ts"),
        "import { util } from './util';\nconsole.log(util());\n",
    )
    .unwrap();

    // Point out_dir at dist/ inside the tempdir.
    let out_dir = dir.path().join("dist");

    // Build the cloudpack.toml with absolute paths.
    // The entry path "src/index.ts" relies on the suffix-match fallback in the
    // graph analyzer (n.path.ends_with("/src/index.ts")) which works even when
    // the root is an absolute path and WalkDir produces absolute node paths.
    let root_str = src.display().to_string().replace('\\', "/");
    let out_str = out_dir.display().to_string().replace('\\', "/");
    let config_content = format!(
        "[build]\nroot = \"{root_str}\"\nout_dir = \"{out_str}\"\n\n[entry]\n\"/\" = \"src/index.ts\"\n"
    );

    let config_path = dir.path().join("cloudpack.toml");
    fs::write(&config_path, &config_content).unwrap();

    // Run `cloudpack build --config <path>`.
    let output = Command::new(cloudpack_bin())
        .arg("build")
        .arg("--config")
        .arg(&config_path)
        .output()
        .expect("failed to spawn cloudpack binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );

    // Assert expected artifacts exist.
    assert!(
        out_dir.join("manifest.json").exists(),
        "expected dist/manifest.json to exist after build"
    );
    assert!(
        out_dir.join("chunks").exists(),
        "expected dist/chunks/ to exist after build"
    );
}
