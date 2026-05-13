// Validation module — module graph validation utilities.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use rayon::prelude::*;
use walkdir::WalkDir;

use crate::cache::local::LocalCache;
use crate::summarizer::ModuleSummarizer;
use crate::types::BundleGraphNode;

const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// Statistics reported by [`run_validate_scale`].
#[derive(Debug)]
pub struct ValidateScaleStats {
    pub module_count: usize,
    pub total_cache_bytes: u64,
    pub p50_micros: u64,
    pub p95_micros: u64,
    pub dead_code_estimate_pct: f64,
}

/// Compute the p-th percentile of a sorted slice.
///
/// Returns `0` if the slice is empty.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() * p / 100).min(sorted.len() - 1);
    sorted[idx]
}

/// Walk `dir` for JS/TS files, summarize each one in parallel, populate the
/// cache, and return aggregate performance and quality statistics.
pub fn run_validate_scale(dir: &Path, cache: &LocalCache) -> Result<ValidateScaleStats> {
    // Collect JS/TS file paths via walkdir (same extensions as the summarizer).
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

    let module_count = paths.len();

    // Per-file timings and successfully processed nodes, collected via Mutexes
    // so they can be populated from the rayon thread pool.
    let timings: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    let nodes: Mutex<Vec<BundleGraphNode>> = Mutex::new(Vec::new());

    paths.par_iter().for_each(|path| {
        let summarizer = ModuleSummarizer::new();
        let t0 = Instant::now();
        if let Ok(node) = summarizer.summarize(path) {
            let elapsed = t0.elapsed().as_micros() as u64;
            let id = node.id.clone();
            let summary = node.summary.clone();

            if let Ok(mut t) = timings.lock() {
                t.push(elapsed);
            }
            // Best-effort cache write; ignore errors to keep the run going.
            let _ = cache.put(&id, &summary);

            if let Ok(mut n) = nodes.lock() {
                n.push(node);
            }
        }
    });

    let mut timings_micros = timings.into_inner().unwrap_or_default();
    timings_micros.sort_unstable();

    let p50_micros = percentile(&timings_micros, 50);
    let p95_micros = percentile(&timings_micros, 95);

    let total_cache_bytes = cache.total_bytes()?;

    // Dead-code estimate:
    //   - all_exported: every export name seen across all nodes
    //   - all_imported: every import binding seen across all nodes
    //   - referenced   = |intersection|
    //   - dead_pct     = (1 - referenced / all_exported.len()) * 100
    let collected_nodes = nodes.into_inner().unwrap_or_default();

    let mut all_exported: HashSet<String> = HashSet::new();
    let mut all_imported: HashSet<String> = HashSet::new();

    for node in &collected_nodes {
        for export in &node.summary.exports {
            all_exported.insert(export.name.clone());
        }
        for import in &node.summary.imports {
            for binding in &import.bindings {
                all_imported.insert(binding.clone());
            }
        }
    }

    let referenced = all_exported.intersection(&all_imported).count();
    let dead_code_estimate_pct = if all_exported.is_empty() {
        0.0
    } else {
        (1.0 - referenced as f64 / all_exported.len() as f64) * 100.0
    };

    Ok(ValidateScaleStats {
        module_count,
        total_cache_bytes,
        p50_micros,
        p95_micros,
        dead_code_estimate_pct,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_ts_module(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn test_validate_scale_counts_modules_and_reports_cache_size() {
        let src_dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = crate::cache::local::LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        // Write 5 TypeScript modules with distinct exports so the summarizer has something to parse.
        make_ts_module(src_dir.path(), "a.ts", "export const a = 1;");
        make_ts_module(src_dir.path(), "b.ts", "export const b = 2;");
        make_ts_module(src_dir.path(), "c.ts", "export const c = 3;");
        make_ts_module(src_dir.path(), "d.ts", "export const d = 4;");
        make_ts_module(src_dir.path(), "e.ts", "export const e = 5;");

        let stats = run_validate_scale(src_dir.path(), &cache).unwrap();

        assert_eq!(stats.module_count, 5, "expected 5 modules");
        assert!(
            stats.total_cache_bytes > 0,
            "cache should contain data after processing"
        );
        assert!(stats.p50_micros > 0, "p50 timing should be > 0");
        assert!(
            stats.p95_micros >= stats.p50_micros,
            "p95 ({}) must be >= p50 ({})",
            stats.p95_micros,
            stats.p50_micros
        );
    }

    #[test]
    fn test_validate_scale_dead_code_estimate_is_bounded() {
        let src_dir = TempDir::new().unwrap();
        let cache_dir = TempDir::new().unwrap();
        let cache = crate::cache::local::LocalCache::new(cache_dir.path().to_path_buf()).unwrap();

        // Some exports, some imports — exercises the intersection logic.
        make_ts_module(
            src_dir.path(),
            "lib.ts",
            "export const foo = 1; export const bar = 2;",
        );
        make_ts_module(
            src_dir.path(),
            "app.ts",
            "import { foo } from './lib'; console.log(foo);",
        );

        let stats = run_validate_scale(src_dir.path(), &cache).unwrap();

        assert!(
            stats.dead_code_estimate_pct >= 0.0,
            "dead_code_estimate_pct must be >= 0.0, got {}",
            stats.dead_code_estimate_pct
        );
        assert!(
            stats.dead_code_estimate_pct <= 100.0,
            "dead_code_estimate_pct must be <= 100.0, got {}",
            stats.dead_code_estimate_pct
        );
    }
}
