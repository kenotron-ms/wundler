
//! Integration tests for `repo_scale` module.
//!
//! Written BEFORE implementation (TDD / RED phase).
//! Each test name directly maps to the spec requirement list.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use wundler_bench::repo_scale::{
    measure_repo, measure_repo_with_clock, render_json, render_markdown, RepoScaleOptions,
    SCHEMA_VERSION,
};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Run a git command in `repo`, asserting success.
/// Injects author/committer identity via env vars so no user config is needed.
fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Tester")
        .env("GIT_AUTHOR_EMAIL", "t@t.com")
        .env("GIT_COMMITTER_NAME", "Tester")
        .env("GIT_COMMITTER_EMAIL", "t@t.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("failed to spawn git");
    assert!(status.success(), "git {:?} exited non-zero", args);
}

/// Write `content` to `repo/rel_path`, creating parent dirs as needed,
/// and immediately `git add` the file.
fn write_and_track(repo: &Path, rel_path: &str, content: &str) {
    let full = repo.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&full, content).unwrap();
    git(repo, &["add", rel_path]);
}

/// `git commit -m <message>` with no extra options.
fn commit(repo: &Path, message: &str) {
    git(repo, &["commit", "-m", message]);
}

/// Build a default `RepoScaleOptions` pointing at `dir`.
fn opts_for(dir: &Path) -> RepoScaleOptions {
    RepoScaleOptions {
        path: dir.to_path_buf(),
        ..Default::default()
    }
}

// ── Tests 1–18 ───────────────────────────────────────────────────────────────

// 1. Non-git directory → report has is_git_repo=false, zero files.
#[test]
fn measure_repo_on_empty_non_git_dir_returns_zero() {
    let dir = TempDir::new().unwrap();
    let report = measure_repo(&opts_for(dir.path())).unwrap();

    assert!(!report.git.is_git_repo, "expected is_git_repo=false");
    assert_eq!(report.tracked_files_total, 0);
    assert!(report.by_extension.is_empty());
    assert!(report.top_directories.is_empty());
    // A warning about "not a git repo" must appear.
    assert!(
        report.warnings.iter().any(|w| w.to_lowercase().contains("git")),
        "expected a git-related warning; got: {:?}",
        report.warnings
    );
}

// 2. Git repo initialised but NO commits yet → detected as git, HEAD unknown.
#[test]
fn measure_repo_detects_git_repo_with_no_commits() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init"]);

    let report = measure_repo(&opts_for(dir.path())).unwrap();

    assert!(report.git.is_git_repo, "expected is_git_repo=true");
    assert_eq!(report.git.head_commit, None, "expected no HEAD");
    assert_eq!(report.git.commit_count, None, "expected no commit count");
    assert_eq!(report.tracked_files_total, 0);
}

// 3. Tracked files are counted and grouped by extension with correct line totals.
#[test]
fn measure_repo_counts_tracked_files_by_extension() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);
    write_and_track(p, "a.ts", "const x = 1;\n");           // 1 line
    write_and_track(p, "b.ts", "const y = 2;\nconst z = 3;\n"); // 2 lines
    write_and_track(p, "c.js", "// js\n");                  // 1 line
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    assert_eq!(report.tracked_files_total, 3);

    let ts = report
        .by_extension
        .iter()
        .find(|e| e.extension == "ts")
        .expect("expected 'ts' extension");
    assert_eq!(ts.file_count, 2);
    assert_eq!(ts.line_count, 3); // 1 + 2

    let js = report
        .by_extension
        .iter()
        .find(|e| e.extension == "js")
        .expect("expected 'js' extension");
    assert_eq!(js.file_count, 1);
    assert_eq!(js.line_count, 1);
}

// 4. Extensions are normalised to lowercase.
#[test]
fn measure_repo_normalizes_extension_to_lowercase() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);
    write_and_track(p, "Foo.TS", "line\n");
    write_and_track(p, "Bar.ts", "line\n");
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    let ts = report
        .by_extension
        .iter()
        .find(|e| e.extension == "ts")
        .expect("expected lowercase 'ts' entry");
    assert_eq!(ts.file_count, 2, "both .TS and .ts should map to 'ts'");
    assert!(
        !report.by_extension.iter().any(|e| e.extension.contains('T')),
        "no uppercase extension should appear"
    );
}

