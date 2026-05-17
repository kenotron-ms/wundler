
//! Deterministic repo-scale profiler.
//!
//! Measures structural scale of an arbitrary git repository using git metadata
//! and filesystem byte/line counts.  Output is a bounded summary (counts +
//! top-N aggregates), not file lists.
//!
//! # Usage
//!
//! ```no_run
//! use std::path::PathBuf;
//! use wundler_bench::repo_scale::{measure_repo, render_markdown, RepoScaleOptions};
//!
//! let opts = RepoScaleOptions {
//!     path: PathBuf::from("/path/to/repo"),
//!     ..Default::default()
//! };
//! let report = measure_repo(&opts).unwrap();
//! println!("{}", render_markdown(&report));
//! ```

use std::collections::{HashMap, HashSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

// ── Public constants ──────────────────────────────────────────────────────────

/// Schema version embedded in every [`RepoScaleReport`].
pub const SCHEMA_VERSION: u32 = 1;

/// Directories to skip when walking untracked files.
const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
    "out",
    ".venv",
    "__pycache__",
];

// ── Public types ──────────────────────────────────────────────────────────────

/// Options controlling what is measured and how.
#[derive(Debug, Clone)]
pub struct RepoScaleOptions {
    /// Root of the repository to measure.
    pub path: PathBuf,
    /// How many top-level directories to include in the report.
    pub top_dirs: usize,
    /// How many extensions to include in the report.
    pub top_extensions: usize,
    /// When `true`, filesystem-walk finds files not tracked by git
    /// (excluding [`EXCLUDED_DIRS`]).
    pub include_untracked: bool,
}

impl Default for RepoScaleOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::from("."),
            top_dirs: 20,
            top_extensions: 30,
            include_untracked: false,
        }
    }
}

/// Top-level measurement result; also used as the profile target shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RepoScaleReport {
    pub git_stats: GitStats,
    pub workspace_stats: WorkspaceStats,
    pub manifest_stats: ManifestStats,
}

/// Git-level statistics extracted without checking out anything extra.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GitStats {
    pub is_git_repo: bool,
    pub head_commit: Option<String>,
    pub current_branch: Option<String>,
    pub commit_count: Option<u64>,
    pub files_ever_tracked: Option<u64>,
}

/// Workspace-level aggregate statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct WorkspaceStats {
    /// Total number of files in the workspace.
    pub total_files: u64,
    /// Total byte count across all files.
    pub total_bytes: u64,
    /// Per-extension aggregate counts, keyed by lowercase extension (no leading dot).
    pub extension_stats: HashMap<String, ExtensionStat>,
    /// Per-top-level-directory file counts, sorted by file count descending.
    pub directory_stats: Vec<DirectoryStat>,
    /// Workspace package directory names (from `packages/` or `apps/`).
    pub packages: Vec<String>,
}

/// Per-extension aggregate counts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ExtensionStat {
    pub file_count: u64,
    pub total_bytes: u64,
    pub total_lines: u64,
    pub avg_file_bytes: f64,
}

/// Per-top-level-directory aggregate counts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DirectoryStat {
    /// First path component, or `"<root>"` for files at the repo root.
    pub path: String,
    pub file_count: u64,
}

/// Count of key manifest files found anywhere in the tracked file list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManifestStats {
    pub package_json: u64,
    pub cargo_toml: u64,
    pub pyproject_toml: u64,
    pub tsconfig_json: u64,
    /// `.yml` / `.yaml` files located anywhere under `azure/`.
    pub azure_pipelines_yaml: u64,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Measure the repository described by `opts`, stamping the report with the
/// current UTC time.
pub fn measure_repo(opts: &RepoScaleOptions) -> io::Result<RepoScaleReport> {
    let now = current_utc_timestamp();
    measure_repo_with_clock(opts, &now)
}

