//! SWC-based transform adapter: TypeScript transpilation, dead export stripping,
//! and ESM scope-flattened chunk concatenation.

use std::collections::HashSet;

use anyhow::Result;
use swc_core::ecma::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat};

use crate::swc_util::{emit_module, parse_source};

/// Strip dead exports from `src`.
///
/// * Named exports (`export function foo`, `export const foo`, `export class Foo`) are
///   removed when the binding name is in `dead`.
/// * Re-export specifiers (`export { a, b } from './x'`) have dead specifiers removed;
///   if all specifiers become dead, the entire statement is removed.
/// * `export default ...` is **never** stripped, regardless of what is in `dead`.
pub fn strip_dead_exports(src: &str, dead: &HashSet<String>) -> Result<String> {
    let (cm, mut module) = parse_source("<input>.ts", src)?;
    if dead.is_empty() {
        return emit_module(cm, &module);
    }
    module.body = module
        .body
        .into_iter()
        .filter_map(|item| match item {
            ModuleItem::ModuleDecl(decl) => {
                filter_export_decl(decl, dead).map(ModuleItem::ModuleDecl)
            }
            other => Some(other),
        })
        .collect();
    emit_module(cm, &module)
}

fn filter_export_decl(decl: ModuleDecl, dead: &HashSet<String>) -> Option<ModuleDecl> {
    match decl {
        // Default exports are never stripped, regardless of what is in `dead`.
        d @ ModuleDecl::ExportDefaultDecl(_) => Some(d),
        d @ ModuleDecl::ExportDefaultExpr(_) => Some(d),

        // Named export declaration: `export function foo() {}`, `export const bar = 1`, etc.
        ModuleDecl::ExportDecl(export) => {
            let remove = decl_binding_name(&export.decl)
                .map(|name| dead.contains(&name))
                .unwrap_or(false);
            if remove {
                None
            } else {
                Some(ModuleDecl::ExportDecl(export))
            }
        }

        // Re-export bindings: `export { a, b } from './x'`
        ModuleDecl::ExportNamed(mut named) => {
            named.specifiers.retain(|spec| {
                let exported_name = export_specifier_name(spec);
                !dead.contains(&exported_name)
            });
            if named.specifiers.is_empty() {
                None
            } else {
                Some(ModuleDecl::ExportNamed(named))
            }
        }

        // `export * from './x'` — never stripped.
        d @ ModuleDecl::ExportAll(_) => Some(d),

        // All other module items (import declarations, etc.) — keep as-is.
        other => Some(other),
    }
}

/// Returns the exported name of a specifier (the name visible to importers).
fn export_specifier_name(spec: &ExportSpecifier) -> String {
    match spec {
        ExportSpecifier::Named(n) => {
            // `exported` is the alias; if absent the exported name equals the original.
            let name = n.exported.as_ref().unwrap_or(&n.orig);
            module_export_name_str(name)
        }
        ExportSpecifier::Default(_) => "default".to_string(),
        ExportSpecifier::Namespace(ns) => module_export_name_str(&ns.name),
    }
}

fn module_export_name_str(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::Ident(i) => i.sym.to_string(),
        // `Str::value` is a `Wtf8Atom` which lacks `Display`; convert via `Atom` first.
        ModuleExportName::Str(s) => s.value.to_atom_lossy().to_string(),
    }
}

