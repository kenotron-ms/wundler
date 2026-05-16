//! On-disk archive of installed `ChunkManifest`s.
//!
//! Layout:
//! ```text
//! <archive_dir>/
//!     a1b2c3d4e5f6a7b8.json
//!     0123456789abcdef.json
//!     current ──────────► a1b2c3d4e5f6a7b8.json  (symlink)
//! ```

use std::cmp::Reverse;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use wundler_graph::ChunkManifest;

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveEntry {
    pub build_id: String,
    pub installed_at_ms: u64,
    pub bytes: u64,
    pub is_current: bool,
}

#[derive(Debug, Clone)]
pub struct ManifestArchive {
    pub archive_dir: PathBuf,
    pub retention: usize,
}

impl ManifestArchive {
    pub fn open(dir: &Path, retention: usize) -> Result<Self> {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create archive dir {}", dir.display()))?;
        Ok(Self {
            archive_dir: dir.to_path_buf(),
            retention: retention.max(1),
        })
    }

    pub fn install(&self, manifest: &ChunkManifest) -> Result<ArchiveEntry> {
        let build_id = manifest.build_id.clone();
        let target = self.path_for(&build_id);

        if !target.exists() {
            let bytes = serde_json::to_vec(manifest).context("serialize manifest to JSON")?;
            write_atomic(&target, &bytes).with_context(|| {
                format!("failed to write archive entry {}", target.display())
            })?;
        }

        self.prune().context("prune after install")?;

        let mut entry = entry_for(&target, &build_id)?;
        entry.is_current = self.current()?.as_deref() == Some(build_id.as_str());
        Ok(entry)
    }

    pub fn list(&self) -> Result<Vec<ArchiveEntry>> {
        let current = self.current()?;
        let mut entries = read_dir_entries(&self.archive_dir, current.as_deref())?;
        entries.sort_by_key(|e| Reverse(e.installed_at_ms));
        Ok(entries)
    }

    pub fn current(&self) -> Result<Option<String>> {
        let link = self.archive_dir.join("current");
        match fs::read_link(&link) {
            Ok(target) => {
                let file_name = target
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| anyhow!("current symlink target has no file name"))?;
                let build_id = file_name
                    .strip_suffix(".json")
                    .ok_or_else(|| anyhow!("current symlink target lacks .json suffix"))?;
                Ok(Some(build_id.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context(format!("read_link {}", link.display())),
        }
    }

    fn path_for(&self, build_id: &str) -> PathBuf {
        self.archive_dir.join(format!("{build_id}.json"))
    }

    fn prune(&self) -> Result<()> {
        let current = self.current()?;
        let mut entries = read_dir_entries(&self.archive_dir, current.as_deref())?;
        entries.sort_by_key(|e| Reverse(e.installed_at_ms));

        if entries.len() <= self.retention {
            return Ok(());
        }

        for stale in entries.iter().skip(self.retention) {
            if stale.is_current {
                continue;
            }
            let path = self.path_for(&stale.build_id);
            if let Err(e) = fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    return Err(e)
                        .with_context(|| format!("prune: failed to remove {}", path.display()));
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path {} has no parent", path.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-")
        .suffix(".json")
        .tempfile_in(parent)
        .context("create temp file in archive dir")?;
    tmp.write_all(bytes).context("write temp file bytes")?;
    tmp.as_file().sync_all().context("fsync temp file")?;
    tmp.persist(path)
        .map_err(|e| anyhow!("persist temp file to {}: {}", path.display(), e.error))?;
    Ok(())
}

pub(crate) fn read_dir_entries(dir: &Path, current: Option<&str>) -> Result<Vec<ArchiveEntry>> {
    let mut out = Vec::new();
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| format!("read_dir {}", dir.display())),
    };

    for dent in read {
        let dent = dent.context("read_dir entry")?;
        let path = dent.path();
        let file_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if file_name == "current" || file_name.starts_with(".tmp-") {
            continue;
        }
        let build_id = match file_name.strip_suffix(".json") {
            Some(b) => b,
            None => continue,
        };
        let mut entry = entry_for(&path, build_id)?;
        entry.is_current = current == Some(build_id);
        out.push(entry);
    }
    Ok(out)
}

pub(crate) fn entry_for(path: &Path, build_id: &str) -> Result<ArchiveEntry> {
    let meta = fs::metadata(path)
        .with_context(|| format!("stat archive entry {}", path.display()))?;
    let mtime = meta
        .modified()
        .with_context(|| format!("mtime of {}", path.display()))?;
    let installed_at_ms = mtime
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Ok(ArchiveEntry {
        build_id: build_id.to_string(),
        installed_at_ms,
        bytes: meta.len(),
        is_current: false,
    })
}
