//! Rolldown-based transform adapter: invokes Node.js with an embedded driver
//! script that uses `rolldown` for bundling, with an automatic SWC fallback.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};
use crate::swc_adapter::SwcTransformAdapter;
use wundler_core::types::{BundleGraphNode, ContentHash};
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
    // Internal: attempt rolldown bundling
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

        if !entry.eq("__empty__.js") {
            // Make sure the entry is present (it always is when derived from a node).
        }

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
                reason: format!(
                    "node driver exited with code {code}: {stderr}"
                ),
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
        })
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
}
