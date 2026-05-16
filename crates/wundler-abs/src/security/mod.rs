//! Security middleware for the Asset Bundling Server.
//!
//! This module provides optional bearer-token authentication applied as an
//! Axum middleware layer. When the `[security]` section is absent from
//! `wundler.toml`, the middleware is a transparent pass-through — existing
//! behaviour is unchanged.

pub mod auth;

use std::path::PathBuf;

use crate::security::auth::SecretToken;

// ---------------------------------------------------------------------------
// SecurityConfig
// ---------------------------------------------------------------------------

/// Configuration for the ABS security layer, parsed from `[security]` in
/// `wundler.toml`.
///
/// All fields are optional; `Default` produces a no-security configuration
/// that preserves today's unauthenticated behaviour.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct SecurityConfig {
    /// Path to a file containing the bearer token (one line, trimmed).
    ///
    /// Storing the token in a file (rather than inline in `wundler.toml`)
    /// prevents accidental commit and log leakage.
    ///
    /// If this field is absent, bearer-token authentication is disabled.
    pub bearer_token_file: Option<PathBuf>,

    /// Allowlist of cross-origin request `Origin` header values.
    ///
    /// Empty (default) means no cross-origin requests are accepted.
    /// Wildcards ("*") are never accepted.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

// ---------------------------------------------------------------------------
// SecurityError
// ---------------------------------------------------------------------------

/// Errors that can occur while resolving a [`SecurityConfig`].
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// The bearer token file could not be read.
    #[error("failed to read bearer token file at {0}: {1}")]
    TokenFileRead(PathBuf, std::io::Error),

    #[error("invalid allowed_origins entry: {0:?}")]
    InvalidOrigin(String),
}

// ---------------------------------------------------------------------------
// ResolvedSecurity
// ---------------------------------------------------------------------------

/// The runtime-ready form of [`SecurityConfig`].
///
/// Created once at server startup by [`ResolvedSecurity::from_config`] and
/// then shared across all requests via `Arc`.
#[derive(Debug)]
pub struct ResolvedSecurity {
    /// The bearer token, or `None` if authentication is disabled.
    pub token: Option<SecretToken>,
    /// Parsed and validated CORS allowlist origins.
    pub allowed_origins: Vec<axum::http::HeaderValue>,
}

impl ResolvedSecurity {
    /// Build a `ResolvedSecurity` from a [`SecurityConfig`].
    ///
    /// When `config.bearer_token_file` is `Some`, the file is read
    /// synchronously (startup path, not per-request) and the content is
    /// trimmed of surrounding whitespace before being stored.
    ///
    /// # Errors
    ///
    /// Returns [`SecurityError::TokenFileRead`] if the file cannot be read.
    /// Returns [`SecurityError::InvalidOrigin`] if any entry in
    /// `config.allowed_origins` is `"*"` or is not a valid HTTP header value.
    pub fn from_config(config: &SecurityConfig) -> Result<Self, SecurityError> {
        let token = match &config.bearer_token_file {
            None => None,
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| SecurityError::TokenFileRead(path.clone(), e))?;
                Some(SecretToken::new(raw.trim().as_bytes().to_vec()))
            }
        };

        let mut allowed_origins = Vec::with_capacity(config.allowed_origins.len());
        for origin in &config.allowed_origins {
            if origin == "*" {
                return Err(SecurityError::InvalidOrigin(origin.clone()));
            }
            let hv = axum::http::HeaderValue::from_str(origin)
                .map_err(|_| SecurityError::InvalidOrigin(origin.clone()))?;
            allowed_origins.push(hv);
        }

        Ok(Self { token, allowed_origins })
    }

    /// Returns `true` if bearer-token authentication is active.
    pub fn is_enabled(&self) -> bool {
        self.token.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_origins_resolve_to_empty_vec() {
        let cfg = SecurityConfig::default();
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert!(resolved.allowed_origins.is_empty());
    }

    #[test]
    fn valid_origins_parse_to_header_values() {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec![
                "https://app.example.com".to_string(),
                "https://staging.example.com".to_string(),
            ],
        };
        let resolved = ResolvedSecurity::from_config(&cfg).expect("resolve");
        assert_eq!(resolved.allowed_origins.len(), 2);
        assert_eq!(
            resolved.allowed_origins[0].to_str().unwrap(),
            "https://app.example.com"
        );
        assert_eq!(
            resolved.allowed_origins[1].to_str().unwrap(),
            "https://staging.example.com"
        );
    }

    #[test]
    fn wildcard_origin_is_rejected() {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec!["*".to_string()],
        };
        let err = ResolvedSecurity::from_config(&cfg).expect_err("wildcard must fail");
        match err {
            SecurityError::InvalidOrigin(s) => assert_eq!(s, "*"),
            other => panic!("expected InvalidOrigin, got {other:?}"),
        }
    }

    #[test]
    fn invalid_header_value_origin_is_rejected() {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: vec!["https://app.example.com\n".to_string()],
        };
        let err = ResolvedSecurity::from_config(&cfg).expect_err("newline must fail");
        assert!(matches!(err, SecurityError::InvalidOrigin(_)));
    }
}