// 5. Files without any extension get the empty-string extension key.
#[test]
fn measure_repo_handles_files_with_no_extension() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);
    write_and_track(p, "Makefile", "all:\n");
    write_and_track(p, "README", "hello\n");
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    let no_ext = report
        .by_extension
        .iter()
        .find(|e| e.extension.is_empty())
        .expect("expected empty-string extension for extensionless files");
    assert_eq!(no_ext.file_count, 2);
}

// 6. top_directories is capped at top_dirs and sorted descending by file_count.
#[test]
fn measure_repo_top_directories_sorted_and_capped() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    // dir00 gets 1 file, dir01 gets 2, …, dir24 gets 25 files.
    for i in 0usize..25 {
        for j in 0..=i {
            write_and_track(p, &format!("dir{i:02}/file{j}.txt"), "x\n");
        }
    }
    commit(p, "init");

    let opts = RepoScaleOptions {
        path: p.to_path_buf(),
        top_dirs: 5,
        ..Default::default()
    };
    let report = measure_repo(&opts).unwrap();

    assert_eq!(report.top_directories.len(), 5, "capped at top_dirs=5");

    // Descending order
    for w in report.top_directories.windows(2) {
        assert!(
            w[0].file_count >= w[1].file_count,
            "not sorted descending: {:?} before {:?}",
            w[0],
            w[1]
        );
    }

    // The directory with the most files (dir24 → 25 files) must be first.
    assert_eq!(report.top_directories[0].file_count, 25);
}

// 7. by_extension is capped at top_extensions.
#[test]
fn measure_repo_top_extensions_capped() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    // 35 distinct extensions
    for i in 0u32..35 {
        write_and_track(p, &format!("file{i}.ext{i:02}"), "x\n");
    }
    commit(p, "init");

    let opts = RepoScaleOptions {
        path: p.to_path_buf(),
        top_extensions: 10,
        ..Default::default()
    };
    let report = measure_repo(&opts).unwrap();

    assert_eq!(
        report.by_extension.len(),
        10,
        "capped at top_extensions=10"
    );
}

// 8. packages/ and docs/ workspace stats are populated correctly.
#[test]
fn measure_repo_detects_workspace_packages() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    write_and_track(p, "packages/pkg-a/package.json", r#"{"name":"pkg-a"}"#);
    write_and_track(p, "packages/pkg-a/src/index.ts", "export {};");
    write_and_track(p, "packages/pkg-b/package.json", r#"{"name":"pkg-b"}"#);
    write_and_track(p, "docs/guide.md", "# Guide\n");
    write_and_track(p, "docs/api.md", "# API\n");
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    assert_eq!(
        report.workspace.packages_immediate_subdirs,
        Some(2),
        "expected 2 packages"
    );
    assert_eq!(
        report.workspace.docs_file_count,
        Some(2),
        "expected 2 docs files"
    );
}

// 9. Manifest counts (package.json, Cargo.toml, tsconfig.json, pyproject.toml).
#[test]
fn measure_repo_counts_manifests() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    write_and_track(p, "package.json", "{}");
    write_and_track(p, "packages/a/package.json", "{}");
    write_and_track(p, "Cargo.toml", "[package]");
    write_and_track(p, "crates/core/Cargo.toml", "[package]");
    write_and_track(p, "tsconfig.json", "{}");
    write_and_track(p, "pyproject.toml", "[tool]");
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    assert_eq!(report.manifests.package_json, 2);
    assert_eq!(report.manifests.cargo_toml, 2);
    assert_eq!(report.manifests.tsconfig_json, 1);
    assert_eq!(report.manifests.pyproject_toml, 1);
}

// 10. azure_pipelines_yaml counts .yml/.yaml files under azure/ (recursive).
#[test]
fn measure_repo_counts_azure_pipelines() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    write_and_track(p, "azure/build.yml", "steps: []");
    write_and_track(p, "azure/deploy.yaml", "steps: []");
    write_and_track(p, "azure/nested/test.yml", "steps: []");
    write_and_track(p, "other/pipeline.yml", "steps: []"); // NOT under azure/
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    assert_eq!(
        report.manifests.azure_pipelines_yaml, 3,
        "3 yml/yaml files under azure/"
    );
}

// 11. A warning is emitted when node_modules is found in the tracked file list.
#[test]
fn measure_repo_warns_if_node_modules_tracked() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);
    // Disable any excludes file so node_modules can be staged.
    git(p, &["config", "core.excludesFile", "/dev/null"]);

    write_and_track(p, "node_modules/lodash/index.js", "module.exports = {};\n");
    commit(p, "init");

    let report = measure_repo(&opts_for(p)).unwrap();

    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("node_modules")),
        "expected node_modules warning; got: {:?}",
        report.warnings
    );
}

