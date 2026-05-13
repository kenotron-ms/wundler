// Local cache — content-hash-keyed on-disk cache for module summaries.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::types::{ContentHash, ModuleSummary};

/// Content-addressed on-disk cache for [`ModuleSummary`] values.
///
/// Entries are stored as JSON files sharded by the first two hex characters of
/// the content hash: `<root>/<HH>/<rest>.json`.  Writes are atomic via a
/// temporary file + rename so a crash mid-write never leaves a corrupt entry.
pub struct LocalCache {
    root: PathBuf,
}

impl LocalCache {
    /// Create a new cache rooted at `root`, creating the directory if needed.
    pub fn new(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create cache root: {}", root.display()))?;
        Ok(Self { root })
    }

    /// Create a cache using `~/.wundler/cache/summaries/` as the root.
    ///
    /// Falls back to the `USERPROFILE` env var (Windows), then `.` if neither
    /// `HOME` nor `USERPROFILE` is set.
    pub fn with_default_root() -> Result<Self> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        let root = PathBuf::from(home)
            .join(".wundler")
            .join("cache")
            .join("summaries");
        Self::new(root)
    }

    /// Look up a cached [`ModuleSummary`] by its content hash.
    ///
    /// Returns `Ok(None)` when no entry exists for `hash`.
    /// Returns an error if the entry exists but cannot be read or parsed
    /// (corrupt cache).
    pub fn get(&self, hash: &ContentHash) -> Result<Option<ModuleSummary>> {
        let path = self.entry_path(hash);
        if !path.exists() {
            return Ok(None);
        }
        let json = fs::read_to_string(&path)
            .with_context(|| format!("failed to read cache entry: {}", path.display()))?;
        let summary: ModuleSummary = serde_json::from_str(&json)
            .with_context(|| format!("corrupt cache entry: {}", path.display()))?;
        Ok(Some(summary))
    }

    /// Store a [`ModuleSummary`] under its content hash.
    ///
    /// The write is atomic: the JSON is first written to a `.tmp` sibling file
    /// and then renamed into place so a crash never leaves a half-written entry.
    pub fn put(&self, hash: &ContentHash, summary: &ModuleSummary) -> Result<()> {
        let path = self.entry_path(hash);
        // Ensure the shard sub-directory exists.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create shard directory: {}", parent.display())
            })?;
        }
        let json = serde_json::to_string(summary).context("failed to serialize module summary")?;
        // Atomic write: write to .tmp then rename.
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &json)
            .with_context(|| format!("failed to write temp cache file: {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &path)
            .with_context(|| format!("failed to rename temp cache file to: {}", path.display()))?;
        Ok(())
    }

    /// Return the total size in bytes of all files in the cache.
    ///
    /// Returns `0` if the cache root does not exist yet.
    pub fn total_bytes(&self) -> Result<u64> {
        if !self.root.exists() {
            return Ok(0);
        }
        let mut total: u64 = 0;
        for entry in WalkDir::new(&self.root) {
            let entry = entry.context("failed to walk cache directory")?;
            if entry.file_type().is_file() {
                total += entry
                    .metadata()
                    .context("failed to read file metadata")?
                    .len();
            }
        }
        Ok(total)
    }

    /// Compute the path for a cache entry.
    ///
    /// Shards by the first two hex characters:
    /// `<root>/<HH>/<rest>.json`
    fn entry_path(&self, hash: &ContentHash) -> PathBuf {
        let hex = hash.as_str();
        self.root
            .join(&hex[..2])
            .join(format!("{}.json", &hex[2..]))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Export, ExportKind, SideEffectMarker};
    use tempfile::TempDir;

    fn make_summary_with_foo_export() -> ModuleSummary {
        ModuleSummary {
            exports: vec![Export {
                name: "foo".to_string(),
                kind: ExportKind::Named,
                source: None,
            }],
            imports: vec![],
            side_effects: SideEffectMarker::None,
            call_edges: vec![],
            ambient_refs: vec![],
        }
    }

    #[test]
    fn test_put_and_get_round_trip() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("test_module");
        let summary = make_summary_with_foo_export();

        cache.put(&hash, &summary).unwrap();
        let result = cache.get(&hash).unwrap();

        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.exports.len(), 1);
        assert_eq!(retrieved.exports[0].name, "foo");
    }

    #[test]
    fn test_get_returns_none_for_missing_key() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("nonexistent_module_key");

        let result = cache.get(&hash).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_put_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("idempotent_test");
        let summary = make_summary_with_foo_export();

        cache.put(&hash, &summary).unwrap();
        cache.put(&hash, &summary).unwrap(); // second put must not error

        let result = cache.get(&hash).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_cache_is_sharded_into_subdirectories() {
        let dir = TempDir::new().unwrap();
        let cache = LocalCache::new(dir.path().to_path_buf()).unwrap();
        let hash = ContentHash::from_source("sharding_test");
        let summary = make_summary_with_foo_export();

        cache.put(&hash, &summary).unwrap();

        let hex = hash.as_str();
        let shard_dir = dir.path().join(&hex[..2]);
        assert!(
            shard_dir.exists(),
            "shard directory should exist: {}",
            shard_dir.display()
        );
    }
}
