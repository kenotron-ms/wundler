// Summarizer module — parses source files and emits compact ModuleSummary values.
pub mod ambient_refs;
pub mod call_edges;
pub mod exports;
pub mod imports;
pub mod parser;
pub mod side_effects;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::cjs;
use crate::summarizer::ambient_refs::extract_ambient_refs;
use crate::summarizer::call_edges::extract_call_edges;
use crate::summarizer::exports::extract_exports;
use crate::summarizer::imports::extract_imports;
use crate::summarizer::parser::parse_module;
use crate::summarizer::side_effects::analyze_side_effects;
use crate::types::{BundleGraphNode, ContentHash, ModuleSummary};

/// Reads a source file from disk, parses it, and produces a `BundleGraphNode`
/// containing the module's id, path, and full summary.
pub struct ModuleSummarizer;

impl ModuleSummarizer {
    /// Create a new `ModuleSummarizer`.
    pub fn new() -> Self {
        ModuleSummarizer
    }

    /// Summarize the module at `path`.
    ///
    /// - Reads the source via `fs::read_to_string`.
    /// - Computes a `ContentHash` from the original source.
    /// - For `.js` files that look like CJS, transpiles them to an ESM stub
    ///   before parsing (changes the effective extension to `.mjs`).
    /// - Runs all extractors and assembles a `BundleGraphNode`.
    pub fn summarize(&self, path: &Path) -> Result<BundleGraphNode> {
        let source = fs::read_to_string(path)?;

        // Hash the *original* source for stable cache keying.
        let id = ContentHash::from_source(&source);

        // Determine what source / path to feed the parser.
        let (parse_source, parse_path_buf) = {
            let is_js = path.extension().and_then(|e| e.to_str()) == Some("js");
            if is_js && cjs::is_cjs(&source) {
                let cjs_exports = cjs::detect_cjs_exports(&source);
                let stub = cjs::generate_cjs_stub(path, &cjs_exports);
                (stub, path.with_extension("mjs"))
            } else {
                (source.clone(), path.to_path_buf())
            }
        };

        let module = parse_module(&parse_source, &parse_path_buf)?;

        // Run all extractors.
        let exports = extract_exports(&module);
        let imports = extract_imports(&module);
        let side_effects = analyze_side_effects(&module);

        let exported_names: HashSet<String> = exports.iter().map(|e| e.name.clone()).collect();
        let call_edges = extract_call_edges(&module, &exported_names);
        let ambient_refs = extract_ambient_refs(&module);

        Ok(BundleGraphNode {
            id,
            path: path.to_string_lossy().into_owned(),
            summary: ModuleSummary {
                exports,
                imports,
                side_effects,
                call_edges,
                ambient_refs,
            },
            alive: false,
            chunk_id: None,
        })
    }
}

impl Default for ModuleSummarizer {
    fn default() -> Self {
        Self::new()
    }
}
