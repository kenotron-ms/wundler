// Package-level fast path cache for node_modules.
//
// ~80% of modules in a typical bundler run are stable vendor packages inside
// `node_modules/`. Re-hashing their source on every build is wasteful. Instead,
// we derive the cache key from the package's `package.json` (name + version +
// dependency lists) rather than from the raw source bytes.  Cache misses only
// occur when `package.json` changes (i.e., when a package is updated).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cache::local::LocalCache;
use crate::types::{ContentHash, ModuleSummary};

// ---------------------------------------------------------------------------
// find_package_dir
// ---------------------------------------------------------------------------

/// Locate the package directory that owns `module_path`.
///
/// Searches for `node_modules/` in the path string.  After that marker:
/// - If the first segment starts with `@` (scoped package), consumes two
///   segments (e.g. `@scope/name`).
/// - Otherwise consumes one segment (e.g. `lodash`).
///
/// Returns `Some(pkg_dir)` only when `pkg_dir/package.json` exists on disk.
pub fn find_package_dir(module_path: &Path) -> Option<PathBuf> {
    let path_str = module_path.to_string_lossy();
    let nm_marker = "node_modules/";

    let nm_pos = path_str.find(nm_marker)?;

    // Everything up to and including "node_modules/"
    let base = &path_str[..nm_pos + nm_marker.len()];
    let rest = &path_str[nm_pos + nm_marker.len()..];

    let components: Vec<&str> = rest.split('/').collect();
    let first = components.first()?;

    let pkg_dir = if first.starts_with('@') {
        // Scoped package: need at least two segments
        if components.len() < 2 {
            return None;
        }
        PathBuf::from(format!("{}{}/{}", base, components[0], components[1]))
    } else {
        // Normal package: one segment
        PathBuf::from(format!("{}{}", base, components[0]))
    };

    if pkg_dir.join("package.json").exists() {
        Some(pkg_dir)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// compute_package_hash
// ---------------------------------------------------------------------------

/// Compute a stable content hash for a package directory.
///
/// Reads `pkg_dir/package.json`, extracts `name`, `version`, `dependencies`,
/// and `peerDependencies`, then hashes the combined string.  The hash is
/// deterministic: two calls on the same directory always produce the same value.
pub fn compute_package_hash(pkg_dir: &Path) -> Result<ContentHash> {
    let json_str = fs::read_to_string(pkg_dir.join("package.json"))
        .with_context(|| format!("failed to read package.json in {}", pkg_dir.display()))?;

    let json: serde_json::Value = serde_json::from_str(&json_str)
        .with_context(|| format!("failed to parse package.json in {}", pkg_dir.display()))?;

    let name = json["name"].as_str().unwrap_or("unknown");
    let version = json["version"].as_str().unwrap_or("0.0.0");

    let empty_obj = serde_json::Value::Object(serde_json::Map::new());
    let deps = json.get("dependencies").unwrap_or(&empty_obj);
    let peer_deps = json.get("peerDependencies").unwrap_or(&empty_obj);

    let hash_input = format!(
        "{name}@{version}\ndeps:{}\npeerDeps:{}",
        serde_json::to_string(deps).context("failed to serialize dependencies")?,
        serde_json::to_string(peer_deps).context("failed to serialize peerDependencies")?
    );

    Ok(ContentHash::from_bytes(hash_input.as_bytes()))
}

// ---------------------------------------------------------------------------
// PackageLevelCache
// ---------------------------------------------------------------------------

/// A cache layer that uses package-level keys for `node_modules/` files.
///
/// For files inside `node_modules/`, the cache key is derived from the
/// package's `package.json` rather than the module source.  This means the
/// cache remains valid across builds as long as the package itself has not
/// changed.
///
/// For files outside `node_modules/`, the cache key falls back to a hash of
/// the raw source content.
pub struct PackageLevelCache {
    inner: LocalCache,
}

impl PackageLevelCache {
    /// Create a new `PackageLevelCache` wrapping `inner`.
    pub fn new(inner: LocalCache) -> Self {
        Self { inner }
    }

    /// Compute a cache key for `module_path` with the given `source` content.
    ///
    /// - If the module lives inside `node_modules/` and a `package.json` can
    ///   be found, the key is derived from the package hash + relative path
    ///   within the package.
    /// - Otherwise the key is derived from `source`.
    pub fn cache_key_for(&self, module_path: &Path, source: &str) -> Result<ContentHash> {
        if let Some(pkg_dir) = find_package_dir(module_path) {
            let pkg_hash = compute_package_hash(&pkg_dir)?;
            let rel = module_path.strip_prefix(&pkg_dir).with_context(|| {
                format!(
                    "failed to strip prefix {} from {}",
                    pkg_dir.display(),
                    module_path.display()
                )
            })?;
            let combined = format!("{}{}", pkg_hash.as_str(), rel.to_string_lossy());
            Ok(ContentHash::from_bytes(combined.as_bytes()))
        } else {
            Ok(ContentHash::from_source(source))
        }
    }

    /// Look up a cached [`ModuleSummary`] by its content hash key.
    pub fn get(&self, key: &ContentHash) -> Result<Option<ModuleSummary>> {
        self.inner.get(key)
    }

    /// Store a [`ModuleSummary`] under its content hash key.
    pub fn put(&self, key: &ContentHash, summary: &ModuleSummary) -> Result<()> {
        self.inner.put(key, summary)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // Helper: create a minimal fake package inside a temp directory
    // -----------------------------------------------------------------------

    /// Creates `<root>/node_modules/<pkg_name>/package.json` with minimal
    /// content and returns the package directory path.
    ///
    /// `pkg_name` may contain a `/` for scoped packages (e.g. `@scope/pkg`).
    fn create_fake_package(root: &Path, pkg_name: &str, version: &str) -> PathBuf {
        let pkg_dir = pkg_name
            .split('/')
            .fold(root.join("node_modules"), |acc, segment| acc.join(segment));
        fs::create_dir_all(&pkg_dir).unwrap();
        let package_json = serde_json::json!({
            "name": pkg_name,
            "version": version,
        });
        fs::write(pkg_dir.join("package.json"), package_json.to_string()).unwrap();
        pkg_dir
    }

    // -----------------------------------------------------------------------
    // find_package_dir tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_package_dir_detects_node_modules() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");

        // Simulate a module file inside the package
        let module_path = pkg_dir.join("utils").join("index.js");

        let result = find_package_dir(&module_path);
        assert!(
            result.is_some(),
            "expected Some for node_modules/lodash path"
        );
        assert_eq!(result.unwrap(), pkg_dir);
    }

    #[test]
    fn test_find_package_dir_returns_none_for_src_file() {
        // A path with no node_modules segment should return None
        let path = PathBuf::from("/some/project/src/components/Button.js");
        let result = find_package_dir(&path);
        assert!(result.is_none(), "expected None for non-node_modules path");
    }

    #[test]
    fn test_find_package_dir_handles_scoped_packages() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "@scope/pkg", "1.0.0");

        let module_path = pkg_dir.join("src").join("index.js");

        let result = find_package_dir(&module_path);
        assert!(result.is_some(), "expected Some for @scope/pkg path");
        assert_eq!(result.unwrap(), pkg_dir);
    }

    // -----------------------------------------------------------------------
    // compute_package_hash tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_compute_package_hash_is_stable() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");

        let hash1 = compute_package_hash(&pkg_dir).unwrap();
        let hash2 = compute_package_hash(&pkg_dir).unwrap();

        assert_eq!(hash1, hash2, "hash must be deterministic across calls");
    }

    // -----------------------------------------------------------------------
    // PackageLevelCache.cache_key_for tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_node_modules_key_is_stable_across_source_changes() {
        let dir = TempDir::new().unwrap();
        let pkg_dir = create_fake_package(dir.path(), "lodash", "4.17.21");
        let module_path = pkg_dir.join("utils").join("cloneDeep.js");

        let cache_dir = TempDir::new().unwrap();
        let cache =
            PackageLevelCache::new(LocalCache::new(cache_dir.path().to_path_buf()).unwrap());

        // Source content changes — key must remain the same because it is
        // derived from package.json, not from source bytes.
        let key1 = cache.cache_key_for(&module_path, "const x = 1;").unwrap();
        let key2 = cache
            .cache_key_for(&module_path, "const x = 999; // changed!")
            .unwrap();

        assert_eq!(
            key1, key2,
            "node_modules cache key must not change when only source content changes"
        );
    }

    #[test]
    fn test_source_file_key_changes_with_content() {
        let dir = TempDir::new().unwrap();
        // A path outside node_modules
        let module_path = dir.path().join("src").join("components").join("Button.js");

        let cache_dir = TempDir::new().unwrap();
        let cache =
            PackageLevelCache::new(LocalCache::new(cache_dir.path().to_path_buf()).unwrap());

        let key1 = cache.cache_key_for(&module_path, "const a = 1;").unwrap();
        let key2 = cache.cache_key_for(&module_path, "const a = 2;").unwrap();

        assert_ne!(
            key1, key2,
            "non-node_modules cache key must change when source content changes"
        );
    }
}
