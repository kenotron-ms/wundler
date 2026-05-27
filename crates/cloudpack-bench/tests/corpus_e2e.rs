//! End-to-end: load the committed tiny profile, generate + verify.

use std::path::PathBuf;

use cloudpack_bench::corpus_gen::generate_corpus;
use cloudpack_bench::corpus_verify::verify_corpus;
use cloudpack_bench::profile::{load_profile, CheckStatus};

fn tiny_profile_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("profiles/test/tiny.v1.json")
}

#[test]
fn tiny_profile_generates_and_verifies() {
    let profile = load_profile(&tiny_profile_path()).expect("load tiny profile");
    let dir = tempfile::tempdir().unwrap();
    generate_corpus(&profile, dir.path()).expect("generate");
    let report = verify_corpus(&profile, dir.path()).expect("verify");
    assert!(report.passed, "report: {report:#?}");
    assert!(report.checks.iter().all(|c| c.status != CheckStatus::Fail));
}

#[test]
fn tiny_profile_generation_is_deterministic() {
    let profile = load_profile(&tiny_profile_path()).expect("load");
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    generate_corpus(&profile, a.path()).unwrap();
    generate_corpus(&profile, b.path()).unwrap();

    fn read_sorted(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.file_name().and_then(|s| s.to_str())
                    != Some(".cloudpack-bench-fingerprint.json")
                {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                    out.push((rel, std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort_by(|x, y| x.0.cmp(&y.0));
        out
    }

    assert_eq!(read_sorted(a.path()), read_sorted(b.path()));
}
