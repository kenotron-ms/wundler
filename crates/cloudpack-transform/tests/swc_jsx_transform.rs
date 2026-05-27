
//! Tests that `.tsx`/`.jsx` files have their JSX syntax transformed to
//! `_jsx` / `_jsxs` calls (Automatic runtime) by `transform_ts_to_js`.
//!
//! With the **Automatic** JSX runtime, SWC injects
//! `import { jsx as _jsx, … } from "react/jsx-runtime"` automatically.
//! The `React` default import is therefore no longer required in scope and is
//! stripped when it isn't referenced by any non-JSX value expression.

use cloudpack_transform::swc_util::transform_ts_to_js;

/// A `.tsx` file with a JSX element should emit an automatic-runtime call
/// (`_jsx`) and import from `"react/jsx-runtime"`.  No raw JSX `<div` should
/// remain in the output.
#[test]
fn tsx_jsx_becomes_jsx_runtime_call() {
    let src = r#"const el = <div className="test">Hello</div>;"#;
    let result = transform_ts_to_js("app.tsx", src)
        .expect("transform_ts_to_js should succeed on valid TSX");

    // Automatic runtime — the output must reference react/jsx-runtime
    assert!(
        result.contains("react/jsx-runtime"),
        "expected import from react/jsx-runtime in output, got:\n{result}"
    );
    // No JSX open/close tag syntax should remain
    assert!(
        !result.contains("<div"),
        "expected no raw JSX <div in output, got:\n{result}"
    );
}

/// A plain `.ts` file (no JSX) must not be affected — types are stripped
/// but everything else passes through intact.
#[test]
fn ts_file_unchanged_by_jsx_pass() {
    let src = "const x: number = 42;";
    let result = transform_ts_to_js("lib.ts", src)
        .expect("transform_ts_to_js should succeed on plain TS");

    assert!(
        result.contains("42"),
        "numeric literal must survive transform, got:\n{result}"
    );
    assert!(
        !result.contains(": number"),
        "TS type annotation must be stripped, got:\n{result}"
    );
}

/// A `.tsx` component with React JSX, including a self-closing element,
/// must be converted to `_jsx` / `_jsxs` calls with no raw `<App` syntax.
#[test]
fn tsx_self_closing_element_becomes_jsx_runtime_call() {
    let src = "const el = <App />;";
    let result = transform_ts_to_js("index.tsx", src)
        .expect("transform_ts_to_js should succeed");

    assert!(
        result.contains("react/jsx-runtime"),
        "expected import from react/jsx-runtime in output, got:\n{result}"
    );
    assert!(
        !result.contains("<App"),
        "expected no raw JSX <App in output, got:\n{result}"
    );
}

/// A `.tsx` file using `React.createElement` already (not JSX) must
/// compile fine — the JSX transform pass must not break non-JSX TSX files.
#[test]
fn tsx_with_create_element_compiles_fine() {
    let src = r#"const el = React.createElement("div", null, "Hello");"#;
    let result = transform_ts_to_js("already.tsx", src)
        .expect("transform_ts_to_js should succeed");

    assert!(
        result.contains("React.createElement"),
        "React.createElement call must survive, got:\n{result}"
    );
}
