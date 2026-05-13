// CJS stub — placeholder CommonJS interop utilities.
// Full implementation is deferred to a later task.

/// Returns true if the source looks like a CommonJS module.
///
/// A module is considered CJS if it contains `module.exports` or `exports.`.
pub fn is_cjs(source: &str) -> bool {
    source.contains("module.exports") || source.contains("exports.")
}

/// Detect exported names from a CJS module.
///
/// Placeholder implementation — returns an empty vector.
pub fn detect_cjs_exports(_source: &str) -> Vec<String> {
    vec![]
}

/// Generate an ESM stub from a CJS source string.
///
/// Placeholder implementation — returns a comment string.
pub fn generate_cjs_stub(_source: &str) -> String {
    "// cjs-stub: placeholder ESM wrapper".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cjs_detects_module_exports() {
        assert!(is_cjs("module.exports = { foo: 1 };"));
    }

    #[test]
    fn test_is_cjs_detects_exports_dot() {
        assert!(is_cjs("exports.foo = function() {};"));
    }

    #[test]
    fn test_is_cjs_rejects_esm() {
        assert!(!is_cjs("export const foo = 1;"));
    }

    #[test]
    fn test_detect_cjs_exports_returns_empty() {
        let result = detect_cjs_exports("module.exports = { foo: 1 };");
        assert!(result.is_empty());
    }

    #[test]
    fn test_generate_cjs_stub_returns_string() {
        let stub = generate_cjs_stub("module.exports = {};");
        assert!(!stub.is_empty());
    }
}
