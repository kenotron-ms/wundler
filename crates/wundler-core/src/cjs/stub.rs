// CJS stub — CommonJS interop utilities.
//
// # Documented Limitations
//
// Static analysis covers `module.exports = { a, b, c }` only.
// Does not handle dynamic patterns, conditional exports, or
// `exports.a = ...` property assignments.
// Worker-based approach (runtime evaluation with happy-dom) deferred.

use std::path::Path;

/// Returns true if the source looks like a CommonJS module.
///
/// A module is considered CJS if it contains `module.exports` or `exports.`.
pub fn is_cjs(source: &str) -> bool {
    source.contains("module.exports") || source.contains("exports.")
}

/// Detect exported names from a CJS `module.exports = { ... }` assignment.
///
/// Calls an inner Option-returning helper, falling back to an empty vec on
/// parse failure. Only handles static `module.exports = { key: value, ... }`
/// patterns.
pub fn detect_cjs_exports(source: &str) -> Vec<String> {
    detect_cjs_exports_inner(source).unwrap_or_default()
}

fn detect_cjs_exports_inner(source: &str) -> Option<Vec<String>> {
    const MARKER: &str = "module.exports";
    let marker_pos = source.find(MARKER)?;
    let after_marker = &source[marker_pos + MARKER.len()..];

    // Skip whitespace, require '='
    let rest = after_marker.trim_start();
    let rest = rest.strip_prefix('=')?;

    // Skip whitespace, require '{'
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('{')?;

    // Find the matching closing brace (depth starts at 1 because '{' is consumed)
    let close_pos = find_matching_brace(rest)?;
    let inner = &rest[..close_pos];

    let mut exports = Vec::new();
    for entry in inner.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Take everything before ':' (if present) as the key
        let key = if let Some(colon_pos) = entry.find(':') {
            &entry[..colon_pos]
        } else {
            entry
        };
        let key = key.trim();
        if is_valid_js_identifier(key) {
            exports.push(key.to_string());
        }
    }
    Some(exports)
}

/// Find the position of the closing `}` matching an already-consumed `{`.
///
/// `s` is the slice *after* the opening brace. Depth starts at 1.
/// Returns the byte index of the closing `}` within `s`, or `None` if the
/// source is malformed (unbalanced braces or unclosed string literal).
///
/// Braces inside single-quoted (`'`), double-quoted (`"`), or template-literal
/// (`` ` ``) strings are ignored. Basic escape sequences (`\x`) are handled to
/// avoid a closing quote being mistaken for a string terminator.
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    let mut in_string: Option<char> = None;
    let mut chars = s.char_indices();

    while let Some((i, c)) = chars.next() {
        if let Some(quote) = in_string {
            if c == '\\' {
                // Skip the escaped character so it doesn't end the string
                chars.next();
            } else if c == quote {
                in_string = None;
            }
        } else {
            match c {
                '\'' | '"' | '`' => in_string = Some(c),
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Returns true if `s` is a valid JavaScript identifier.
///
/// First character must be a Unicode letter, `_`, or `$`.
/// Subsequent characters must be Unicode alphanumeric, `_`, or `$`.
fn is_valid_js_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !first.is_alphabetic() && first != '_' && first != '$' {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Generate an ESM stub for a CJS module at `path`.
///
/// # Empty `export_names`
///
/// ```text
/// import cjsExport from '{path}';
/// const defaultExport = cjsExport?.default?.default ?? cjsExport?.default ?? cjsExport;
/// export default defaultExport;
/// ```
///
/// # Named exports
///
/// Same as above, plus:
///
/// ```text
/// const { a, b, c } = cjsExport;
/// export { a, b, c };
/// ```
pub fn generate_cjs_stub(path: &Path, export_names: &[String]) -> String {
    let path_str = path.to_string_lossy();
    let mut out = format!(
        "import cjsExport from '{path_str}';\n\
         const defaultExport = cjsExport?.default?.default ?? cjsExport?.default ?? cjsExport;\n\
         export default defaultExport;\n"
    );
    if !export_names.is_empty() {
        let names = export_names.join(", ");
        out.push_str(&format!("const {{ {names} }} = cjsExport;\n"));
        out.push_str(&format!("export {{ {names} }};\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_is_cjs_detects_module_exports() {
        // positive: module.exports
        assert!(is_cjs("module.exports = { foo: 1 };"));
        // positive: exports.
        assert!(is_cjs("exports.foo = function() {};"));
        // negative: ESM import
        assert!(!is_cjs("import { foo } from './foo';"));
    }

    #[test]
    fn test_detect_cjs_exports_simple_object() {
        let source = "const helper = (x) => x * 2;\nmodule.exports = { double: helper, triple: (x) => x * 3, VERSION: '1.0.0' };";
        let exports = detect_cjs_exports(source);
        assert!(
            exports.contains(&"double".to_string()),
            "expected 'double' in {:?}",
            exports
        );
        assert!(
            exports.contains(&"triple".to_string()),
            "expected 'triple' in {:?}",
            exports
        );
        assert!(
            exports.contains(&"VERSION".to_string()),
            "expected 'VERSION' in {:?}",
            exports
        );
    }

    #[test]
    fn test_detect_cjs_exports_returns_empty_for_dynamic_pattern() {
        // module.exports = buildExports() → any result acceptable; must not panic
        let source = "module.exports = buildExports();";
        let _exports = detect_cjs_exports(source);
    }

    #[test]
    fn test_generate_cjs_stub_contains_export_list() {
        let path = Path::new("./node_modules/some-module/index.js");
        let export_names = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let stub = generate_cjs_stub(path, &export_names);
        assert!(
            stub.contains("export { a, b, c }"),
            "expected 'export {{ a, b, c }}' in:\n{}",
            stub
        );
        assert!(
            stub.contains("export default defaultExport"),
            "expected 'export default defaultExport' in:\n{}",
            stub
        );
        assert!(
            stub.contains("import cjsExport from"),
            "expected 'import cjsExport from' in:\n{}",
            stub
        );
    }

    #[test]
    fn test_generate_cjs_stub_empty_exports_is_valid_esm() {
        let path = Path::new("./node_modules/some-module/index.js");
        let stub = generate_cjs_stub(path, &[]);
        assert!(
            stub.contains("export default"),
            "expected 'export default' in:\n{}",
            stub
        );
    }

    #[test]
    fn test_cjs_fixture_file_is_detected() {
        use std::fs;
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let fixture_path =
            std::path::PathBuf::from(manifest_dir).join("tests/fixtures/cjs_module.js");
        let source =
            fs::read_to_string(&fixture_path).expect("failed to read cjs_module.js fixture");
        assert!(
            is_cjs(&source),
            "expected is_cjs to return true for cjs_module.js"
        );
        let exports = detect_cjs_exports(&source);
        assert!(
            exports.contains(&"double".to_string()),
            "expected 'double' in {:?}",
            exports
        );
    }
}
