use std::collections::HashSet;
use cloudpack_transform::swc_adapter::strip_dead_exports;

fn dead(names: &[&str]) -> HashSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// A named function export in the dead set must be removed; others are kept.
#[test]
fn strip_named_function_export() {
    let src = "export function foo() {}\nexport function bar() {}";
    let result = strip_dead_exports(src, &dead(&["foo"])).unwrap();
    assert!(
        !result.contains("function foo"),
        "foo should be stripped but got: {result}"
    );
    assert!(
        result.contains("bar"),
        "bar should be kept but got: {result}"
    );
}

/// A named variable export (const) in the dead set must be removed; others are kept.
#[test]
fn strip_named_variable_export() {
    let src = "export const foo = 1;\nexport const bar = 2;";
    let result = strip_dead_exports(src, &dead(&["foo"])).unwrap();
    assert!(
        !result.contains("foo"),
        "foo should be stripped but got: {result}"
    );
    assert!(
        result.contains("bar"),
        "bar should be kept but got: {result}"
    );
}

/// `export default` must NEVER be stripped, even if "default" is in the dead set.
#[test]
fn default_export_never_stripped() {
    let src = "export default function() { return 42; }";
    let result = strip_dead_exports(src, &dead(&["default"])).unwrap();
    assert!(
        result.contains("export default"),
        "default export should be kept even when 'default' is in dead set, got: {result}"
    );
}

/// Dead specifiers are removed from re-export bindings; live ones are kept.
#[test]
fn strip_reexport_binding() {
    let src = "export { alpha, beta } from './x';";
    let result = strip_dead_exports(src, &dead(&["alpha"])).unwrap();
    assert!(
        !result.contains("alpha"),
        "alpha should be stripped but got: {result}"
    );
    assert!(
        result.contains("beta"),
        "beta should be kept but got: {result}"
    );
}

/// When the dead set is empty, source passes through unchanged (modulo formatting).
#[test]
fn no_dead_exports_passthrough() {
    let src = "export function foo() {}\nexport const bar = 1;";
    let result = strip_dead_exports(src, &dead(&[])).unwrap();
    assert!(
        result.contains("foo"),
        "foo should be kept but got: {result}"
    );
    assert!(
        result.contains("bar"),
        "bar should be kept but got: {result}"
    );
}

/// Syntactically invalid source must return an `Err`.
#[test]
fn parse_error_returns_err() {
    let src = "export function {{{ INVALID SYNTAX HERE";
    let result = strip_dead_exports(src, &dead(&[]));
    assert!(result.is_err(), "invalid source should return Err");
}