/// Like [`measure_repo`] but accepts an explicit timestamp string, enabling
/// deterministic output in tests.
pub fn measure_repo_with_clock(opts: &RepoScaleOptions, _now_utc: &str) -> io::Result<RepoScaleReport> {
    let path = &opts.path;
    let mut warnings: Vec<String> = Vec::new();

    // ── 1. Git statistics ──────────────────────────────────────────────────
    let git_stats = collect_git_stats(path, &mut warnings);

    // ── 2. Build the file list ─────────────────────────────────────────────
    let mut files: Vec<String> = if git_stats.is_git_repo {
        get_tracked_files(path, &mut warnings)?
    } else {
        Vec::new()
    };

    if opts.include_untracked {
        let tracked_set: HashSet<String> = files.iter().cloned().collect();
        let untracked = walk_untracked(path, &tracked_set);
        files.extend(untracked);
    }

    // ── 3. Warnings ────────────────────────────────────────────────────────
    if files
        .iter()
        .any(|f| f.split('/').any(|seg| seg == "node_modules"))
    {
        warnings.push(
            "Warning: node_modules files found in tracked files list. \
             Consider adding node_modules to .gitignore."
                .to_string(),
        );
    }

    let total = files.len() as u64;
    if total > 100_000 {
        warnings.push(format!(
            "Warning: total file count ({total}) exceeds 100,000"
        ));
    }

    // ── 4. Aggregate stats ─────────────────────────────────────────────────
    let workspace_stats =
        compute_workspace_stats(path, &files, opts.top_dirs, &mut warnings);
    let manifest_stats = compute_manifest_stats(&files);

    Ok(RepoScaleReport {
        git_stats,
        workspace_stats,
        manifest_stats,
    })
}

/// Render `report` as pretty-printed JSON.
pub fn render_json(report: &RepoScaleReport) -> String {
    serde_json::to_string_pretty(report).expect("RepoScaleReport must always be serialisable")
}

/// Render `report` as GitHub-flavoured Markdown.
pub fn render_markdown(report: &RepoScaleReport) -> String {
    let mut s = String::with_capacity(4096);

    // Title & metadata
    s.push_str("# Repo Scale Report\n\n");

    // Git section
    s.push_str("## Git\n\n");
    s.push_str(&format!(
        "- Is git repo: {}  \n",
        report.git_stats.is_git_repo
    ));
    s.push_str(&format!(
        "- HEAD commit: {}  \n",
        report.git_stats.head_commit.as_deref().unwrap_or("—")
    ));
    s.push_str(&format!(
        "- Branch: {}  \n",
        report.git_stats.current_branch.as_deref().unwrap_or("—")
    ));
    s.push_str(&format!(
        "- Commit count: {}  \n",
        fmt_opt_u64(report.git_stats.commit_count)
    ));
    s.push_str(&format!(
        "- Files ever tracked: {}  \n\n",
        fmt_opt_u64(report.git_stats.files_ever_tracked)
    ));

    // Workspace overview
    s.push_str("## Workspace\n\n");
    s.push_str(&format!(
        "- Total files: {}  \n",
        fmt_n(report.workspace_stats.total_files)
    ));
    s.push_str(&format!(
        "- Total bytes: {}  \n\n",
        fmt_bytes(report.workspace_stats.total_bytes)
    ));

    // Top extensions
    s.push_str("## Top extensions\n\n");
    if report.workspace_stats.extension_stats.is_empty() {
        s.push_str("*(none)*\n\n");
    } else {
        s.push_str("| Extension | Files | Lines | Bytes |\n");
        s.push_str("|-----------|------:|------:|------:|\n");
        let mut exts: Vec<(&String, &ExtensionStat)> =
            report.workspace_stats.extension_stats.iter().collect();
        exts.sort_by(|a, b| {
            b.1.file_count
                .cmp(&a.1.file_count)
                .then_with(|| a.0.cmp(b.0))
        });
        for (ext, stat) in &exts {
            let ext_label = if ext.is_empty() {
                "*(none)*".to_string()
            } else {
                ext.to_string()
            };
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                ext_label,
                fmt_n(stat.file_count),
                fmt_n(stat.total_lines),
                fmt_bytes(stat.total_bytes)
            ));
        }
        s.push('\n');
    }

    // Top directories
    s.push_str("## Top directories (depth=1)\n\n");
    if report.workspace_stats.directory_stats.is_empty() {
        s.push_str("*(none)*\n\n");
    } else {
        s.push_str("| Directory | Files |\n");
        s.push_str("|-----------|------:|\n");
        for d in &report.workspace_stats.directory_stats {
            s.push_str(&format!("| {} | {} |\n", d.path, fmt_n(d.file_count)));
        }
        s.push('\n');
    }

    // Workspace packages
    if !report.workspace_stats.packages.is_empty() {
        s.push_str("## Workspace packages\n\n");
        for p in &report.workspace_stats.packages {
            s.push_str(&format!("- {p}  \n"));
        }
        s.push('\n');
    }

    // Manifests
    s.push_str("## Manifests\n\n");
    s.push_str("| Type | Count |\n");
    s.push_str("|------|------:|\n");
    let m = &report.manifest_stats;
    s.push_str(&format!("| package.json | {} |\n", m.package_json));
    s.push_str(&format!("| Cargo.toml | {} |\n", m.cargo_toml));
    s.push_str(&format!("| pyproject.toml | {} |\n", m.pyproject_toml));
    s.push_str(&format!("| tsconfig.json | {} |\n", m.tsconfig_json));
    s.push_str(&format!(
        "| azure pipelines (.yml/.yaml) | {} |\n\n",
        m.azure_pipelines_yaml
    ));

    s
}

