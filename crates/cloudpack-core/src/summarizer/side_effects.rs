//! Side-effect analyser — inspects top-level statements of a parsed SWC `Module`
//! and classifies whether the module has no side effects, possible side effects,
//! or definite side effects.

use swc_core::ecma::ast::{Decl, Expr, Module, ModuleItem, Stmt};

use crate::types::SideEffectMarker;

/// The known ambient global object identifiers that indicate a definite side
/// effect when assigned to as a member expression.
const AMBIENT_GLOBALS: &[&str] = &[
    "window",
    "document",
    "globalThis",
    "global",
    "self",
    "navigator",
];

/// Analyse the top-level statements of `module` and return a `SideEffectMarker`.
///
/// - **NONE**: module contains only function/class/type declarations,
///   import/export statements, and pure variable initialisers.
/// - **DEFINITE**: a top-level expression statement writes to a member of a
///   known ambient global (e.g. `window.APP_VERSION = '2.0.0'`).
/// - **POSSIBLE**: anything else at the top level.
pub fn analyze_side_effects(module: &Module) -> SideEffectMarker {
    for item in &module.body {
        match item {
            // Import / export declarations are never side effects.
            ModuleItem::ModuleDecl(_) => continue,
            ModuleItem::Stmt(stmt) => match stmt {
                Stmt::Decl(decl) => match decl {
                    // Pure declarations — no side effects.
                    Decl::Fn(_)
                    | Decl::Class(_)
                    | Decl::TsInterface(_)
                    | Decl::TsTypeAlias(_)
                    | Decl::TsEnum(_)
                    | Decl::TsModule(_) => continue,

                    // Variable declarations — ok only when every initialiser is pure.
                    Decl::Var(var_decl) => {
                        for declarator in &var_decl.decls {
                            if let Some(init) = &declarator.init {
                                if !is_pure_expr(init) {
                                    return SideEffectMarker::Possible {
                                        reason: "top-level variable with non-literal initializer"
                                            .to_string(),
                                    };
                                }
                            }
                        }
                    }

                    // Any other declaration (e.g. `using`) — treat as possible.
                    _ => {
                        return SideEffectMarker::Possible {
                            reason: "top-level declaration with possible side effects".to_string(),
                        };
                    }
                },

                // Expression statements — check for definite global writes first.
                Stmt::Expr(expr_stmt) => {
                    if let Expr::Assign(assign) = expr_stmt.expr.as_ref() {
                        if is_global_member_assignment(assign) {
                            return SideEffectMarker::Definite;
                        }
                    }
                    return SideEffectMarker::Possible {
                        reason: "top-level expression statement".to_string(),
                    };
                }

                // All other statement kinds (if, for, try, return, …) — possible.
                _ => {
                    return SideEffectMarker::Possible {
                        reason: "top-level non-declaration statement".to_string(),
                    };
                }
            },
        }
    }
    SideEffectMarker::None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `assign` writes to a member of a known ambient global
/// (e.g. `window.APP_VERSION = '2.0.0'`).
fn is_global_member_assignment(assign: &swc_core::ecma::ast::AssignExpr) -> bool {
    use swc_core::ecma::ast::{AssignTarget, SimpleAssignTarget};

    let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left else {
        return false;
    };
    let Expr::Ident(ident) = member.obj.as_ref() else {
        return false;
    };
    AMBIENT_GLOBALS.contains(&ident.sym.as_ref())
}

/// Returns `true` for "pure" initialisers that cannot produce observable side
/// effects: literals, arrow functions, function expressions, and identifiers.
fn is_pure_expr(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Lit(_) | Expr::Arrow(_) | Expr::Fn(_) | Expr::Ident(_)
    )
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
    fn test_function_only_module_is_none() {
        let module = parse("function greet(name) { return 'Hello, ' + name; }");
        assert_eq!(analyze_side_effects(&module), SideEffectMarker::None);
    }

    #[test]
    fn test_class_only_module_is_none() {
        let module = parse("class Animal { constructor(name) { this.name = name; } }");
        assert_eq!(analyze_side_effects(&module), SideEffectMarker::None);
    }

    #[test]
    fn test_pure_literal_const_is_none() {
        let module = parse("const VERSION = '1.0.0'; const MAX_SIZE = 100;");
        assert_eq!(analyze_side_effects(&module), SideEffectMarker::None);
    }

    #[test]
    fn test_top_level_call_is_possible() {
        // `computeValue()` is a call expression — not a pure initialiser.
        let module = parse("const result = computeValue();");
        assert!(
            matches!(
                analyze_side_effects(&module),
                SideEffectMarker::Possible { .. }
            ),
            "expected Possible for top-level call initialiser"
        );
    }

    #[test]
    fn test_top_level_expression_statement_is_possible() {
        // `console.log(…)` is a top-level call expression statement.
        let module = parse("console.log('hello');");
        assert!(
            matches!(
                analyze_side_effects(&module),
                SideEffectMarker::Possible { .. }
            ),
            "expected Possible for top-level expression statement"
        );
    }

    #[test]
    fn test_window_property_write_is_definite() {
        let module = parse("window.APP_VERSION = '2.0.0';");
        assert_eq!(analyze_side_effects(&module), SideEffectMarker::Definite);
    }

    #[test]
    fn test_document_property_write_is_definite() {
        let module = parse("document.title = 'My App';");
        assert_eq!(analyze_side_effects(&module), SideEffectMarker::Definite);
    }
}
