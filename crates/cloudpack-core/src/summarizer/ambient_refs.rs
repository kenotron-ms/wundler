//! Ambient reference extractor — walks a parsed SWC `Module` and collects
//! references to well-known browser and Node.js global objects.

use std::collections::HashSet;

use swc_core::ecma::ast::{Expr, MemberExpr, MemberProp, Module};
use swc_core::ecma::visit::{Visit, VisitWith};

/// The known ambient global object identifiers (same set as the side-effects
/// analyser): window, document, globalThis, global, self, navigator.
const AMBIENT_GLOBALS: &[&str] = &[
    "window",
    "document",
    "globalThis",
    "global",
    "self",
    "navigator",
];

/// Extract all ambient global references from a parsed `Module`.
///
/// Returns a deduplicated, insertion-ordered list of reference strings.
/// Member accesses are rendered as `"obj.prop"` (e.g. `"window.APP_VERSION"`);
/// bare global identifiers are rendered as just the name (e.g. `"window"`).
pub fn extract_ambient_refs(module: &Module) -> Vec<String> {
    let mut visitor = AmbientRefVisitor {
        seen: HashSet::new(),
        refs: Vec::new(),
    };
    module.visit_with(&mut visitor);
    visitor.refs
}

// ---------------------------------------------------------------------------
// Visitor
// ---------------------------------------------------------------------------

struct AmbientRefVisitor {
    seen: HashSet<String>,
    refs: Vec<String>,
}

impl AmbientRefVisitor {
    /// Insert `s` into `refs` only if it has not been seen before, preserving
    /// insertion order.
    fn record(&mut self, s: String) {
        if self.seen.insert(s.clone()) {
            self.refs.push(s);
        }
    }
}

impl Visit for AmbientRefVisitor {
    /// Handle expression nodes.
    ///
    /// - `Expr::Member`: when the object is an ambient global ident, record the
    ///   ref (`"window.prop"` or just `"window"` for computed access) and
    ///   **stop recursing** so the bare object name is not also recorded.
    ///   When the object is not an ambient ident, recurse into children (which
    ///   will trigger `visit_member_expr` for the inner `MemberExpr`).
    /// - `Expr::Ident`: record bare name when it is an ambient global.
    /// - Everything else: recurse via `visit_children_with`.
    fn visit_expr(&mut self, n: &Expr) {
        match n {
            Expr::Member(member) => {
                if let Expr::Ident(obj) = member.obj.as_ref() {
                    if AMBIENT_GLOBALS.contains(&obj.sym.as_ref()) {
                        let ref_str = match &member.prop {
                            MemberProp::Ident(prop) => format!("{}.{}", obj.sym, prop.sym),
                            // Computed access like window["x"] — record just the object name.
                            _ => obj.sym.to_string(),
                        };
                        self.record(ref_str);
                        // Do NOT recurse — avoid also recording the bare obj name.
                        return;
                    }
                }
                // Object is not an ambient ident — recurse into children.
                // This propagates to `visit_member_expr` for the inner MemberExpr.
                n.visit_children_with(self);
            }
            Expr::Ident(ident) => {
                if AMBIENT_GLOBALS.contains(&ident.sym.as_ref()) {
                    self.record(ident.sym.to_string());
                }
                // Identifiers have no children to recurse into.
            }
            _ => n.visit_children_with(self),
        }
    }

    /// Handle `MemberExpr` nodes that appear outside of `Expr` context — most
    /// importantly, the **left-hand side of assignments** such as
    /// `window.APP_VERSION = '1.0.0'`, where the LHS is an `AssignTarget` and
    /// therefore never reaches `visit_expr`.
    ///
    /// The logic mirrors `visit_expr`'s `Expr::Member` arm exactly.
    fn visit_member_expr(&mut self, n: &MemberExpr) {
        if let Expr::Ident(obj) = n.obj.as_ref() {
            if AMBIENT_GLOBALS.contains(&obj.sym.as_ref()) {
                let ref_str = match &n.prop {
                    MemberProp::Ident(prop) => format!("{}.{}", obj.sym, prop.sym),
                    _ => obj.sym.to_string(),
                };
                self.record(ref_str);
                // Do NOT recurse.
                return;
            }
        }
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

    #[test]
    fn test_detects_window_property_access() {
        let module = parse("window.APP_VERSION = '1.0.0';");
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.contains(&"window.APP_VERSION".to_string()),
            "expected 'window.APP_VERSION' in refs, got: {:?}",
            refs
        );
    }

    #[test]
    fn test_detects_document_property() {
        let module = parse("document.title = 'My App';");
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.contains(&"document.title".to_string()),
            "expected 'document.title' in refs, got: {:?}",
            refs
        );
    }

    #[test]
    fn test_detects_global_this() {
        let module = parse("globalThis.process;");
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.contains(&"globalThis.process".to_string()),
            "expected 'globalThis.process' in refs, got: {:?}",
            refs
        );
    }

    #[test]
    fn test_pure_function_has_no_ambient_refs() {
        let module = parse("export function add(a, b) { return a + b; }");
        let refs = extract_ambient_refs(&module);
        assert!(
            refs.is_empty(),
            "expected no ambient refs for pure function, got: {:?}",
            refs
        );
    }

    #[test]
    fn test_deduplicates_repeated_global() {
        let module = parse("window.X; window.X; window.Y;");
        let refs = extract_ambient_refs(&module);
        let x_count = refs.iter().filter(|r| r.as_str() == "window.X").count();
        assert_eq!(
            x_count, 1,
            "expected 'window.X' to appear exactly once, got refs: {:?}",
            refs
        );
        assert!(
            refs.contains(&"window.Y".to_string()),
            "expected 'window.Y' in refs, got: {:?}",
            refs
        );
    }
}
