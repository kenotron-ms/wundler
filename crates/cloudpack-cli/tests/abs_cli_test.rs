//! Integration tests for the ABS-related CLI subcommands added in task-15:
//! `cloudpack abs serve`, `cloudpack abs keygen`, and `cloudpack build --sign`.

use std::process::Command;

fn cloudpack_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cloudpack"))
}

/// `cloudpack abs keygen --signing-out <path> --verifying-out <path>` must:
/// - exit 0
/// - write a file at the signing-out path containing "BEGIN PRIVATE KEY"
/// - write a file at the verifying-out path containing "BEGIN PUBLIC KEY"
#[test]
fn abs_keygen_writes_two_pem_files() {
    let dir = tempfile::TempDir::new().expect("failed to create tempdir");
    let signing_out = dir.path().join("test_signing.pem");
    let verifying_out = dir.path().join("test_verifying.pem");

    let output = Command::new(cloudpack_bin())
        .args([
            "abs",
            "keygen",
            "--signing-out",
            signing_out.to_str().unwrap(),
            "--verifying-out",
            verifying_out.to_str().unwrap(),
        ])
        .output()
        .expect("failed to spawn cloudpack binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );

    assert!(
        signing_out.exists(),
        "expected signing key file at {}",
        signing_out.display()
    );
    assert!(
        verifying_out.exists(),
        "expected verifying key file at {}",
        verifying_out.display()
    );

    let signing_content =
        std::fs::read_to_string(&signing_out).expect("failed to read signing key file");
    let verifying_content =
        std::fs::read_to_string(&verifying_out).expect("failed to read verifying key file");

    assert!(
        signing_content.contains("BEGIN PRIVATE KEY"),
        "expected 'BEGIN PRIVATE KEY' in signing key file, got:\n{signing_content}"
    );
    assert!(
        verifying_content.contains("BEGIN PUBLIC KEY"),
        "expected 'BEGIN PUBLIC KEY' in verifying key file, got:\n{verifying_content}"
    );
}

/// `cloudpack abs serve --help` must exit 0 and advertise the `--config` flag.
#[test]
fn abs_serve_help_advertises_config_flag() {
    let output = Command::new(cloudpack_bin())
        .args(["abs", "serve", "--help"])
        .output()
        .expect("failed to spawn cloudpack binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\n{combined}",
        output.status
    );
    assert!(
        combined.contains("--config"),
        "expected '--config' in 'abs serve --help' output, got:\n{combined}"
    );
}

/// `cloudpack build --help` must advertise the `--sign` flag.
#[test]
fn build_help_advertises_sign_flag() {
    let output = Command::new(cloudpack_bin())
        .args(["build", "--help"])
        .output()
        .expect("failed to spawn cloudpack binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        output.status.success(),
        "expected exit 0, got: {:?}\n{combined}",
        output.status
    );
    assert!(
        combined.contains("--sign"),
        "expected '--sign' in 'build --help' output, got:\n{combined}"
    );
}
