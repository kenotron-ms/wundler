//! Security middleware for the Asset Bundling Server.
//!
//! This module provides optional bearer-token authentication applied as an
//! Axum middleware layer. When the `[security]` section is absent from
//! `wundler.toml`, the middleware is a transparent pass-through — existing
//! behaviour is unchanged.

pub mod auth;
