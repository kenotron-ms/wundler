use std::path::PathBuf;
use wundler_core::{ExportKind, ModuleSummarizer, SideEffectMarker};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn test_summarize_esm_basic() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("esm_basic.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    // id must be a 64-char hex string (SHA-256)
    assert_eq!(
        node.id.as_str().len(),
        64,
        "expected 64-char hex id, got: {}",
        node.id.as_str()
    );

    // path ends with the fixture filename
    assert!(
        node.path.ends_with("esm_basic.ts"),
        "expected path to end with 'esm_basic.ts', got: {}",
        node.path
    );

    // all four exports must be present
    let export_names: Vec<&str> = node
        .summary
        .exports
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert!(
        export_names.contains(&"VERSION"),
        "expected VERSION in exports: {:?}",
        export_names
    );
    assert!(
        export_names.contains(&"greet"),
        "expected greet in exports: {:?}",
        export_names
    );
    assert!(
        export_names.contains(&"Greeter"),
        "expected Greeter in exports: {:?}",
        export_names
    );
    assert!(
        export_names.contains(&"default"),
        "expected default in exports: {:?}",
        export_names
    );

    // 'react' must appear in imports (type-only import is excluded)
    let import_specs: Vec<&str> = node
        .summary
        .imports
        .iter()
        .map(|i| i.specifier.as_str())
        .collect();
    assert!(
        import_specs.contains(&"react"),
        "expected 'react' in imports: {:?}",
        import_specs
    );

    // console.log is inside a function body → module is side-effect-free
    assert_eq!(
        node.summary.side_effects,
        SideEffectMarker::None,
        "expected no side effects for esm_basic.ts"
    );
}

#[test]
fn test_summarize_esm_reexport() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("esm_reexport.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    let kinds: Vec<&ExportKind> = node.summary.exports.iter().map(|e| &e.kind).collect();

    assert!(
        kinds.contains(&&ExportKind::ReExport),
        "expected at least one ReExport, got kinds: {:?}",
        kinds
    );
    assert!(
        kinds.contains(&&ExportKind::StarExport),
        "expected at least one StarExport, got kinds: {:?}",
        kinds
    );
}

#[test]
fn test_summarize_dynamic_import() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("dynamic_import.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    let dynamic_imports: Vec<&str> = node
        .summary
        .imports
        .iter()
        .filter(|i| i.is_dynamic)
        .map(|i| i.specifier.as_str())
        .collect();

    assert_eq!(
        dynamic_imports.len(),
        1,
        "expected exactly 1 literal dynamic import, got: {:?}",
        dynamic_imports
    );
    assert_eq!(
        dynamic_imports[0], "./heavy",
        "expected dynamic import specifier './heavy', got: {}",
        dynamic_imports[0]
    );
}

#[test]
fn test_summarize_side_effects_ambient_is_definite() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("side_effects_ambient.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    assert_eq!(
        node.summary.side_effects,
        SideEffectMarker::Definite,
        "expected Definite side effects for ambient globals file"
    );

    assert!(
        node.summary.ambient_refs.contains(&"window.APP_VERSION".to_string()),
        "expected 'window.APP_VERSION' in ambient_refs, got: {:?}",
        node.summary.ambient_refs
    );
}

#[test]
fn test_summarize_side_effects_pure_is_none() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("side_effects_pure.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    assert_eq!(
        node.summary.side_effects,
        SideEffectMarker::None,
        "expected no side effects for pure module"
    );

    assert!(
        node.summary.ambient_refs.is_empty(),
        "expected empty ambient_refs for pure module, got: {:?}",
        node.summary.ambient_refs
    );
}

#[test]
fn test_summary_serializes_to_json() {
    let summarizer = ModuleSummarizer::new();
    let path = fixtures_dir().join("side_effects_pure.ts");
    let node = summarizer.summarize(&path).expect("summarize failed");

    let json = serde_json::to_string(&node).expect("JSON serialization failed");

    assert!(
        json.contains("sideEffects"),
        "expected 'sideEffects' key in JSON: {}",
        json
    );
    assert!(
        json.contains("NONE"),
        "expected 'NONE' value in JSON: {}",
        json
    );
}
