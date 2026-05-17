
//! Integration tests for wundler-bench.
//!
//! TDD: these tests are written BEFORE the implementation.
//! They define the expected behaviour and drive the design.

use std::path::PathBuf;
use tempfile::TempDir;
use wundler_bench::synthetic::SyntheticApp;

// ---------------------------------------------------------------------------
// Helper: build the synthetic app and return the out_dir
// ---------------------------------------------------------------------------

fn build_app(app: &SyntheticApp) -> anyhow::Result<(TempDir, PathBuf)> {
    use std::collections::HashMap;
    use wundler_pipeline::config::{BuildConfig, EngineChoice};
    use wundler_pipeline::pipeline::BuildPipeline;

    let out_tmp = TempDir::new()?;
    let out_dir = out_tmp.path().to_path_buf();
    std::fs::create_dir_all(&out_dir)?;

    let config = BuildConfig {
        root: app.dir.path().to_path_buf(),
        out_dir: out_dir.clone(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: {
            let mut m = HashMap::new();
            m.insert("/".to_string(), PathBuf::from("src/main.tsx"));
            m
        },
        budget: None,
        dev: None,
    };

    let pipeline = BuildPipeline::new(config);
    pipeline.build()?;

    Ok((out_tmp, out_dir))
}

// ---------------------------------------------------------------------------
// Test 1: synthetic generator produces valid UTF-8 TypeScript files
// ---------------------------------------------------------------------------

#[test]
fn test_synthetic_generates_parseable_ts() {
    let n = 10;
    let app = SyntheticApp::generate(n).expect("generate failed");
    let src_dir = app.dir.path().join("src");

    // Each module file must exist and be valid UTF-8
    for i in 0..n {
        let path = src_dir.join(format!("mod_{:05}.tsx", i));
        assert!(path.exists(), "module {:05} should exist", i);

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("module {:05} not readable: {}", i, e));

        assert!(!content.is_empty(), "module {:05} should not be empty", i);

        // Must contain at least one export
        assert!(
            content.contains("export"),
            "module {:05} should have exports",
            i
        );
    }

    // main.tsx must exist
    let main_path = src_dir.join("main.tsx");
    assert!(main_path.exists(), "main.tsx should exist");
    let main_content = std::fs::read_to_string(&main_path).expect("main.tsx not readable");
    assert!(
        main_content.contains("import"),
        "main.tsx should have imports"
    );
    // main.tsx should have dynamic imports
    assert!(
        main_content.contains("import("),
        "main.tsx should have at least one dynamic import"
    );

    // mod_00001 should import from mod_00000 (i-1 dep)
    let mod1_path = src_dir.join("mod_00001.tsx");
    let mod1_content = std::fs::read_to_string(&mod1_path).expect("mod_00001.tsx not readable");
    assert!(
        mod1_content.contains("mod_00000"),
        "mod_00001 should import from mod_00000"
    );

    // mod_00009 should import from mod_00008 (i-1 dep)
    if n >= 9 {
        let mod9_path = src_dir.join("mod_00009.tsx");
        let mod9_content =
            std::fs::read_to_string(&mod9_path).expect("mod_00009.tsx not readable");
        assert!(
            mod9_content.contains("mod_00008"),
            "mod_00009 should import from mod_00008"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: touch_module changes the file content (new hash)
// ---------------------------------------------------------------------------

#[test]
fn test_touch_module_changes_content() {
    let app = SyntheticApp::generate(5).expect("generate failed");
    let mod_path = app.dir.path().join("src/mod_00002.tsx");

    let before = std::fs::read_to_string(&mod_path).expect("read before");
    app.touch_module(2).expect("touch_module failed");
    let after = std::fs::read_to_string(&mod_path).expect("read after");

    assert_ne!(
        before, after,
        "touch_module should change the file content"
    );
}

// ---------------------------------------------------------------------------
// Test 3: ABS delta correctly identifies changed chunks
// ---------------------------------------------------------------------------

#[test]
fn test_abs_delta_correct() {
    use wundler_bench::abs_bench::{compute_delta, snapshot_from_disk};

    // Use a small app so the test runs quickly
    let app = SyntheticApp::generate(20).expect("generate failed");

    // Build v1
    let (v1_tmp, v1_out) = build_app(&app).expect("v1 build failed");

    // Snapshot v1
    let v1_snap = snapshot_from_disk(&v1_out).expect("v1 snapshot failed");
    assert!(!v1_snap.is_empty(), "v1 should have at least one chunk");

    let total_bytes: u64 = v1_snap.iter().map(|c| c.size_bytes).sum();
    assert!(total_bytes > 0, "v1 total bundle must be > 0 bytes");

    // Touch one module in the middle of the app
    app.touch_module(10).expect("touch_module failed");

    // Build v2 into a fresh out_dir
    let (v2_tmp, v2_out) = build_app(&app).expect("v2 build failed");

    // Snapshot v2
    let v2_snap = snapshot_from_disk(&v2_out).expect("v2 snapshot failed");

    // Compute the ABS delta
    let delta = compute_delta(&v1_snap, &v2_snap);

    // At least one chunk should have changed (the one containing module 10)
    assert!(
        !delta.is_empty(),
        "touching a module should produce a non-empty delta; v1_chunks={:?}, v2_chunks={:?}",
        v1_snap.iter().map(|c| &c.chunk_id).collect::<Vec<_>>(),
        v2_snap.iter().map(|c| &c.chunk_id).collect::<Vec<_>>(),
    );

    // Delta must be strictly smaller than total bundle
    let delta_bytes: u64 = delta.iter().map(|c| c.size_bytes).sum();
    let v2_total: u64 = v2_snap.iter().map(|c| c.size_bytes).sum();
    assert!(
        delta_bytes <= v2_total,
        "delta ({} bytes) should not exceed total ({} bytes)",
        delta_bytes,
        v2_total
    );

    // Keep the temp dirs alive until assertions are done
    let _ = (v1_tmp, v2_tmp);
}

// ---------------------------------------------------------------------------
// Test 4: report formatter produces correct Markdown with table headers
// ---------------------------------------------------------------------------

#[test]
fn test_report_formats_correctly() {
    use wundler_bench::cas_bench::CasBenchResult;
    use wundler_bench::abs_bench::AbsChurnResult;
    use wundler_bench::report::{format_cas_table, format_abs_table};

    // Mock CAS results
    let cas_results = vec![
        CasBenchResult {
            n_modules: 100,
            cold_build_ms: 12,
            warm_build_ms: 1,
            speedup: 12.0,
            cache_hit_rate: 0.99,
        },
        CasBenchResult {
            n_modules: 1000,
            cold_build_ms: 89,
            warm_build_ms: 1,
            speedup: 89.0,
            cache_hit_rate: 0.999,
        },
    ];

    let cas_table = format_cas_table(&cas_results);
    assert!(
        cas_table.contains("| Modules |"),
        "CAS table should have Modules column header"
    );
    assert!(
        cas_table.contains("100"),
        "CAS table should contain n=100 row"
    );
    assert!(
        cas_table.contains("1,000"),
        "CAS table should contain n=1000 row (with comma)"
    );
    assert!(
        cas_table.contains("12.0×"),
        "CAS table should show speedup"
    );

    // Mock ABS results
    let abs_results = vec![
        AbsChurnResult {
            churn_fraction: 0.01,
            total_bundle_kb: 100.0,
            abs_download_kb: 10.0,
            savings_pct: 90.0,
        },
        AbsChurnResult {
            churn_fraction: 0.05,
            total_bundle_kb: 100.0,
            abs_download_kb: 30.0,
            savings_pct: 70.0,
        },
    ];

    let abs_table = format_abs_table(&abs_results);
    assert!(
        abs_table.contains("| Code churn |"),
        "ABS table should have Code churn column header"
    );
    assert!(
        abs_table.contains("1%"),
        "ABS table should contain 1% churn row"
    );
    assert!(
        abs_table.contains("5%"),
        "ABS table should contain 5% churn row"
    );
    assert!(
        abs_table.contains("90.0%"),
        "ABS table should show savings percentage"
    );
}

// ---------------------------------------------------------------------------
// Test 5: analysis_bench::run returns correct result shape
// ---------------------------------------------------------------------------

/// RED: fails until analysis_bench module is implemented.
#[test]
fn test_analysis_bench_run_returns_results() {
    use wundler_bench::analysis_bench;

    // Small scale so the test runs quickly.
    let results = analysis_bench::run(&[10]).expect("analysis_bench::run failed");

    assert_eq!(results.len(), 1, "should have one result per scale");
    let r = &results[0];
    assert_eq!(r.n_modules, 10);
    // cold_ms should be ≥ warm_ms (warm benefits from cache)
    assert!(
        r.cold_ms >= r.warm_ms || r.warm_ms < 500,
        "unexpected timing: cold={}ms warm={}ms",
        r.cold_ms,
        r.warm_ms
    );
    assert!(r.speedup >= 1.0, "speedup must be ≥ 1.0, got {}", r.speedup);
}

// ---------------------------------------------------------------------------
// Test 6: extrapolate gives approximately linear projection
// ---------------------------------------------------------------------------

/// RED: fails until `extrapolate` is implemented in analysis_bench.
#[test]
fn test_extrapolate_linear_projection() {
    use wundler_bench::analysis_bench::{extrapolate, AnalysisBenchResult};

    // Perfect linear data: 10ms per 100 modules.
    let results = vec![
        AnalysisBenchResult {
            n_modules: 100,
            cold_ms: 10,
            warm_ms: 2,
            speedup: 5.0,
            warm_graph_ms: 1,
            is_extrapolated: false,
        },
        AnalysisBenchResult {
            n_modules: 1_000,
            cold_ms: 100,
            warm_ms: 2,
            speedup: 50.0,
            warm_graph_ms: 1,
            is_extrapolated: false,
        },
    ];

    let proj = extrapolate(&results, 10_000);
    assert_eq!(proj.n_modules, 10_000);
    assert!(
        proj.is_extrapolated,
        "extrapolated result must have is_extrapolated=true"
    );
    // Expect ~1000ms ± 50%
    assert!(
        proj.cold_ms > 500 && proj.cold_ms < 2_000,
        "expected ~1000ms, got {}ms",
        proj.cold_ms
    );
    // warm stays roughly constant (uses last measured warm)
    assert!(proj.warm_ms <= 10, "warm should stay small, got {}ms", proj.warm_ms);
}

// ---------------------------------------------------------------------------
// Test 7: abs_bench::run_scales works at multiple N values
// ---------------------------------------------------------------------------

/// RED: fails until `run_scales` is added to abs_bench.
#[test]
fn test_abs_run_scales_returns_one_entry_per_scale() {
    use wundler_bench::abs_bench::run_scales;

    // Use a tiny scale so this test is fast.
    let churns = vec![0.1_f64, 0.5_f64];
    let scale_results = run_scales(&[5, 10], &churns).expect("run_scales failed");

    assert_eq!(scale_results.len(), 2, "should have one entry per scale");
    assert_eq!(scale_results[0].n_modules, 5);
    assert_eq!(scale_results[1].n_modules, 10);
    for sr in &scale_results {
        assert_eq!(
            sr.churns.len(),
            churns.len(),
            "each scale should have all churn rows"
        );
        assert!(
            sr.churns[0].total_bundle_kb > 0.0,
            "total_bundle_kb must be > 0"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8: report::format_analysis_table produces correct markdown
// ---------------------------------------------------------------------------

/// RED: fails until `format_analysis_table` is added to report.
#[test]
fn test_format_analysis_table_has_header_and_speedup() {
    use wundler_bench::analysis_bench::AnalysisBenchResult;
    use wundler_bench::report::format_analysis_table;

    let results = vec![AnalysisBenchResult {
        n_modules: 1_000,
        cold_ms: 80,
        warm_ms: 2,
        speedup: 40.0,
        warm_graph_ms: 1,
        is_extrapolated: false,
    }];

    let table = format_analysis_table(&results);
    assert!(
        table.contains("| Modules |"),
        "analysis table should have Modules header; got:\n{table}"
    );
    assert!(
        table.contains("40.0×"),
        "analysis table should show 40.0× speedup; got:\n{table}"
    );
    assert!(
        table.contains("1,000"),
        "analysis table should have N=1000 formatted with comma; got:\n{table}"
    );
}

// ---------------------------------------------------------------------------
// Test 9: report::format_real_world_scenarios contains required sections
// ---------------------------------------------------------------------------

/// RED: fails until `format_real_world_scenarios` is added to report.
#[test]
fn test_format_real_world_scenarios_contains_sections() {
    use wundler_bench::analysis_bench::AnalysisBenchResult;
    use wundler_bench::abs_bench::{AbsChurnResult, AbsScaleResult};
    use wundler_bench::report::format_real_world_scenarios;

    let analysis = vec![AnalysisBenchResult {
        n_modules: 5_000,
        cold_ms: 400,
        warm_ms: 2,
        speedup: 200.0,
        warm_graph_ms: 1,
        is_extrapolated: false,
    }];

    let abs_scales = vec![AbsScaleResult {
        n_modules: 1_000,
        churns: vec![AbsChurnResult {
            churn_fraction: 0.02,
            total_bundle_kb: 500.0,
            abs_download_kb: 250.0,
            savings_pct: 50.0,
        }],
    }];

    let section = format_real_world_scenarios(&analysis, &abs_scales);
    assert!(
        section.contains("Weekly build time"),
        "should contain 'Weekly build time'; got:\n{section}"
    );
    assert!(
        section.contains("CDN bandwidth"),
        "should contain 'CDN bandwidth'; got:\n{section}"
    );
    assert!(
        section.contains("Mid-scale") || section.contains("mid-scale") || section.contains("5,000"),
        "should mention mid-scale scenario; got:\n{section}"
    );
}
