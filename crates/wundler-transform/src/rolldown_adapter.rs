//! Rolldown-based transform adapter: invokes Node.js with an embedded driver
//! script that uses `rolldown` for bundling, with an automatic SWC fallback.
//!
//! ## Per-chunk path (legacy)
//!
//! The `transform_chunk` method drives a Node.js subprocess that wraps
//! rolldown's programmatic API.  It feeds virtual-module source into rolldown
//! and captures the emitted ESM bundle.
//!
//! ## Batch path (new — Path B)
//!
//! `batch_transform` runs the `rolldown` CLI **once** for the whole project.
//! It generates a temporary `rolldown.config.mjs`, invokes
//! `<project>/node_modules/.bin/rolldown -c <config>`, and collects the
//! produced `.js` files from `config.out_dir`.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::engine::{
    sanitize_entry_key, BatchConfig, ChunkOutput, TransformDecisions, TransformEngine,
    TransformError,
};
use crate::swc_adapter::SwcTransformAdapter;
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::analyzer::AnalysisResult;
use wundler_graph::types::Chunk;

// ---------------------------------------------------------------------------
// Embedded Node.js driver script
// ---------------------------------------------------------------------------

/// Inline CJS driver script.  The JSON input path is passed as the last
/// process argument (after `--`).
///
/// Exit codes:
///   * 0  — success; JSON `{code, source_map}` written to stdout.
///   * 2  — `rolldown` package could not be loaded (not installed).
///   * 3  — rolldown threw an error during bundling.
const DRIVER_SCRIPT: &str = r#"
(async () => {
    const inputPath = process.argv[process.argv.length - 1];
    let rolldown;
    try {
        rolldown = require('rolldown');
    } catch (_e) {
        process.exit(2);
    }
    const fs = require('fs');
    let input;
    try {
        input = JSON.parse(fs.readFileSync(inputPath, 'utf8'));
    } catch (e) {
        process.stderr.write('failed to read input: ' + String(e));
        process.exit(3);
    }
    const virtualModules = input.modules;
    try {
        const bundle = await rolldown.rolldown({
            input: input.entry,
            plugins: [{
                name: 'wundler-virtual',
                resolveId(id) {
                    if (Object.prototype.hasOwnProperty.call(virtualModules, id)) return id;
                    return null;
                },
                load(id) {
                    if (Object.prototype.hasOwnProperty.call(virtualModules, id)) {
                        return { code: virtualModules[id] };
                    }
                    return null;
                }
            }]
        });
        const result = await bundle.generate({ format: 'esm' });
        const first = result.output[0];
        const out = {
            code: first.code,
            source_map: (first.map != null) ? JSON.stringify(first.map) : null
        };
        process.stdout.write(JSON.stringify(out));
    } catch (e) {
        process.stderr.write(String(e));
        process.exit(3);
    }
})().catch(e => {
    process.stderr.write(String(e));
    process.exit(3);
});
"#;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for [`RolldownAdapter`].
#[derive(Debug, Clone)]
pub struct RolldownAdapterConfig {
    /// Path (or name) of the `node` binary to invoke.  Defaults to `"node"`,
    /// which relies on `$PATH` resolution.
    pub node_path: PathBuf,

    /// When `true` (the default), fall back to [`SwcTransformAdapter`] if
    /// Node.js or `rolldown` is unavailable or the bundling step fails.
    /// When `false`, errors are propagated directly to the caller.
    pub fallback_on_failure: bool,
}