// 12. files_ever_tracked counts files that were added then deleted.
#[test]
fn measure_repo_files_ever_tracked_counts_deletions() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    // Commit 1: add file1.txt
    write_and_track(p, "file1.txt", "content\n");
    commit(p, "add file1");

    // Commit 2: delete file1.txt, add file2.txt
    git(p, &["rm", "file1.txt"]);
    write_and_track(p, "file2.txt", "content\n");
    commit(p, "replace file1 with file2");

    let report = measure_repo(&opts_for(p)).unwrap();

    // Only file2.txt tracked now
    assert_eq!(report.tracked_files_total, 1);
    // Both were ever added
    assert_eq!(
        report.git.files_ever_tracked,
        Some(2),
        "expected files_ever_tracked=2"
    );
}

// 13. Untracked files are excluded by default (include_untracked=false).
#[test]
fn measure_repo_excludes_untracked_by_default() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    write_and_track(p, "tracked.ts", "const x = 1;\n");
    commit(p, "init");

    // Create an untracked file (not staged, not committed)
    std::fs::write(p.join("untracked.ts"), "const y = 2;\n").unwrap();

    let report = measure_repo(&opts_for(p)).unwrap();
    assert_eq!(report.tracked_files_total, 1, "untracked should not be counted");
}

// 14. Untracked files ARE included when include_untracked=true.
#[test]
fn measure_repo_includes_untracked_when_flag_set() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);

    write_and_track(p, "tracked.ts", "const x = 1;\n");
    commit(p, "init");

    std::fs::write(p.join("untracked.ts"), "const y = 2;\n").unwrap();

    let opts = RepoScaleOptions {
        path: p.to_path_buf(),
        include_untracked: true,
        ..Default::default()
    };
    let report = measure_repo(&opts).unwrap();
    assert_eq!(
        report.tracked_files_total, 2,
        "untracked should be counted when flag is set"
    );
}

// 15. render_json produces valid JSON containing schema_version=1.
#[test]
fn render_json_includes_schema_version_1() {
    let dir = TempDir::new().unwrap();
    let report = measure_repo(&opts_for(dir.path())).unwrap();
    let json = render_json(&report);

    let v: serde_json::Value = serde_json::from_str(&json).expect("render_json is not valid JSON");
    assert_eq!(
        v["schema_version"],
        serde_json::json!(SCHEMA_VERSION),
        "schema_version must be {SCHEMA_VERSION}"
    );
}

// 16. render_markdown includes all required section headings.
#[test]
fn render_markdown_contains_required_sections() {
    let dir = TempDir::new().unwrap();
    let report = measure_repo(&opts_for(dir.path())).unwrap();
    let md = render_markdown(&report);

    for heading in &[
        "# Repo Scale Report",
        "## Top extensions",
        "## Top directories (depth=1)",
        "## Workspace shape",
        "## Manifests",
        "## Warnings",
    ] {
        assert!(
            md.contains(heading),
            "markdown missing required section: {heading}\n\nFull output:\n{md}"
        );
    }
}

// 17. Two calls with the same clock produce identical JSON (deterministic).
#[test]
fn measure_repo_is_deterministic_across_runs() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init"]);
    write_and_track(p, "a.ts", "hello\n");
    write_and_track(p, "b.ts", "world\n");
    write_and_track(p, "c.js", "foo\n");
    commit(p, "init");

    let opts = opts_for(p);
    let ts = "2024-01-01T00:00:00Z";

    let r1 = measure_repo_with_clock(&opts, ts).unwrap();
    let r2 = measure_repo_with_clock(&opts, ts).unwrap();

    assert_eq!(
        render_json(&r1),
        render_json(&r2),
        "two runs with same clock must produce identical JSON"
    );
}

// 18. measure_repo_with_clock uses the supplied timestamp verbatim.
#[test]
fn measure_repo_with_clock_uses_supplied_timestamp() {
    let dir = TempDir::new().unwrap();
    let ts = "2099-12-31T23:59:59Z";
    let report = measure_repo_with_clock(&opts_for(dir.path()), ts).unwrap();
    assert_eq!(report.measured_at_utc, ts);
}
