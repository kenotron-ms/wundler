use std::io::Write;
use tempfile::NamedTempFile;
use wundler_pipeline::config::{BuildConfig, EngineChoice};

fn write_temp_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("could not create temp file");
    write!(f, "{content}").expect("write failed");
    f
}

#[test]
fn load_minimal_config() {
    let toml = r#"
[build]
root = "src"
out_dir = "dist"

[entry]
"/" = "src/index.ts"
"#;
    let f = write_temp_toml(toml);
    let cfg = BuildConfig::load(f.path()).expect("should load");
    assert_eq!(cfg.root.to_str().unwrap(), "src");
    assert_eq!(cfg.out_dir.to_str().unwrap(), "dist");
    assert_eq!(cfg.entry_points.get("/").unwrap().to_str().unwrap(), "src/index.ts");
}

#[test]
fn defaults_applied_when_omitted() {
    let toml = r#"
[build]
root = "src"
out_dir = "dist"

[entry]
"/" = "src/index.ts"
"#;
    let f = write_temp_toml(toml);
    let cfg = BuildConfig::load(f.path()).expect("should load");
    assert!(!cfg.source_maps, "source_maps should default to false");
    assert_eq!(cfg.commons_threshold, 2, "commons_threshold should default to 2");
    assert!(
        matches!(cfg.engine, EngineChoice::Swc),
        "engine should default to Swc"
    );
}

#[test]
fn engine_choice_rolldown() {
    let toml = r#"
[build]
root = "src"
out_dir = "dist"
engine = "rolldown"

[entry]
"/" = "src/index.ts"
"#;
    let f = write_temp_toml(toml);
    let cfg = BuildConfig::load(f.path()).expect("should load");
    assert!(
        matches!(cfg.engine, EngineChoice::Rolldown),
        "engine should be Rolldown"
    );
}

#[test]
fn engine_choice_rspack() {
    let toml = r#"
[build]
root = "src"
out_dir = "dist"
engine = "rspack"

[entry]
"/" = "src/index.ts"
"#;
    let f = write_temp_toml(toml);
    let cfg = BuildConfig::load(f.path()).expect("should load");
    assert!(
        matches!(cfg.engine, EngineChoice::Rspack),
        "engine should be Rspack"
    );
}

#[test]
fn missing_file_returns_error() {
    let result = BuildConfig::load(std::path::Path::new("/nonexistent/path/wundler.toml"));
    assert!(result.is_err(), "should return error for missing file");
}

#[test]
fn unknown_engine_is_error() {
    let toml = r#"
[build]
root = "src"
out_dir = "dist"
engine = "webpack-classic"

[entry]
"/" = "src/index.ts"
"#;
    let f = write_temp_toml(toml);
    let result = BuildConfig::load(f.path());
    assert!(result.is_err(), "unknown engine should return error");
}
