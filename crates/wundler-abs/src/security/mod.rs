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
    pub fn from_config(config: &SecurityConfig) -> Result<Self, SecurityError> {
        let token = match &config.bearer_token_file {
            None => None,
            Some(path) => {
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| SecurityError::TokenFileRead(path.clone(), e))?;
                Some(SecretToken::new(raw.trim().as_bytes().to_vec()))
            }
        };
        Ok(Self { token })
    }

    /// Returns `true` if bearer-token authentication is active.
    pub fn is_enabled(&self) -> bool {
        self.token.is_some()
    }
}
