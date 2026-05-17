//! Level-0 ESM build verification test.
//!
//! Tests that the build pipeline:
//! 1. Produces chunk files whose hashes match what index.html references (Bug 1)
//! 2. Strips TypeScript syntax so output is valid JS (Bug 2)
//! 3. Uses ESM scope-flattening instead of IIFE wrapping (Bug 3)

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use wundler_pipeline::config::{BuildConfig, EngineChoice};
use wundler_pipeline::pipeline::BuildPipeline;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal 3-file TypeScript project.
///
/// `tag` is embedded in each file so parallel test instances produce
/// distinct content hashes and don't race on the same cache entry.
///
///  - `src/entry.ts`  — entry point; imports `add` from `./util`; uses TS type annotation
///  - `src/util.ts`   — exports a typed `add` function
///  - `src/lazy.ts`   — standalone module; TS interface + typed export
///
/// Returns `(project_dir, out_dir, config)`.
fn make_ts_project(tag: &str) -> (TempDir, TempDir, BuildConfig) {
    let project_dir = TempDir::new().unwrap();
    let out_dir = TempDir::new().unwrap();

    let src = project_dir.path().join("src");
    fs::create_dir_all(&src).unwrap();

    // entry.ts — uses TypeScript type annotation `: number`
    fs::write(
        src.join("entry.ts"),
        format!(
            "// {tag}\nimport {{ add }} from './util';\nconst result: number = add(1, 2);\nconsole.log(result);\n"
        ),
    )
    .unwrap();

    // util.ts — exports a function with TypeScript parameter/return types
    fs::write(
        src.join("util.ts"),
        format!(
            "// {tag}\nexport function add(a: number, b: number): number {{\n    return a + b;\n}}\n"
        ),
    )
    .unwrap();

    // lazy.ts — TypeScript interface + typed const export
    fs::write(
        src.join("lazy.ts"),
        format!(
            "// {tag}\ninterface LazyData {{\n    value: string;\n}}\nexport const LAZY_VALUE: string = 'lazy';\n"
        ),
    )
    .unwrap();

    let config = BuildConfig {
        root: project_dir.path().to_path_buf(),
        out_dir: out_dir.path().to_path_buf(),
        source_maps: false,
        commons_threshold: 2,
        engine: EngineChoice::Swc,
        entry_points: {
            let mut m = HashMap::new();
            m.insert("main".to_string(), PathBuf::from("src/entry.ts"));
            m
        },
        budget: None,
        dev: None,
    };

    (project_dir, out_dir, config)
}

// ---------------------------------------------------------------------------
// Test 1: index.html references chunk files that actually exist on disk
// (Bug 1 — hash mismatch)
// ---------------------------------------------------------------------------

#[test]
fn index_html_references_existing_chunk_files() {
    let (_project_dir, out_dir, config) = make_ts_project("level0-hash-test");
    let out_path = out_dir.path().to_path_buf();

    let pipeline = BuildPipeline::new(config);
    pipeline.build().expect("build() must succeed");

    let html = fs::read_to_string(out_path.join("index.html"))
        .expect("index.html must exist");

    // Extract every `src="chunks/<filename>"` reference from index.html.
    let mut referenced_files: Vec<String> = Vec::new();
    for line in html.lines() {
        if let Some(start) = line.find("src=\"chunks/") {
            let after = &line[start + "src=\"chunks/".len()..];
            if let Some(end) = after.find('"') {
                referenced_files.push(after[..end].to_string());
            }
        }
    }

    assert!(
        !referenced_files.is_empty(),
        "index.html must reference at least one chunk file; got:\n{html}"
    );

    let chunks_dir = out_path.join("chunks");
    for file_name in &referenced_files {
        let chunk_path = chunks_dir.join(file_name);
        assert!(
            chunk_path.exists(),
            "index.html references '{file_name}' but that file does not exist in chunks/.\n\
             Existing chunks: {:?}",
            fs::read_dir(&chunks_dir)
                .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: chunk files contain no TypeScript syntax
// (Bug 2 — missing TS→JS transpilation)
// ---------------------------------------------------------------------------

#[test]
fn chunk_files_contain_no_typescript_syntax() {
    let (_project_dir, out_dir, config) = make_ts_project("level0-ts-syntax-test");
    let out_path = out_dir.path().to_path_buf();

    let pipeline = BuildPipeline::new(config);
    pipeline.build().expect("build() must succeed");

    let chunks_dir = out_path.join("chunks");
    let js_files: Vec<_> = fs::read_dir(&chunks_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "js").unwrap_or(false))
        .collect();

    assert!(!js_files.is_empty(), "at least one .js chunk must be emitted");

    for entry in &js_files {
        let code = fs::read_to_string(entry.path()).unwrap();
        let path = entry.path();

        // TypeScript-specific syntax patterns that must NOT appear in emitted JS.
        let ts_patterns = [
            (": number", "typed parameter/return annotation"),
            (": string", "typed parameter/return annotation"),
            ("interface ", "TypeScript interface declaration"),
        ];

        for (pattern, description) in &ts_patterns {
            assert!(
                !code.contains(pattern),
                "chunk {:?} contains TypeScript syntax ({description}): {pattern:?}\n\
                 First 500 chars of chunk:\n{}\n",
                path,
                &code[..code.len().min(500)]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 3: chunk files use ESM (no IIFE wrapping, imports at top level)
// (Bug 3 — IIFE incompatible with ESM)
// ---------------------------------------------------------------------------

#[test]
fn chunk_files_use_esm_not_iife() {
    let (_project_dir, out_dir, config) = make_ts_project("level0-iife-test");
    let out_path = out_dir.path().to_path_buf();

    let pipeline = BuildPipeline::new(config);
    pipeline.build().expect("build() must succeed");

    let chunks_dir = out_path.join("chunks");
    let js_files: Vec<_> = fs::read_dir(&chunks_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "js").unwrap_or(false))
        .collect();

    assert!(!js_files.is_empty(), "at least one .js chunk must be emitted");

    for entry in &js_files {
        let code = fs::read_to_string(entry.path()).unwrap();
        let path = entry.path();

        // Must NOT use IIFE pattern.
        assert!(
            !code.contains("(function()"),
            "chunk {:?} uses IIFE wrapping which is ESM-incompatible:\n{}",
            path,
            &code[..code.len().min(500)]
        );
        assert!(
            !code.contains("})();"),
            "chunk {:?} uses IIFE close `}})();` which is ESM-incompatible:\n{}",
            path,
            &code[..code.len().min(500)]
        );

        // Any `import` statement must appear before any function/const declaration.
        // Collect the line numbers of import statements and function declarations.
        let lines: Vec<&str> = code.lines().collect();
        let first_import_line = lines
            .iter()
            .position(|l| l.trim_start().starts_with("import "));
        let first_decl_line = lines.iter().position(|l| {
            let t = l.trim_start();
            t.starts_with("function ")
                || t.starts_with("const ")
                || t.starts_with("let ")
                || t.starts_with("var ")
                || t.starts_with("class ")
        });

        if let (Some(import_ln), Some(decl_ln)) = (first_import_line, first_decl_line) {
            assert!(
                import_ln < decl_ln,
                "chunk {:?}: import statement at line {} appears AFTER first declaration at line {}\n\
                 (imports must be hoisted to the top of the file for valid ESM)\n\
                 Full chunk:\n{}",
                path,
                import_ln + 1,
                decl_ln + 1,
                code
            );
        }
    }
}