// ── Internal: git helpers ─────────────────────────────────────────────────────

fn run_git(path: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn collect_git_stats(path: &Path, warnings: &mut Vec<String>) -> GitStats {
    // Detect repo using `git rev-parse --is-inside-work-tree`.
    let is_git = run_git(path, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s.trim() == "true")
        .unwrap_or(false);

    if !is_git {
        warnings.push(format!(
            "Not a git repository (or git not available): {}",
            path.display()
        ));
        return GitStats {
            is_git_repo: false,
            head_commit: None,
            current_branch: None,
            commit_count: None,
            files_ever_tracked: None,
        };
    }

    let head_commit = run_git(path, &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());

    // `--abbrev-ref HEAD` returns "HEAD" in detached / no-commit state.
    let current_branch = run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s != "HEAD");

    let commit_count = run_git(path, &["rev-list", "--count", "HEAD"])
        .and_then(|s| s.trim().parse::<u64>().ok());

    let files_ever_tracked = count_files_ever_tracked(path);

    GitStats {
        is_git_repo: true,
        head_commit,
        current_branch,
        commit_count,
        files_ever_tracked,
    }
}

/// Count unique filenames ever added to the repo's history (including deleted ones).
///
/// `--root` is required so that the initial commit's files are included in the diff
/// (without it, the first commit has no parent and is not diffed, so its files are
/// invisible to `--diff-filter=A`).
///
/// `--diff-filter=AR` is used instead of `--diff-filter=A` alone because git
/// auto-detects renames (e.g. when a file is deleted and a new file with identical
/// content is added in the same commit, git records it as `R` not `A+D`).  The `R`
/// filter includes the *destination* filename of renames, so the unique set correctly
/// captures all distinct paths that were ever present in the repository.
fn count_files_ever_tracked(path: &Path) -> Option<u64> {
    let out = run_git(
        path,
        &[
            "log",
            "--all",
            "--root",
            "--pretty=format:",
            "--name-only",
            "--diff-filter=AR",
        ],
    )?;
    // Output contains blank lines between commits; filter those out.
    let unique: HashSet<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    Some(unique.len() as u64)
}

/// Return the NUL-separated list of tracked files (`git ls-files -z`).
fn get_tracked_files(path: &Path, _warnings: &mut Vec<String>) -> io::Result<Vec<String>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        // Empty repository (no commits yet) → ls-files exits non-zero.
        return Ok(Vec::new());
    }

    let files = output
        .stdout
        .split(|&b| b == 0)
        .filter(|seg| !seg.is_empty())
        .filter_map(|seg| String::from_utf8(seg.to_vec()).ok())
        .collect();

    Ok(files)
}

// ── Internal: untracked walk ──────────────────────────────────────────────────

fn walk_untracked(root: &Path, tracked: &HashSet<String>) -> Vec<String> {
    let mut result = Vec::new();
    walk_dir(root, root, tracked, &mut result);
    result
}

fn walk_dir(root: &Path, current: &Path, tracked: &HashSet<String>, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let full = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if EXCLUDED_DIRS.contains(&name_str.as_ref()) {
            continue;
        }

        if full.is_dir() {
            walk_dir(root, &full, tracked, out);
        } else if let Ok(rel) = full.strip_prefix(root) {
            // Normalise to forward-slash separators for cross-platform consistency.
            let rel_str = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if !tracked.contains(&rel_str) {
                out.push(rel_str);
            }
        }
    }
}