/// Extract the single binding name from an export declaration.
///
/// For `export function foo()` → `Some("foo")`.
/// For `export const a = 1, b = 2` → `Some("a")` (first binding only).
///
/// **Known limitation:** for multi-binding `export const a = 1, b = 2` where only
/// `b` is dead, the entire statement is kept (returned as `Some`), not partially stripped.
fn decl_binding_name(decl: &Decl) -> Option<String> {
    match decl {
        Decl::Fn(f) => Some(f.ident.sym.to_string()),
        Decl::Class(c) => Some(c.ident.sym.to_string()),
        Decl::Var(v) => v.decls.first().and_then(|d| {
            if let Pat::Ident(ident) = &d.name {
                Some(ident.id.sym.to_string())
            } else {
                None
            }
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ESM scope-flattening helpers
// ---------------------------------------------------------------------------

/// Separate `import` declarations from the module body and strip the `export`
/// keyword from top-level declarations so they land in shared chunk scope.
///
/// Returns `(import_lines, body_text)` where:
/// * `import_lines` — every line that begins with `import ` (verbatim).
/// * `body_text`    — the remaining lines, with `export ` stripped from
///   top-level function/const/class/let/var declarations.
///
/// Lines of the form `export default …` are rewritten to
/// `const __default__ = …` so the binding stays in scope.
///
/// This is a line-oriented regex approach suitable for SWC-emitted JavaScript
/// (well-formatted, declarations always start at the beginning of a line).
fn strip_imports_from_transpiled_js(js: &str) -> (Vec<String>, String) {
    let mut imports = Vec::new();
    let mut body = String::new();

    for line in js.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("import ") {
            // Collect the whole import line; deduplication is handled by the caller.
            imports.push(line.to_string());
        } else if trimmed.starts_with("export ")
            && (trimmed.contains("function ")
                || trimmed.contains("const ")
                || trimmed.contains("class ")
                || trimmed.contains("let ")
                || trimmed.contains("var "))
        {
            // Strip the `export ` keyword; the declaration stays in shared scope.
            body.push_str(&line.replacen("export ", "", 1));
            body.push('\n');
        } else if trimmed.starts_with("export default ") {
            // Rename `export default <expr>` to `const __default__ = <expr>`.
            body.push_str(&line.replacen("export default ", "const __default__ = ", 1));
            body.push('\n');
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }

    (imports, body)
}

// ---------------------------------------------------------------------------
// SwcTransformAdapter
// ---------------------------------------------------------------------------

use crate::engine::{ChunkOutput, TransformDecisions, TransformEngine, TransformError};
use wundler_core::types::{BundleGraphNode, ContentHash};
use wundler_graph::types::Chunk;

/// Configuration for `SwcTransformAdapter`.
#[derive(Debug, Clone, Default)]
pub struct SwcAdapterConfig {
    /// When `true`, a v3 source map JSON is emitted alongside the code.
    pub source_maps: bool,
}

/// SWC-backed `TransformEngine` implementation.
///
/// For each chunk:
/// 1. Transpiles every TypeScript/TSX module to plain JavaScript via SWC
///    (`transform_ts_to_js`).
/// 2. Strips dead exports (`strip_dead_exports`).
/// 3. Hoists all `import` declarations to the top of the chunk and removes
///    the `export` keyword from inline declarations, producing a valid ESM
///    file with a flat shared scope (no IIFE wrappers).
pub struct SwcTransformAdapter {
    config: SwcAdapterConfig,
}

impl SwcTransformAdapter {
    /// Create a new `SwcTransformAdapter` with default configuration.
    pub fn new() -> Self {
        Self {
            config: SwcAdapterConfig::default(),
        }
    }

    /// Create a new `SwcTransformAdapter` with the given configuration.
    pub fn with_config(config: SwcAdapterConfig) -> Self {
        Self { config }
    }
}

impl Default for SwcTransformAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformEngine for SwcTransformAdapter {
    fn transform_chunk(
        &self,
        modules: &[BundleGraphNode],
        chunk: &Chunk,
        decisions: &TransformDecisions,
    ) -> Result<ChunkOutput, TransformError> {
        if modules.is_empty() {
            return Ok(ChunkOutput {
                chunk_id: chunk.id.clone(),
                hash: ContentHash::from_bytes(b""),
                code: format!("// chunk: {} (empty)\n", chunk.id),
                source_map: None,
            });
        }

        // Collect deduplicated import lines and per-module bodies.
        let mut all_imports: Vec<String> = Vec::new();
        let mut seen_imports: HashSet<String> = HashSet::new();
        let mut module_bodies: Vec<String> = Vec::new();

        // Per-module source paths for source-map generation.
        let mut sources: Vec<String> = Vec::new();

        for node in modules {
            let source = node.source.as_deref().ok_or_else(|| {
                TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("node {:?} missing source", node.path),
                }
            })?;

            // Bug 2 fix: transpile TypeScript → JavaScript before any further
            // processing.  This strips all TS-specific syntax (type annotations,
            // interfaces, enums, etc.) so the emitted chunk is valid ES module JS.
            let js =
                crate::swc_util::transform_ts_to_js(&node.path, source).map_err(|e| {
                    TransformError::TransformFailed {
                        chunk_id: chunk.id.clone(),
                        reason: format!("transform_ts_to_js({:?}): {}", node.path, e),
                    }
                })?;

            // Strip dead exports (if any are present for this module).
            let after_dce = if let Some(dead) = decisions.dead_exports.get(&node.id) {
                strip_dead_exports(&js, dead).map_err(|e| TransformError::TransformFailed {
                    chunk_id: chunk.id.clone(),
                    reason: format!("strip_dead_exports({:?}): {}", node.path, e),
                })?
            } else {
                js
            };

            // Bug 3 fix: extract imports and strip `export` keywords so this
            // module's declarations land in the shared chunk scope instead of
            // being wrapped in an IIFE.
            let (module_imports, module_body) = strip_imports_from_transpiled_js(&after_dce);

            // Deduplicate imports by exact line.
            for imp in module_imports {
                if seen_imports.insert(imp.clone()) {
                    all_imports.push(imp);
                }
            }

            module_bodies.push(format!("// --- {} ---\n{}", node.path, module_body));

            if self.config.source_maps {
                sources.push(node.path.clone());
            }
        }

        // Assemble the chunk in ESM order:
        //   1. chunk header comment
        //   2. deduplicated import declarations (top-level, as ESM requires)
        //   3. module bodies (declarations in shared scope)
        let mut combined = format!("// chunk: {}\n", chunk.id);

        if !all_imports.is_empty() {
            for imp in &all_imports {
                combined.push_str(imp);
                combined.push('\n');
            }
            combined.push('\n');
        }

        for body in &module_bodies {
            combined.push_str(body);
        }

        let hash = ContentHash::from_bytes(combined.as_bytes());

        let source_map = if self.config.source_maps {
            Some(build_source_map(&sources, &[]))
        } else {
            None
        };

        Ok(ChunkOutput {
            chunk_id: chunk.id.clone(),
            hash,
            code: combined,
            source_map,
        })
    }
}

/// Build a v3-format source map JSON string.
///
/// This is a "line-coarse" map: actual column/VLQ mappings are not produced;
/// instead the per-module source paths are recorded.  The `x_wundler_segments`
/// field can carry additional tooling metadata.
fn build_source_map(sources: &[String], segments: &[String]) -> String {
    let map = serde_json::json!({
        "version": 3,
        "file": serde_json::Value::Null,
        "sources": sources,
        "sourcesContent": Vec::<&str>::new(),
        "names": Vec::<&str>::new(),
        "mappings": "",
        "x_wundler_segments": segments,
    });
    map.to_string()
}
