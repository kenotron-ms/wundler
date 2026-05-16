//! Application state for the Asset Bundling Server.
//!
//! [`AppState`] wraps the current [`ChunkManifest`] in
//! `Arc<RwLock<Arc<ChunkManifest>>>` so that:
//!
//! * **Readers** clone the inner `Arc` under a short read guard — O(1) and never blocks.
//! * **Writers** swap the inner `Arc` atomically under a write guard.
//! * A separate `reload_lock` (`tokio::sync::Mutex`) serializes concurrent swaps.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ed25519_dalek::Signature;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};
use wundler_graph::ChunkManifest;

use crate::archive::ManifestArchive;
use crate::signing::ManifestVerifier;

/// Result of a successful swap.
#[derive(Debug, Clone, Serialize)]
pub struct SwapReport {
    pub previous: String,
    pub current: String,
}

/// Shared application state carried by every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub manifest: Arc<RwLock<Arc<ChunkManifest>>>,
    pub archive: Arc<ManifestArchive>,
    pub reload_lock: Arc<Mutex<()>>,
    pub cdn_base_url: Arc<String>,
    pub ttl_seconds: u64,
}

impl AppState {
    /// Load a manifest from disk, seed the archive, and return `AppState`.
    pub async fn load_signed_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
        archive: ManifestArchive,
        verify: Option<(&ManifestVerifier, &Signature)>,
    ) -> Result<Self> {
        let bytes = tokio::fs::read(manifest_path)
            .await
            .with_context(|| format!("failed to read manifest from {}", manifest_path.display()))?;

        let manifest: ChunkManifest = serde_json::from_slice(&bytes).with_context(|| {
            format!("failed to parse manifest JSON from {}", manifest_path.display())
        })?;

        if let Some((verifier, sig)) = verify {
            verifier.verify(&manifest, sig).context("manifest signature verification failed")?;
        }

        let build_id = manifest.build_id.clone();
        archive.install(&manifest).context("seed archive")?;
        archive.set_current(&build_id).context("point archive `current` at seed manifest")?;

        Ok(Self {
            manifest: Arc::new(RwLock::new(Arc::new(manifest))),
            archive: Arc::new(archive),
            reload_lock: Arc::new(Mutex::new(())),
            cdn_base_url: Arc::new(cdn_base_url),
            ttl_seconds,
        })
    }

    /// Convenience wrapper that skips signature verification.
    pub async fn load_from_disk(
        manifest_path: &Path,
        cdn_base_url: String,
        ttl_seconds: u64,
        archive: ManifestArchive,
    ) -> Result<Self> {
        Self::load_signed_from_disk(manifest_path, cdn_base_url, ttl_seconds, archive, None).await
    }

    /// Return an `Arc<ChunkManifest>` snapshot — O(1), never blocks writers.
    pub async fn snapshot(&self) -> Arc<ChunkManifest> {
        let guard = self.manifest.read().await;
        Arc::clone(&*guard)
    }

    /// Swap the active manifest to the archive entry for `build_id`.
    pub async fn swap_to(&self, build_id: &str) -> Result<SwapReport> {
        let _lock = self.reload_lock.lock().await;

        let new_manifest = self
            .archive
            .load(build_id)
            .with_context(|| format!("swap_to: load build_id={build_id}"))?;

        if new_manifest.build_id != build_id {
            return Err(anyhow::anyhow!(
                "swap_to: archive file for {build_id} has mismatched build_id={}",
                new_manifest.build_id
            ));
        }

        self.archive
            .set_current(build_id)
            .with_context(|| format!("swap_to: set_current {build_id}"))?;

        let new_arc = Arc::new(new_manifest);
        let mut guard = self.manifest.write().await;
        let previous = guard.build_id.clone();
        *guard = new_arc;
        drop(guard);

        Ok(SwapReport { previous, current: build_id.to_string() })
    }

    /// Re-read `archive.current` and swap to it.
    pub async fn reload_from_current(&self) -> Result<SwapReport> {
        let current = self
            .archive
            .current()
            .context("read archive `current` symlink")?
            .ok_or_else(|| anyhow::anyhow!("archive has no `current` symlink"))?;
        self.swap_to(&current).await
    }
}
