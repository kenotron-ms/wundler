//! Tests for bearer-token authentication — unit tests (SecretToken) and
//! HTTP integration tests (require_bearer middleware end-to-end).

use wundler_abs::security::auth::SecretToken;

// ── SecretToken unit tests ────────────────────────────────────────────────

/// Debug output must never contain the raw token bytes.
///
/// This is a hard security requirement: if a `SecretToken` is accidentally
/// included in a `tracing` event or a `{:?}` panic message, the token must
/// not be visible.
#[test]
fn test_secret_token_debug_does_not_leak_value() {
    let token = SecretToken::new(b"super-secret-token-value".to_vec());
    let debug_str = format!("{:?}", token);

    assert!(
        !debug_str.contains("super-secret-token-value"),
        "Debug output must not contain the raw token; got: {debug_str}"
    );
    assert!(
        debug_str.contains("REDACTED"),
        "Debug output should contain 'REDACTED'; got: {debug_str}"
    );
}

/// Display output must never contain the raw token bytes.
#[test]
fn test_secret_token_display_does_not_leak_value() {
    let token = SecretToken::new(b"another-secret-12345".to_vec());
    let display_str = format!("{}", token);

    assert!(
        !display_str.contains("another-secret-12345"),
        "Display output must not contain the raw token; got: {display_str}"
    );
    assert!(
        display_str.contains("REDACTED"),
        "Display output should contain 'REDACTED'; got: {display_str}"
    );
}

/// verify() returns true when the provided string matches the stored token.
#[test]
fn test_verify_correct_token_returns_true() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        token.verify("correct-token"),
        "verify() should return true for the correct token"
    );
}

/// verify() returns false when the provided string does not match.
#[test]
fn test_verify_wrong_token_returns_false() {
    let token = SecretToken::new(b"correct-token".to_vec());
    assert!(
        !token.verify("wrong-token"),
        "verify() should return false for an incorrect token"
    );
}

/// verify() returns false for an empty string, even if the token is non-empty.
#[test]
fn test_verify_empty_string_returns_false() {
    let token = SecretToken::new(b"non-empty-token".to_vec());
    assert!(
        !token.verify(""),
        "verify() should return false for an empty provided string"
    );
}

// ── SecurityConfig / ResolvedSecurity unit tests ──────────────────────────

use std::io::Write as _;
use tempfile::NamedTempFile;
use wundler_abs::security::{ResolvedSecurity, SecurityConfig, SecurityError};

/// When no `bearer_token_file` is set, security is disabled (pass-through).
#[test]
fn test_no_config_security_is_not_enabled() {
    let config = SecurityConfig::default();
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        !security.is_enabled(),
        "security should be disabled when no token file is configured"
    );
}

/// When a token file exists and is non-empty, security is enabled.
#[test]
fn test_token_file_present_security_is_enabled() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    write!(tmp, "my-bearer-token").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    assert!(
        security.is_enabled(),
        "security should be enabled when a token file is configured"
    );
}

/// Token files may have a trailing newline (common from `echo` or editors).
/// The token is trimmed before storage.
#[test]
fn test_token_file_with_trailing_newline_is_trimmed() {
    let mut tmp = NamedTempFile::new().expect("create token file");
    write!(tmp, "trimmed-token\n").expect("write token");

    let config = SecurityConfig {
        bearer_token_file: Some(tmp.path().to_path_buf()),
    };
    let security = ResolvedSecurity::from_config(&config).expect("from_config should succeed");
    // The stored token should verify against the trimmed string, not the newline-terminated one.
    assert!(
        security.token.as_ref().unwrap().verify("trimmed-token"),
        "token should be trimmed of trailing whitespace"
    );
    assert!(
        !security.token.as_ref().unwrap().verify("trimmed-token\n"),
        "token should not verify against the un-trimmed version"
    );
}

/// A non-existent token file path produces a `SecurityError::TokenFileRead`.
#[test]
fn test_missing_token_file_returns_error() {
    let config = SecurityConfig {
        bearer_token_file: Some("/nonexistent/path/that/does/not/exist/token.txt".into()),
    };
    let result = ResolvedSecurity::from_config(&config);
    assert!(
        result.is_err(),
        "from_config should fail when the token file does not exist"
    );
    // The error message should reference the path.
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("nonexistent"),
        "error message should mention the path; got: {err_str}"
    );
}
