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
}

#[derive(Debug, Clone)]
pub struct DepPrebundler {
    cache_root: PathBuf,
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
    pub fn new(cache_root: PathBuf, ttl_days: u32) -> Self {
        Self { cache_root, ttl_days }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
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