// ── Internal: aggregate stats ─────────────────────────────────────────────────

/// Build `WorkspaceStats` from the file list with a single filesystem pass.
fn compute_workspace_stats(
    repo_path: &Path,
    files: &[String],
    top_dirs: usize,
    warnings: &mut Vec<String>,
) -> WorkspaceStats {
    // Maps: extension → (file_count, line_count, byte_count)
    let mut ext_map: HashMap<String, (u64, u64, u64)> = HashMap::new();
    // Maps: top-level dir → file_count
    let mut dir_map: HashMap<String, u64> = HashMap::new();

    let mut total_bytes: u64 = 0;
    let mut unreadable: u64 = 0;

    for rel in files {
        let (lines, bytes) = match count_lines_and_bytes(&repo_path.join(rel)) {
            Ok(v) => v,
            Err(_) => {
                unreadable += 1;
                (0, 0)
            }
        };

        total_bytes += bytes;

        let ext = get_extension(rel);
        let dir = get_top_level_dir(rel);

        let e = ext_map.entry(ext).or_insert((0, 0, 0));
        e.0 += 1;
        e.1 += lines;
        e.2 += bytes;

        *dir_map.entry(dir).or_insert(0) += 1;
    }

    if unreadable > 0 {
        warnings.push(format!("{unreadable} file(s) could not be read"));
    }

    // ── Build extension_stats ─────────────────────────────────────────────
    let extension_stats: HashMap<String, ExtensionStat> = ext_map
        .into_iter()
        .map(|(ext, (fc, lc, bc))| {
            let avg = if fc > 0 { bc as f64 / fc as f64 } else { 0.0 };
            (
                ext,
                ExtensionStat {
                    file_count: fc,
                    total_bytes: bc,
                    total_lines: lc,
                    avg_file_bytes: avg,
                },
            )
        })
        .collect();

    // ── Build and sort directory_stats ────────────────────────────────────
    let mut directory_stats: Vec<DirectoryStat> = dir_map
        .into_iter()
        .map(|(path, fc)| DirectoryStat {
            path,
            file_count: fc,
        })
        .collect();
    directory_stats.sort_unstable_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then_with(|| a.path.cmp(&b.path))
    });
    directory_stats.truncate(top_dirs);

    // ── Build packages list ───────────────────────────────────────────────
    let file_set: HashSet<&str> = files.iter().map(|s| s.as_str()).collect();
    let mut packages: Vec<String> = Vec::new();
    for prefix in &["packages/", "apps/"] {
        let mut pkgs = collect_packages(files, &file_set, prefix);
        packages.append(&mut pkgs);
    }
    packages.sort();
    packages.dedup();

    WorkspaceStats {
        total_files: files.len() as u64,
        total_bytes,
        extension_stats,
        directory_stats,
        packages,
    }
}

/// Collect immediate sub-directory names under `prefix` that contain a `package.json`.
fn collect_packages(files: &[String], file_set: &HashSet<&str>, prefix: &str) -> Vec<String> {
    let mut pkgs: HashSet<String> = HashSet::new();
    for f in files {
        if let Some(rest) = f.strip_prefix(prefix) {
            if let Some(slash) = rest.find('/') {
                let name = &rest[..slash];
                let pkg_json_path = format!("{prefix}{name}/package.json");
                if file_set.contains(pkg_json_path.as_str()) {
                    pkgs.insert(name.to_string());
                }
            }
        }
    }
    pkgs.into_iter().collect()
}

fn compute_manifest_stats(files: &[String]) -> ManifestStats {
    let mut s = ManifestStats::default();
    for f in files {
        let basename = f.rsplit('/').next().unwrap_or(f.as_str());
        match basename {
            "package.json" => s.package_json += 1,
            "Cargo.toml" => s.cargo_toml += 1,
            "pyproject.toml" => s.pyproject_toml += 1,
            "tsconfig.json" => s.tsconfig_json += 1,
            _ => {}
        }
        // azure_pipelines_yaml: any .yml or .yaml under azure/
        if f.starts_with("azure/") {
            if f.ends_with(".yml") || f.ends_with(".yaml") {
                s.azure_pipelines_yaml += 1;
            }
        }
    }
    s
}

// ── Internal: I/O ─────────────────────────────────────────────────────────────

