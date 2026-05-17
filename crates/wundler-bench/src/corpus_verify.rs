//! Conformance verifier: re-measures a corpus directory and checks structural conformance.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use swc_core::common::{sync::Lrc, FileName, SourceMap};
use swc_core::ecma::parser::{Parser, StringInput, Syntax, TsSyntax};

use crate::profile::{
    check_schema_version, BenchProfile, Check, CheckStatus, ConformanceReport,
    PROFILE_SCHEMA_VERSION,
};
use crate::repo_scale::ExtensionStat;

/// Verify that `corpus_dir` conforms to `profile` within configured tolerances.
pub fn verify_corpus(profile: &BenchProfile, corpus_dir: &Path) -> Result<ConformanceReport> {
    check_schema_version(profile)?;
    let measured = measure_corpus(corpus_dir)?;

    let mut checks: Vec<Check> = Vec::new();
    let tol = &profile.gen.tolerances;

    // V1: per-extension structural checks.
    for (ext, target) in &profile.target.workspace_stats.extension_stats {
        let actual = measured.get(ext).cloned().unwrap_or_default();

        checks.push(within_tol(
            format!("ext[{ext}].file_count"),
            target.file_count as f64,
            actual.file_count as f64,
            tol.file_count_pct,
        ));
        checks.push(within_tol(
            format!("ext[{ext}].total_bytes"),
            target.total_bytes as f64,
            actual.total_bytes as f64,
            tol.byte_count_pct,
        ));
        checks.push(within_tol(
            format!("ext[{ext}].total_lines"),
            target.total_lines as f64,
            actual.total_lines as f64,
            tol.line_count_pct,
        ));
    }

    // V3: SWC parse sample on .ts files.
    checks.push(ts_parse_sample(profile, corpus_dir)?);

    let passed = checks.iter().all(|c| c.status != CheckStatus::Fail);
    Ok(ConformanceReport {
        profile_schema_version: PROFILE_SCHEMA_VERSION,
        checks,
        passed,
    })
}

fn within_tol(name: String, target: f64, actual: f64, pct: f64) -> Check {
    if target == 0.0 {
        return Check {
            name,
            target,
            actual,
            tolerance_pct: pct,
            status: if actual == 0.0 {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
        };
    }
    let diff = (actual - target).abs() / target;
    let status = if diff <= pct {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Check { name, target, actual, tolerance_pct: pct, status }
}

fn measure_corpus(root: &Path) -> Result<HashMap<String, ExtensionStat>> {
    let mut out: HashMap<String, ExtensionStat> = HashMap::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)
            .with_context(|| format!("reading directory {}", d.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Skip hidden files (fingerprint, etc.)
            if file_name.starts_with('.') {
                continue;
            }
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };

            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading file {}", path.display()))?;
            let lines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
            let len = bytes.len() as u64;

            let e = out.entry(ext.to_string()).or_default();
            e.file_count += 1;
            e.total_bytes += len;
            e.total_lines += lines;
            e.avg_file_bytes = e.total_bytes as f64 / e.file_count as f64;
        }
    }
    Ok(out)
}

fn ts_parse_sample(profile: &BenchProfile, root: &Path) -> Result<Check> {
    let mut ts_files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)
            .with_context(|| format!("reading directory {}", d.display()))?
            .flatten()
        {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("ts") {
                ts_files.push(p);
            }
        }
    }
    ts_files.sort();

    if ts_files.is_empty() {
        return Ok(Check {
            name: "ts_parse_sample".into(),
            target: 0.0,
            actual: 0.0,
            tolerance_pct: 0.0,
            status: CheckStatus::Skipped,
        });
    }

    let sample_size = if ts_files.len() <= 20 {
        ts_files.len()
    } else {
        ((ts_files.len() as f64) * 0.01).ceil() as usize
    };

    let mut rng = StdRng::seed_from_u64(profile.gen.seed ^ 0xA5A5_A5A5_A5A5_A5A5);
    let mut indices: Vec<usize> = (0..ts_files.len()).collect();
    for i in 0..sample_size {
        let j = i + rng.gen_range(0..(ts_files.len() - i));
        indices.swap(i, j);
    }

    let mut ok = 0u64;
    let total = sample_size as u64;
    for &idx in indices.iter().take(sample_size) {
        let src = std::fs::read_to_string(&ts_files[idx])
            .with_context(|| format!("reading TS file {}", ts_files[idx].display()))?;
        if parse_ts(&src) {
            ok += 1;
        }
    }

    let status = if ok == total {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    };
    Ok(Check {
        name: "ts_parse_sample".into(),
        target: total as f64,
        actual: ok as f64,
        tolerance_pct: 0.0,
        status,
    })
}

