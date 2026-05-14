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
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::cache::local::LocalCache;
use crate::cjs;
use crate::summarizer::ambient_refs::extract_ambient_refs;
use crate::summarizer::call_edges::extract_call_edges;
use crate::summarizer::exports::extract_exports;
use crate::summarizer::imports::extract_imports;
use crate::summarizer::parser::parse_module;
use crate::summarizer::side_effects::analyze_side_effects;
use crate::types::{BundleGraphNode, ContentHash, ModuleSummary};

const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

// ---------------------------------------------------------------------------
// Summarize-with-stats types
// ---------------------------------------------------------------------------

/// Aggregate statistics from a `summarize_directory_with_stats` call.
#[derive(Debug, Clone, Default)]
pub struct SummarizeStats {
    /// Total number of JS/TS files processed.
    pub total: usize,
    /// Files whose summary was read from the on-disk cache (no SWC parse).
    pub cache_hits: usize,
    /// Files that were SWC-parsed and written back to the cache.
    pub cache_misses: usize,
}

/// Return value of [`summarize_directory_with_stats`].
pub struct SummarizeResult {
    /// One [`BundleGraphNode`] per discovered JS/TS file.
    pub nodes: Vec<BundleGraphNode>,
    /// Cache-hit / miss counters for this run.
    pub stats: SummarizeStats,
}

/// Like [`summarize_directory`] but also returns per-run cache statistics.
///
/// Useful for verifying cache behaviour in benchmarks and tests: after a warm
/// run only one file should be a miss (the touched file), and every other file
/// should be a hit.
pub fn summarize_directory_with_stats(
    dir: &Path,
    cache: &LocalCache,
) -> Result<SummarizeResult> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let paths: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if JS_EXTENSIONS.contains(&ext.as_str()) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    let total = paths.len();
    let hits_counter = Arc::new(AtomicUsize::new(0));
    let misses_counter = Arc::new(AtomicUsize::new(0));

    let nodes = {
        let hits = Arc::clone(&hits_counter);
        let misses = Arc::clone(&misses_counter);
        paths
            .par_iter()
            .map(move |path| {
                let source = fs::read_to_string(path)?;
                let hash = ContentHash::from_source(&source);
                if let Some(cached_summary) = cache.get(&hash)? {
                    hits.fetch_add(1, Ordering::Relaxed);
                    return Ok(BundleGraphNode {
                        id: hash,
                        path: path.to_string_lossy().into_owned(),
                        summary: cached_summary,
                        alive: false,
                        chunk_id: None,
                        source: None,
                    });
                }
                misses.fetch_add(1, Ordering::Relaxed);
                let summarizer = ModuleSummarizer::new();
                let node = summarizer.summarize(path)?;
                cache.put(&node.id, &node.summary)?;
                Ok(node)
            })
            .collect::<Vec<Result<_>>>()
            .into_iter()
            .collect::<Result<Vec<_>>>()?
    };

    let cache_hits = hits_counter.load(Ordering::Relaxed);
    let cache_misses = misses_counter.load(Ordering::Relaxed);

    Ok(SummarizeResult {
        nodes,
        stats: SummarizeStats {
            total,
            cache_hits,
            cache_misses,
        },
    })
}

/// Walks `dir` recursively and summarizes every file whose extension is one of
/// [`JS_EXTENSIONS`], returning one [`BundleGraphNode`] per file.
///
/// Results are computed in parallel via Rayon. Cache hits short-circuit the
/// parse step: if a cached [`ModuleSummary`] exists for the file's content
/// hash, the node is reconstructed from the cache without re-parsing.
pub fn summarize_directory(dir: &Path, cache: &LocalCache) -> Result<Vec<BundleGraphNode>> {
    let paths: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if JS_EXTENSIONS.contains(&ext.as_str()) {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    paths
        .par_iter()
        .map(|path| {
            let source = fs::read_to_string(path)?;
            let hash = ContentHash::from_source(&source);
            if let Some(cached_summary) = cache.get(&hash)? {
                return Ok(BundleGraphNode {
                    id: hash,
                    path: path.to_string_lossy().into_owned(),
                    summary: cached_summary,
                    alive: false,
                    chunk_id: None,
                    source: None,
                });
            }
            let summarizer = ModuleSummarizer::new();
            let node = summarizer.summarize(path)?;
            cache.put(&node.id, &node.summary)?;
            Ok(node)
        })
        .collect::<Vec<Result<_>>>()
        .into_iter()
        .collect::<Result<Vec<_>>>()
}

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
            source: None,
        })
    }
}

impl Default for ModuleSummarizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::local::LocalCache;
    use std::fs as sfs;
    use tempfile::TempDir;

    // RED test: summarize_directory_with_stats reports accurate cache_hits
    #[test]
    fn test_summarize_directory_with_stats_reports_cache_hits() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        sfs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();
        sfs::write(dir.path().join("b.ts"), "export const b = 2;").unwrap();
        sfs::write(dir.path().join("c.ts"), "export const c = 3;").unwrap();

        // Cold run — all misses
        let result1 = summarize_directory_with_stats(dir.path(), &cache).unwrap();
        assert_eq!(result1.nodes.len(), 3);
        assert_eq!(result1.stats.total, 3);
        assert_eq!(result1.stats.cache_misses, 3, "cold run should have 3 misses");
        assert_eq!(result1.stats.cache_hits, 0, "cold run should have 0 hits");

        // Warm run — all hits
        let result2 = summarize_directory_with_stats(dir.path(), &cache).unwrap();
        assert_eq!(result2.stats.cache_hits, 3, "warm run should have 3 hits");
        assert_eq!(result2.stats.cache_misses, 0, "warm run should have 0 misses");
    }

    #[test]
    fn test_summarize_directory_processes_all_ts_files() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        sfs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();
        sfs::write(dir.path().join("b.ts"), "export const b = 2;").unwrap();
        sfs::write(dir.path().join("c.ts"), "export const c = 3;").unwrap();

        let nodes = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes.len(), 3);
        for node in &nodes {
            assert!(!node.id.as_str().is_empty(), "node id should be non-empty");
        }
    }

    #[test]
    fn test_summarize_directory_uses_cache_on_second_run() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        sfs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();

        let nodes1 = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes1.len(), 1);
        let id1 = nodes1[0].id.clone();

        let nodes2 = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes2.len(), 1);
        let id2 = nodes2[0].id.clone();

        assert_eq!(id1, id2, "second run should return same id from cache");
    }

    #[test]
    fn test_summarize_directory_skips_non_js_ts_files() {
        let dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        sfs::write(dir.path().join("a.ts"), "export const a = 1;").unwrap();
        sfs::write(dir.path().join("styles.css"), ".foo { color: red; }").unwrap();
        sfs::write(dir.path().join("readme.md"), "# README").unwrap();

        let nodes = summarize_directory(dir.path(), &cache).unwrap();
        assert_eq!(nodes.len(), 1, "only the .ts file should be processed");
    }
}
