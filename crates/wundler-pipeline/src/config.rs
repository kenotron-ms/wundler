//! `wundler.toml` schema — `[build]` + `[entry]` sections deserialization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which bundling engine to use.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    Swc,
    Rolldown,
    Rspack,
}

impl Default for EngineChoice {
    fn default() -> Self {
        EngineChoice::Swc
    }
}

/// Parsed, validated build configuration loaded from `wundler.toml`.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub source_maps: bool,
    pub commons_threshold: usize,
    pub engine: EngineChoice,
    pub entry_points: HashMap<String, PathBuf>,
}

// ---------------------------------------------------------------------------
// Raw (serde) types — private to this module
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawConfig {
    build: RawBuild,
    entry: HashMap<String, PathBuf>,
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
    /// Load and validate a `wundler.toml` file at `path`.
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
        })
    }
}