fn parse_ts(source: &str) -> bool {
    let cm: Lrc<SourceMap> = Default::default();
    let fm = cm.new_source_file(Lrc::new(FileName::Anon), source.to_string());
    let mut p = Parser::new(
        Syntax::Typescript(TsSyntax::default()),
        StringInput::from(&*fm),
        None,
    );
    p.parse_module().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus_gen::generate_corpus;
    use crate::profile::{BenchProfile, CheckStatus, GenHints, Tolerances, PROFILE_SCHEMA_VERSION};
    use crate::repo_scale::{
        DirectoryStat, ExtensionStat, GitStats, ManifestStats, RepoScaleReport, WorkspaceStats,
    };
    use std::collections::HashMap;

    fn tiny_profile() -> BenchProfile {
        let mut ext = HashMap::new();
        ext.insert(
            "ts".to_string(),
            ExtensionStat {
                file_count: 10,
                total_bytes: 4_000,
                total_lines: 50,
                avg_file_bytes: 400.0,
            },
        );
        ext.insert(
            "json".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_000,
                total_lines: 25,
                avg_file_bytes: 200.0,
            },
        );
        ext.insert(
            "md".to_string(),
            ExtensionStat {
                file_count: 5,
                total_bytes: 1_500,
                total_lines: 15,
                avg_file_bytes: 300.0,
            },
        );

        BenchProfile {
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            target: RepoScaleReport {
                git_stats: GitStats::default(),
                workspace_stats: WorkspaceStats {
                    total_files: 20,
                    total_bytes: 6_500,
                    extension_stats: ext,
                    directory_stats: vec![
                        DirectoryStat { path: "src".into(), file_count: 15 },
                        DirectoryStat { path: "docs".into(), file_count: 5 },
                    ],
                    packages: vec!["src".into()],
                },
                manifest_stats: ManifestStats::default(),
            },
            gen: GenHints {
                seed: 7,
                tolerances: Tolerances {
                    file_count_pct: 0.02,
                    byte_count_pct: 0.05,
                    line_count_pct: 0.30,
                },
            },
        }
    }

    #[test]
    fn freshly_generated_corpus_passes() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();
        let report = verify_corpus(&p, dir.path()).unwrap();
        assert!(report.passed, "checks: {:#?}", report.checks);
        for c in &report.checks {
            assert_ne!(c.status, CheckStatus::Fail, "{c:?}");
        }
    }

    #[test]
    fn corrupted_corpus_fails_file_count_check() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();

        let mut deleted = 0;
        for entry in std::fs::read_dir(dir.path().join("src")).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ts") && deleted < 8 {
                std::fs::remove_file(&path).unwrap();
                deleted += 1;
            }
        }

        let report = verify_corpus(&p, dir.path()).unwrap();
        assert!(!report.passed);
        assert!(report.checks.iter().any(|c| {
            c.name.contains("ts") && c.name.contains("file_count") && c.status == CheckStatus::Fail
        }));
    }

    #[test]
    fn ts_parse_sample_passes_on_clean_corpus() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();
        let report = verify_corpus(&p, dir.path()).unwrap();
        let parse_check = report
            .checks
            .iter()
            .find(|c| c.name == "ts_parse_sample")
            .expect("expected ts_parse_sample check");
        assert_eq!(parse_check.status, CheckStatus::Pass);
    }

    #[test]
    fn ts_parse_sample_fails_when_file_is_broken_syntax() {
        let dir = tempfile::tempdir().unwrap();
        let p = tiny_profile();
        generate_corpus(&p, dir.path()).unwrap();

        for entry in std::fs::read_dir(dir.path().join("src")).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("ts") {
                std::fs::write(&path, "this is (((( not valid TS @@@@").unwrap();
                break;
            }
        }

        let report = verify_corpus(&p, dir.path()).unwrap();
        let parse_check = report
            .checks
            .iter()
            .find(|c| c.name == "ts_parse_sample")
            .expect("expected ts_parse_sample check");
        assert_eq!(parse_check.status, CheckStatus::Fail);
        assert!(!report.passed);
    }
}
