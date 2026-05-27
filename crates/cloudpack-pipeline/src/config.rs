//! `cloudpack.toml` schema — `[build]` + `[entry]` sections deserialization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which bundling engine to use.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    #[default]
    Swc,
    Rolldown,
    Rspack,
}

/// `[dev]` section of `cloudpack.toml`. All fields optional.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DevConfig {
    #[serde(default)]
    pub dep_cache_ttl_days: Option<u32>,
}

/// Parsed, validated build configuration loaded from `cloudpack.toml`.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
    pub budget: Option<crate::budget::BudgetConfig>,
    pub dev: Option<DevConfig>,
}

// ---------------------------------------------------------------------------
// Raw (serde) types — private to this module
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
    #[serde(default)]
    budget: Option<crate::budget::BudgetConfig>,
    #[serde(default)]
    dev: Option<DevConfig>,
}

#[derive(Debug, Deserialize)]
struct RawBuild {
    root: PathBuf,
    out_dir: PathBuf,
    #[serde(default)]
    source_maps: bool,
    #[serde(default = "default_commons_threshold")]
    commons_threshold: usize,
    #[serde(default)]
    engine: EngineChoice,
}

fn default_commons_threshold() -> usize {
    2
}

// ---------------------------------------------------------------------------
// Loading logic
// ---------------------------------------------------------------------------

impl BuildConfig {
    /// Load and validate a `cloudpack.toml` file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read config file: {}", path.display()))?;

        let raw: RawConfig = toml::from_str(&text)
            .with_context(|| format!("failed to parse config file: {}", path.display()))?;

        if raw.entry.is_empty() {
            return Err(anyhow!(
                "at least one [entry] mapping is required in {}",
                path.display()
            ));
        }

        Ok(Self {
            root: raw.build.root,
            out_dir: raw.build.out_dir,
            source_maps: raw.build.source_maps,
            commons_threshold: raw.build.commons_threshold,
            engine: raw.build.engine,
            entry_points: raw.entry,
            budget: raw.budget,
            dev: raw.dev,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn absent_dev_section_yields_none() {
        let tmp = TempDir::new().unwrap();
        let cfg = write(tmp.path(), "cloudpack.toml", r#"
[build]
root = "src"
out_dir = "dist"

[entry]
main = "src/index.ts"
"#);
        let parsed = BuildConfig::load(&cfg).unwrap();
        assert!(parsed.dev.is_none());
    }

    #[test]
    fn dev_section_with_ttl_parses() {
        let tmp = TempDir::new().unwrap();
        let cfg = write(tmp.path(), "cloudpack.toml", r#"
[build]
root = "src"
out_dir = "dist"

[entry]
main = "src/index.ts"

[dev]
dep_cache_ttl_days = 7
"#);
        let parsed = BuildConfig::load(&cfg).unwrap();
        let dev = parsed.dev.expect("[dev] should be present");
        assert_eq!(dev.dep_cache_ttl_days, Some(7));
    }
}