impl Default for RolldownAdapterConfig {
    fn default() -> Self {
        Self {
            node_path: PathBuf::from("node"),
            fallback_on_failure: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types (serialized to/from the Node.js driver)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RolldownInput {
    entry: String,
    modules: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RolldownOutput {
    code: String,
    source_map: Option<String>,
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// `TransformEngine` implementation that bundles via Node.js + rolldown,
/// with an optional SWC fallback when rolldown is unavailable.
pub struct RolldownAdapter {
    config: RolldownAdapterConfig,
    fallback: SwcTransformAdapter,
}

impl RolldownAdapter {
    /// Create a `RolldownAdapter` with default configuration.
    pub fn new() -> Self {
        Self {
            config: RolldownAdapterConfig::default(),
            fallback: SwcTransformAdapter::new(),
        }
    }

    /// Create a `RolldownAdapter` with the supplied configuration.
    pub fn with_config(config: RolldownAdapterConfig) -> Self {
        Self {
            config,
            fallback: SwcTransformAdapter::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Internal: attempt rolldown bundling (per-chunk path)
    // -----------------------------------------------------------------------

    fn try_rolldown(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
    ) -> Result<ChunkOutput, TransformError> {
        // Build virtual-module map: path -> source.
        let mut module_map: HashMap<String, String> = HashMap::new();
        for node in modules {
            let src = node.source.as_deref().ok_or_else(|| TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!("node {:?} is missing source for rolldown input", node.path),
            })?;
            module_map.insert(node.path.clone(), src.to_string());
        }

        // Choose an entry point: use the first module's path, or a placeholder
        // for empty chunks.
        let entry = modules
            .first()
            .map(|n| n.path.clone())
            .unwrap_or_else(|| "__empty__.js".to_string());

        let input = RolldownInput {
            entry,
            modules: module_map,
        };

        // Write input JSON to a named temp file.
        let mut tmp = NamedTempFile::new()?;
        let json_bytes = serde_json::to_vec(&input)?;
        tmp.write_all(&json_bytes)?;
        tmp.flush()?;

        let tmp_path = tmp.path().to_path_buf();

        // Invoke Node.js with the embedded driver script.
        let output = Command::new(&self.config.node_path)
            .args(["--input-type=commonjs", "-e", DRIVER_SCRIPT, "--"])
            .arg(&tmp_path)
            .output()
            .map_err(|e| TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!(
                    "failed to spawn node ({:?}): {}",
                    self.config.node_path, e
                ),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output.status.code().unwrap_or(-1);
            return Err(TransformError::TransformFailed {
                chunk_id: chunk.id.clone(),
                reason: format!("node driver exited with code {code}: {stderr}"),
            });
        }

        // Parse the JSON output from stdout.
        let rolldown_out: RolldownOutput = serde_json::from_slice(&output.stdout)?;

        // Compute a deterministic SHA-256 hash of the emitted code.
        let mut hasher = Sha256::new();
        hasher.update(rolldown_out.code.as_bytes());
        let hash = ContentHash(hex::encode(hasher.finalize()));

        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash,
            code: rolldown_out.code,
            source_map: rolldown_out.source_map,
            already_written: false,
        })
    }

    // -----------------------------------------------------------------------
    // Batch path helpers
    // -----------------------------------------------------------------------

    /// Walk up from `root` until we find `node_modules/.bin/rolldown`, or fall
    /// back to the bare name `"rolldown"` (PATH lookup).
    pub fn find_rolldown_bin(root: &Path) -> PathBuf {
        let mut dir = root;
        loop {
            let candidate = dir.join("node_modules").join(".bin").join("rolldown");
            if candidate.exists() {
                return candidate;
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
        PathBuf::from("rolldown")
    }

    /// Write a `rolldown.config.mjs` into `tmp_dir` that points rolldown at
    /// the entry files and output directory specified in `config`.
    ///
    /// All paths in the generated config are absolute so the config works
    /// from any working directory.
    pub fn write_rolldown_config(
        config: &BatchConfig,
        tmp_dir: &Path,
    ) -> anyhow::Result<PathBuf> {
        let cwd = std::env::current_dir()?;

        // Resolve out_dir to an absolute path.
        let abs_out_dir = if config.out_dir.is_absolute() {
            config.out_dir.clone()
        } else {
            cwd.join(&config.out_dir)
        };

        // Build the `input` object entries.
        let mut input_lines = Vec::new();
        for (route, entry_path) in &config.entry_points {
            let key = sanitize_entry_key(route);

            // Resolve entry path: try joining with root, then with cwd.
            let abs_entry = if entry_path.is_absolute() {
                entry_path.clone()
            } else {
                // First try: cwd / entry_path (works when entry is workspace-relative)
                let candidate = cwd.join(entry_path);
                if candidate.exists() {
                    candidate
                } else {
                    // Second try: root / entry_path (entry relative to scan root)
                    let abs_root = if config.root.is_absolute() {
                        config.root.clone()
                    } else {
                        cwd.join(&config.root)
                    };
                    abs_root.join(entry_path)
                }
            };

            input_lines.push(format!(
                "    {key}: {:?},",
                abs_entry.display().to_string()
            ));
        }
        let input_str = input_lines.join("\n");

        let config_content = format!(
            r#"export default {{
  input: {{
{input_str}
  }},
  output: {{
    dir: {out_dir:?},
    format: 'esm',
    entryFileNames: '[name]-[hash].js',
    chunkFileNames: 'chunk-[hash].js',
  }},
  platform: 'browser',
  resolve: {{
    extensions: ['.tsx', '.ts', '.jsx', '.js'],
  }},
}};
"#,
            out_dir = abs_out_dir.display().to_string(),
        );

        let cfg_path = tmp_dir.join("rolldown.config.mjs");
        std::fs::write(&cfg_path, &config_content)?;
        Ok(cfg_path)
    }

    /// Run `<bin> -c <config_path>` and return the process output.
    pub fn run_rolldown(bin: &Path, config_path: &Path) -> anyhow::Result<std::process::Output> {
        let output = Command::new(bin)
            .arg("-c")
            .arg(config_path)
            .output()
            .map_err(|e| anyhow::anyhow!("failed to spawn rolldown ({:?}): {}", bin, e))?;
        Ok(output)
    }

    /// Read all `.js` files written by rolldown to `out_dir`, compute a
    /// SHA-256 content hash for each, and return a [`ChunkOutput`] per file.
    ///
    /// The `chunk_id` is derived from the file's stem before the first `-`
    /// (e.g. `"root"` from `"root-abc123.js"`).
    fn collect_outputs(out_dir: &Path) -> anyhow::Result<Vec<ChunkOutput>> {
        std::fs::create_dir_all(out_dir)?;

        let mut outputs = Vec::new();

        for dir_entry in std::fs::read_dir(out_dir)? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("js") {
                continue;
            }

            let content = std::fs::read(&path)?;

            // Compute SHA-256 of the file content for the Wundler manifest.
            let mut hasher = Sha256::new();
            hasher.update(&content);
            let hash = ContentHash(hex::encode(hasher.finalize()));

            // Derive chunk_id from the filename stem: "root-abc123" → "root".
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("chunk");
            let chunk_id = stem
                .split('-')
                .next()
                .unwrap_or(stem)
                .to_string();

            outputs.push(ChunkOutput {
                chunk_id,
                hash,
                code: String::new(), // already written to disk by rolldown
                source_map: None,
                already_written: true,
            });
        }

        Ok(outputs)
    }
}

impl Default for RolldownAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TransformEngine impl
// ---------------------------------------------------------------------------

impl TransformEngine for RolldownAdapter {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        match self.try_rolldown(modules, chunk) {
            Ok(out) => Ok(out),
            Err(e) => {
                if self.config.fallback_on_failure {
                    self.fallback.transform_chunk(modules, chunk, decisions)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Batch path: invoke `rolldown` CLI once for the entire project.
    ///
    /// 1. Find `node_modules/.bin/rolldown` (walk up from `config.root`).
    /// 2. Write a `rolldown.config.mjs` to a temp directory with absolute paths.
    /// 3. Spawn `rolldown -c <config>`.
    /// 4. Collect every `.js` file rolldown wrote to `config.out_dir`.
    fn batch_transform(
        &self,
        _analysis: &AnalysisResult,
        config: &BatchConfig,
    ) -> Result<Vec<ChunkOutput>, TransformError> {
        let bin = Self::find_rolldown_bin(&config.root);

        // Write config to a temp dir.
        let tmp = tempfile::TempDir::new().map_err(|e| TransformError::TransformFailed {
            chunk_id: "rolldown".to_string(),
            reason: format!("failed to create temp dir: {e}"),
        })?;

        let cfg_path =
            Self::write_rolldown_config(config, tmp.path()).map_err(|e| {
                TransformError::TransformFailed {
                    chunk_id: "rolldown".to_string(),
                    reason: format!("failed to write rolldown config: {e}"),
                }
            })?;

        let out = Self::run_rolldown(&bin, &cfg_path).map_err(|e| {
            TransformError::TransformFailed {
                chunk_id: "rolldown".to_string(),
                reason: e.to_string(),
            }
        })?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            return Err(TransformError::TransformFailed {
                chunk_id: "rolldown".to_string(),
                reason: format!("rolldown exited with status {}: {stderr}\n{stdout}", out.status),
            });
        }

        Self::collect_outputs(&config.out_dir).map_err(|e| TransformError::TransformFailed {
            chunk_id: "rolldown".to_string(),
            reason: e.to_string(),
        })
    }
}
