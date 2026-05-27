//! Call edge analyser — walks a parsed SWC `Module` and produces a list of `CallEdge` items
//! representing intra-module export-to-export call relationships.

use std::collections::HashSet;

use swc_core::ecma::ast::{
    CallExpr, Callee, Decl, Expr, FnDecl, Module, ModuleDecl, ModuleItem, Stmt,
};
use swc_core::ecma::visit::{Visit, VisitWith};

use crate::types::CallEdge;

/// Extract call edges between exported functions in a module.
///
/// Walks each exported function body and emits a `CallEdge` whenever one
/// exported function calls another exported function by name.  Non-exported
/// functions are never tracked as callers.
pub fn extract_call_edges(module: &Module, exported_names: &HashSet<String>) -> Vec<CallEdge> {
    let mut visitor = CallEdgeVisitor {
        exported_names,
        current_fn: None,
        edges: Vec::new(),
    };

    for item in &module.body {
        match item {
            // `export function foo() { … }` — always visit exported fn bodies.
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export_decl)) => {
                if let Decl::Fn(fn_decl) = &export_decl.decl {
                    visitor.visit_exported_fn(fn_decl);
                }
            }
            // `function foo() { … }` at the top level — visit only when the
            // name is in the exported set (e.g. exported via a separate
            // `export { foo }` declaration).
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl)))
                if exported_names.contains(fn_decl.ident.sym.as_ref()) =>
            {
                visitor.visit_exported_fn(fn_decl);
            }
            _ => {}
        }
    }

    visitor.edges
}

// ---------------------------------------------------------------------------
// Visitor
// ---------------------------------------------------------------------------

struct CallEdgeVisitor<'a> {
    exported_names: &'a HashSet<String>,
    /// The name of the exported function whose body is currently being walked.
    current_fn: Option<String>,
    edges: Vec<CallEdge>,
}

impl<'a> CallEdgeVisitor<'a> {
    /// Enter an exported function: set the current caller context, walk the
    /// function body, then restore the previous context.
    fn visit_exported_fn(&mut self, fn_decl: &FnDecl) {
        let prev = self.current_fn.take();
        self.current_fn = Some(fn_decl.ident.sym.to_string());
        fn_decl.function.visit_children_with(self);
        self.current_fn = prev;
    }
}

impl<'a> Visit for CallEdgeVisitor<'a> {
    fn visit_call_expr(&mut self, n: &CallExpr) {
        if let Some(caller) = self.current_fn.clone() {
            if let Callee::Expr(expr) = &n.callee {
                if let Expr::Ident(ident) = expr.as_ref() {
                    let callee_name = ident.sym.to_string();
                    if self.exported_names.contains(&callee_name) && callee_name != caller {
                        self.edges.push(CallEdge {
                            caller,
                            callee: callee_name,
                        });
                    }
                }
            }
        }
        // Always recurse so nested call expressions are also visited.
        n.visit_children_with(self);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    fn parse(src: &str) -> Module {
        parse_module(src, Path::new("test.js")).expect("parse failed")
    }

    fn exported_set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    /// greet (exported) calls formatName (exported) → exactly 1 edge greet→formatName.
    /// helper (non-exported) calls greet but is NOT tracked.
    #[test]
    fn test_detects_call_edge_between_exports() {
        let module = parse(
            "export function greet(name) { return formatName(name); }
             export function formatName(name) { return 'Mr ' + name; }
             function helper() { greet('Alice'); }",
        );
        let exported = exported_set(&["greet", "formatName"]);
        let edges = extract_call_edges(&module, &exported);
        assert_eq!(edges.len(), 1, "expected exactly 1 edge, got: {:?}", edges);
        assert_eq!(edges[0].caller, "greet");
        assert_eq!(edges[0].callee, "formatName");
    }

    /// add and mul are both exported but neither calls the other → 0 edges.
    #[test]
    fn test_no_edges_for_non_calling_module() {
        let module = parse(
            "export function add(a, b) { return a + b; }
             export function mul(a, b) { return a * b; }",
        );
        let exported = exported_set(&["add", "mul"]);
        let edges = extract_call_edges(&module, &exported);
        assert!(edges.is_empty(), "expected no edges, got: {:?}", edges);
    }

    /// With an empty exported_names set, no edges should be emitted regardless
    /// of what functions exist or call each other.
    #[test]
    fn test_no_edges_when_no_exported_names() {
        let module = parse(
            "export function greet() {}
             export function helper() { greet(); }",
        );
        let edges = extract_call_edges(&module, &HashSet::new());
        assert!(
            edges.is_empty(),
            "expected no edges with empty exported set, got: {:?}",
            edges
        );
    }
}