/// Stream-count newlines and bytes in a file without loading it into memory.
fn count_lines_and_bytes(path: &Path) -> io::Result<(u64, u64)> {
    let f = std::fs::File::open(path)?;
    let mut reader = io::BufReader::new(f);
    let mut buf = [0u8; 65536];
    let mut lines: u64 = 0;
    let mut bytes: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        lines += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
    }
    Ok((lines, bytes))
}

// ── Internal: path helpers ────────────────────────────────────────────────────

/// Extract the lowercase extension from a relative path string.
///
/// Rules:
/// - Take the last path component (basename).
/// - Find the last `.`; everything after it (lowercased) is the extension.
/// - If there is no `.`, or the `.` is the first character (hidden file), return `""`.
fn get_extension(rel: &str) -> String {
    let filename = rel.rsplit('/').next().unwrap_or(rel);
    // Skip leading dots (hidden files like `.gitignore`).
    let search_from = if filename.starts_with('.') { 1 } else { 0 };
    let name_tail = &filename[search_from..];
    match name_tail.rfind('.') {
        Some(pos) if pos < name_tail.len() - 1 => {
            name_tail[pos + 1..].to_lowercase()
        }
        _ => String::new(),
    }
}

/// Return the first path component, or `"<root>"` for root-level files.
fn get_top_level_dir(rel: &str) -> String {
    match rel.find('/') {
        Some(pos) => rel[..pos].to_string(),
        None => "<root>".to_string(),
    }
}

// ── Internal: timestamp ───────────────────────────────────────────────────────

fn current_utc_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    unix_to_iso8601(secs)
}

fn unix_to_iso8601(secs: u64) -> String {
    let s = (secs % 60) as u32;
    let m = ((secs / 60) % 60) as u32;
    let h = ((secs / 3600) % 24) as u32;
    let days = secs / 86400;
    let (year, month, day) = days_since_epoch_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
///
/// Uses Howard Hinnant's civil-from-days algorithm (MIT-licensed).
fn days_since_epoch_to_date(days: u64) -> (u32, u32, u32) {
    let z: i64 = days as i64 + 719_468;
    let era: i64 = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe: u64 = (z - era * 146_097) as u64; // day of era [0, 146096]
    let yoe: u64 = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // year of era [0, 399]
    let y: i64 = yoe as i64 + era * 400;
    let doy: u64 = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp: u64 = (5 * doy + 2) / 153; // [0, 11]
    let d: u32 = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let mo: u32 = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y: u32 = (if mo <= 2 { y + 1 } else { y }) as u32;
    (y, mo, d)
}

// ── Internal: formatting helpers ──────────────────────────────────────────────

fn fmt_opt_u64(v: Option<u64>) -> String {
    v.map(|n| fmt_n(n)).unwrap_or_else(|| "—".to_string())
}

fn fmt_n(n: u64) -> String {
    // Group with underscores for readability.
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push('_');
        }
        result.push(ch);
    }
    result.chars().rev().collect()
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_extension_handles_normal() {
        assert_eq!(get_extension("src/foo.ts"), "ts");
        assert_eq!(get_extension("foo.TS"), "ts");
        assert_eq!(get_extension("README"), "");
        assert_eq!(get_extension("Makefile"), "");
        assert_eq!(get_extension(".gitignore"), "");
        assert_eq!(get_extension("a.tar.gz"), "gz");
    }

    #[test]
    fn get_top_level_dir_handles_nested_and_root() {
        assert_eq!(get_top_level_dir("src/foo.ts"), "src");
        assert_eq!(get_top_level_dir("file.txt"), "<root>");
        assert_eq!(get_top_level_dir("a/b/c/d.ts"), "a");
    }

    #[test]
    fn timestamp_round_trip_epoch() {
        // 0 seconds since epoch should produce 1970-01-01T00:00:00Z
        assert_eq!(unix_to_iso8601(0), "1970-01-01T00:00:00Z");
        // 2024-01-01T00:00:00Z = 1704067200 (verified externally)
        assert_eq!(unix_to_iso8601(1_704_067_200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn fmt_n_groups_digits() {
        assert_eq!(fmt_n(0), "0");
        assert_eq!(fmt_n(1000), "1_000");
        assert_eq!(fmt_n(1_000_000), "1_000_000");
    }
}
