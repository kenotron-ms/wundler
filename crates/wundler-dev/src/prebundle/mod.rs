//! Dependency pre-bundling cache.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrebundleResult {
    pub cache_dir: PathBuf,
    pub from_cache: bool,
    pub fingerprint: String,
    /// Number of summary entries newly written to the CAS during this run.
    /// Always 0 on `from_cache == true`.
    pub new_entries: u64,
    /// Number of summary entries that were already warm in the CAS.
    /// Always 0 on `from_cache == true`.
    pub cached_entries: u64,
}

#[derive(Debug, Clone)]
pub struct DepPrebundler {
    cache_root: PathBuf,
    cas_root: PathBuf,
    ttl_days: u32,
}

/// Compute `blake3(package.json || first-found-lockfile)` as lowercase hex.
///
/// Returns an error if `package.json` is missing. If no lockfile is present,
/// only `package.json` contributes to the hash.
pub fn compute_fingerprint(root: &Path) -> Result<String> {
    let pkg_path = root.join("package.json");
    let pkg_bytes = std::fs::read(&pkg_path)
        .with_context(|| format!("could not read {}", pkg_path.display()))?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(&pkg_bytes);

    for name in LOCKFILE_NAMES {
        let lock_path = root.join(name);
        if lock_path.is_file() {
            let lock_bytes = std::fs::read(&lock_path)
                .with_context(|| format!("could not read {}", lock_path.display()))?;
            hasher.update(&lock_bytes);
            break;
        }
    }

    Ok(hasher.finalize().to_hex().to_string())
}

impl DepPrebundler {
    pub fn new(cache_root: PathBuf, cas_root: PathBuf, ttl_days: u32) -> Self {
        Self { cache_root, cas_root, ttl_days }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn cas_root(&self) -> &Path {
        &self.cas_root
    }

    pub fn ensure_fresh(&self, project_root: &Path) -> Result<PrebundleResult> {
        let fingerprint = compute_fingerprint(project_root)?;
        let cache_dir = self.cache_root.join(&fingerprint);

        if cache_dir.join("index.json").is_file() {
            return Ok(PrebundleResult {
                cache_dir,
                from_cache: true,
                fingerprint,
                new_entries: 0,
                cached_entries: 0,
            });
        }

        std::fs::create_dir_all(&self.cache_root).with_context(|| {
            format!("could not create cache root {}", self.cache_root.display())
        })?;

        let (new_entries, cached_entries) =
            self.bundle_into(&fingerprint, &cache_dir, project_root)?;

        Ok(PrebundleResult {
            cache_dir,
            from_cache: false,
            fingerprint,
            new_entries,
            cached_entries,
        })
    }

    fn bundle_into(
        &self,
        fingerprint: &str,
        final_dir: &Path,
        _project_root: &Path,
    ) -> Result<(u64, u64)> {
        let tmp = tempfile::Builder::new()
            .prefix(&format!(".tmp-{fingerprint}-"))
            .tempdir_in(&self.cache_root)
            .with_context(|| {
                format!("could not create temp dir under {}", self.cache_root.display())
            })?;

        // Task 1 stub: zero stats. Task 3 replaces this with real CAS pre-warm.
        let new_entries: u64 = 0;
        let cached_entries: u64 = 0;
        let total_modules: u64 = 0;

        let index = serde_json::json!({
            "fingerprint": fingerprint,
            "new_entries": new_entries,
            "cached_entries": cached_entries,
            "total_modules": total_modules,
        });
        let index_bytes = serde_json::to_vec_pretty(&index)?;
        std::fs::write(tmp.path().join("index.json"), &index_bytes)
            .with_context(|| {
                format!("could not write index.json in {}", tmp.path().display())
            })?;

        let staged = tmp.keep();
        match std::fs::rename(&staged, final_dir) {
            Ok(()) => Ok((new_entries, cached_entries)),
            Err(_) if final_dir.join("index.json").is_file() => {
                let _ = std::fs::remove_dir_all(&staged);
                Ok((new_entries, cached_entries))
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staged);
                Err(anyhow::Error::new(e).context(format!(
                    "could not rename {} → {}",
                    staged.display(),
                    final_dir.display()
                )))
            }
        }
    }

    pub fn gc(&self) -> Result<()> {
        if !self.cache_root.is_dir() {
            return Ok(());
        }

        let ttl_secs = u64::from(self.ttl_days) * 24 * 60 * 60;
        let cutoff = match std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(ttl_secs))
        {
            Some(t) => t,
            None => return Ok(()),
        };

        let entries = std::fs::read_dir(&self.cache_root)
            .with_context(|| format!("could not read {}", self.cache_root.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("gc: bad dir entry under {}: {e}", self.cache_root.display());
                    continue;
                }
            };
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("gc: could not stat {}: {e}", path.display());
                    continue;
                }
            };
            if !meta.is_dir() {
                continue;
            }
            let mtime = match meta.modified() {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("gc: no mtime on {}: {e}", path.display());
                    continue;
                }
            };
            if mtime <= cutoff {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::warn!("gc: could not remove {}: {e}", path.display());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn fingerprint_is_lowercase_hex_and_stable() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.0"}"#);
        let fp1 = compute_fingerprint(tmp.path()).unwrap();
        let fp2 = compute_fingerprint(tmp.path()).unwrap();
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64);
        assert!(fp1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn fingerprint_changes_when_package_json_changes() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.0"}"#);
        let fp1 = compute_fingerprint(tmp.path()).unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x","version":"1.0.1"}"#);
        let fp2 = compute_fingerprint(tmp.path()).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_changes_when_lockfile_changes() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":1}"#);
        let fp1 = compute_fingerprint(tmp.path()).unwrap();
        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":2}"#);
        let fp2 = compute_fingerprint(tmp.path()).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn fingerprint_prefers_package_lock_over_pnpm_lock() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        write(tmp.path(), "package-lock.json", r#"{"lockfileVersion":1}"#);
        write(tmp.path(), "pnpm-lock.yaml", "lockfileVersion: '6.0'\n");
        let fp1 = compute_fingerprint(tmp.path()).unwrap();
        write(tmp.path(), "pnpm-lock.yaml", "lockfileVersion: '7.0'\n");
        let fp2 = compute_fingerprint(tmp.path()).unwrap();
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn fingerprint_missing_package_json_is_error() {
        let tmp = TempDir::new().unwrap();
        let err = compute_fingerprint(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("package.json"));
    }

    #[test]
    fn fingerprint_no_lockfile_still_works() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "package.json", r#"{"name":"x"}"#);
        let fp = compute_fingerprint(tmp.path()).unwrap();
        assert_eq!(fp.len(), 64);
    }
}
