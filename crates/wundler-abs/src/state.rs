//! Application state for the Asset Bundling Server.
//!
//! [`AppState`] wraps a [`ChunkManifest`] in `Arc<RwLock<_>>` to support
//! future hot-reload without restarting the server.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ed25519_dalek::Signature;
use tokio::sync::RwLock;
use wundler_graph::ChunkManifest;

use crate::signing::ManifestVerifier;

/// Shared application state carried by every Axum handler.
///
/// All fields are cheaply `Clone`-able: cloning an `AppState` shares the same
/// underlying data via `Arc`.
#[derive(Clone)]
pub struct AppState {
    /// The chunk manifest, guarded for future hot-reload support.
    pub manifest: Arc<RwLock<ChunkManifest>>,

    /// Base URL of the CDN that serves chunk assets.
    pub cdn_base_url: Arc<String>,

    /// How long clients should cache a manifest response, in seconds.
    pub ttl_seconds: u64,
}

impl AppState {
    /// Load a [`ChunkManifest`] from a JSON file on disk and wrap it in
    /// [`AppState`], optionally verifying a cryptographic signature.
    ///
    /// When `verify` is `Some((verifier, sig))` the loaded manifest is checked
    /// against `sig` using `verifier`.  If verification fails the function
    /// returns an error; the `AppState` is never constructed from a manifest
    /// whose signature is invalid.
    ///
    /// When `verify` is `None` no signature check is performed and any
    /// well-formed manifest is accepted.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// * the file cannot be read,
    /// * the bytes are not valid `ChunkManifest` JSON, or
    /// * `verify` is `Some` and signature verification fails.
    pub async fn load_signed_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
        verify: Option<(&ManifestVerifier, &Signature)>,
    ) -> Result<Self> {
        let bytes = tokio::fs::read(manifest_path)
            .await
            .with_context(|| format!("failed to read manifest from {}", manifest_path.display()))?;

        let manifest: ChunkManifest = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "failed to parse manifest JSON from {}",
                manifest_path.display()
            )
        })?;

        if let Some((verifier, sig)) = verify {
            verifier
                .verify(&manifest, sig)
                .context("manifest signature verification failed")?;
        }

        Ok(Self {
            manifest: Arc::new(RwLock::new(manifest)),
            cdn_base_url: Arc::new(cdn_base_url),
            ttl_seconds,
        })
    }

    /// Load a [`ChunkManifest`] from a JSON file on disk and wrap it in
    /// [`AppState`].
    ///
    /// This is a convenience wrapper around [`Self::load_signed_from_disk`]
    /// that skips signature verification.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if the bytes are not
    /// valid `ChunkManifest` JSON.
    pub async fn load_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
    ) -> Result<Self> {
        Self::load_signed_from_disk(manifest_path, cdn_base_url, ttl_seconds, None).await
    }

    /// Atomically swap the in-memory manifest for `new_manifest`.
    ///
    /// Acquires the write lock, replaces the manifest, releases the lock, and
    /// returns the `build_id` of the newly loaded manifest. In-flight readers
    /// that already hold a read snapshot are unaffected.
    ///
    /// # Panics
    ///
    /// Panics if the `RwLock` is poisoned (should never happen in practice).
    pub async fn reload_manifest(&self, new_manifest: ChunkManifest) -> String {
        let build_id = new_manifest.build_id.clone();
        let mut guard = self.manifest.write().await;
        *guard = new_manifest;
        build_id
    }
}
