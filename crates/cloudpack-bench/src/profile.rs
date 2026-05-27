//! BenchProfile contract — declarative, versioned shape spec for synthetic corpora.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::repo_scale::RepoScaleReport;

/// Bumped only on breaking changes to the on-disk JSON shape.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchProfile {
    pub profile_schema_version: u32,
    pub target: RepoScaleReport,
    pub gen: GenHints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenHints {
    pub seed: u64,
    #[serde(default)]
    pub tolerances: Tolerances,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tolerances {
    pub file_count_pct: f64,
    pub byte_count_pct: f64,
    pub line_count_pct: f64,
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            file_count_pct: 0.02,
            byte_count_pct: 0.05,
            line_count_pct: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformanceReport {
    pub profile_schema_version: u32,
    pub checks: Vec<Check>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub target: f64,
    pub actual: f64,
    pub tolerance_pct: f64,
    pub status: CheckStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skipped,
}

/// Returns `Ok(())` iff `profile.profile_schema_version <= PROFILE_SCHEMA_VERSION`.
pub fn check_schema_version(profile: &BenchProfile) -> Result<()> {
    if profile.profile_schema_version > PROFILE_SCHEMA_VERSION {
        return Err(anyhow!(
            "profile schema version {} is newer than supported maximum {} — upgrade cloudpack-bench",
            profile.profile_schema_version,
            PROFILE_SCHEMA_VERSION
        ));
    }
    Ok(())
}

/// Load a profile from disk and validate its schema version.
pub fn load_profile(path: &Path) -> Result<BenchProfile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading profile from {}", path.display()))?;
    let profile: BenchProfile = serde_json::from_str(&text)
        .with_context(|| format!("parsing profile JSON at {}", path.display()))?;
    check_schema_version(&profile)?;
    Ok(profile)
}

/// Write a profile to disk as pretty-printed JSON.
pub fn save_profile(profile: &BenchProfile, path: &Path) -> Result<()> {
    let text = serde_json::to_string_pretty(profile)?;
    std::fs::write(path, text)
        .with_context(|| format!("writing profile to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_scale::{
        DirectoryStat, ExtensionStat, GitStats, ManifestStats, RepoScaleReport, WorkspaceStats,
    };
    use std::collections::HashMap;

    fn sample_profile() -> BenchProfile {
        let mut ext = HashMap::new();
        ext.insert(
            "ts".to_string(),
            ExtensionStat {
                file_count: 10,
                total_bytes: 10_000,
                total_lines: 500,
                avg_file_bytes: 1_000.0,
            },
        );

        BenchProfile {
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            target: RepoScaleReport {
                git_stats: GitStats::default(),
                workspace_stats: WorkspaceStats {
                    total_files: 10,
                    total_bytes: 10_000,
                    extension_stats: ext,
                    directory_stats: vec![DirectoryStat {
                        path: "src".to_string(),
                        file_count: 10,
                    }],
                    packages: vec!["src".to_string()],
                },
                manifest_stats: ManifestStats::default(),
            },
            gen: GenHints {
                seed: 42,
                tolerances: Tolerances::default(),
            },
        }
    }

    #[test]
    fn round_trip_json() {
        let p = sample_profile();
        let json = serde_json::to_string(&p).unwrap();
        let back: BenchProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.profile_schema_version, PROFILE_SCHEMA_VERSION);
        assert_eq!(back.gen.seed, 42);
        assert_eq!(back.target.workspace_stats.total_files, 10);
    }

    #[test]
    fn default_tolerances() {
        let t = Tolerances::default();
        assert!((t.file_count_pct - 0.02).abs() < f64::EPSILON);
        assert!((t.byte_count_pct - 0.05).abs() < f64::EPSILON);
        assert!((t.line_count_pct - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn schema_version_gate_accepts_current() {
        let p = sample_profile();
        assert!(check_schema_version(&p).is_ok());
    }

    #[test]
    fn schema_version_gate_rejects_future() {
        let mut p = sample_profile();
        p.profile_schema_version = PROFILE_SCHEMA_VERSION + 1;
        let err = check_schema_version(&p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("schema"), "msg was: {msg}");
    }

    #[test]
    fn check_status_serializes_snake_case() {
        let s = serde_json::to_string(&CheckStatus::Pass).unwrap();
        assert_eq!(s, "\"pass\"");
        let s = serde_json::to_string(&CheckStatus::Skipped).unwrap();
        assert_eq!(s, "\"skipped\"");
    }
}
