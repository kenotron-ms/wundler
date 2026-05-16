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
