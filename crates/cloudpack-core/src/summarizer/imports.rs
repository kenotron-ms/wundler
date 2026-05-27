//! Import extractor — walks a parsed SWC `Module` and produces a flat list of `Import` items.
//!
//! Combines static import declarations with dynamic `import()` call expressions.

use swc_core::ecma::ast::{
    Callee, Expr, ImportDecl, ImportSpecifier, Lit, Module, ModuleDecl, ModuleItem,
};
use swc_core::ecma::visit::{Visit, VisitWith};

use crate::types::{Import, ImportKind};

/// Extract all import bindings from a parsed SWC `Module`.
///
/// Combines static (`import … from`) and dynamic (`import(…)`) imports.
pub fn extract_imports(module: &Module) -> Vec<Import> {
    let mut imports = collect_static_imports(module);
    imports.extend(collect_dynamic_imports(module));
    imports
}

// ---------------------------------------------------------------------------
// Static import collection
// ---------------------------------------------------------------------------

fn collect_static_imports(module: &Module) -> Vec<Import> {
    let mut imports = Vec::new();

    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import_decl)) = item else {
            continue;
        };

        collect_import_decl(import_decl, &mut imports);
    }

    imports
}

fn collect_import_decl(import_decl: &ImportDecl, imports: &mut Vec<Import>) {
    // Skip type-only imports (TypeScript `import type { … }`)
    if import_decl.type_only {
        return;
    }

    let specifier = import_decl.src.value.to_string_lossy().into_owned();

    // Side-effect import: `import 'module'` — no specifiers
    if import_decl.specifiers.is_empty() {
        imports.push(Import {
            specifier,
            kind: ImportKind::SideEffect,
            bindings: vec![],
            is_dynamic: false,
        });
        return;
    }

    // Collapse all specifiers into one Import entry per declaration.
    let mut kind = ImportKind::SideEffect; // will be overwritten
    let mut bindings: Vec<String> = Vec::new();
    let mut kind_set = false;

    for spec in &import_decl.specifiers {
        match spec {
            ImportSpecifier::Default(default_spec) => {
                // Default sets kind only if Named hasn't been seen yet.
                if !kind_set || kind != ImportKind::Named {
                    kind = ImportKind::Default;
                }
                kind_set = true;
                bindings.push(default_spec.local.sym.to_string());
            }
            ImportSpecifier::Named(named_spec) => {
                // Named always wins — overrides Default.
                kind = ImportKind::Named;
                kind_set = true;
                bindings.push(named_spec.local.sym.to_string());
            }
            ImportSpecifier::Namespace(ns_spec) => {
                kind = ImportKind::Namespace;
                kind_set = true;
                bindings.push(ns_spec.local.sym.to_string());
            }
        }
    }

    imports.push(Import {
        specifier,
        kind,
        bindings,
        is_dynamic: false,
    });
}

// ---------------------------------------------------------------------------
// Dynamic import collection
// ---------------------------------------------------------------------------

struct DynamicImportVisitor {
    imports: Vec<Import>,
}

impl Visit for DynamicImportVisitor {
    fn visit_call_expr(&mut self, call: &swc_core::ecma::ast::CallExpr) {
        if let Callee::Import(_) = &call.callee {
            if let Some(first_arg) = call.args.first() {
                if let Expr::Lit(Lit::Str(s)) = first_arg.expr.as_ref() {
                    self.imports.push(Import {
                        specifier: s.value.to_string_lossy().into_owned(),
                        kind: ImportKind::Dynamic,
                        bindings: vec![],
                        is_dynamic: true,
                    });
                }
                // Non-literal specifiers are intentionally ignored.
            }
        }
        // Recurse into child nodes.
        call.visit_children_with(self);
    }
}

fn collect_dynamic_imports(module: &Module) -> Vec<Import> {
    let mut visitor = DynamicImportVisitor {
        imports: Vec::new(),
    };
    module.visit_with(&mut visitor);
    visitor.imports
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
    fn test_extract_named_imports() {
        let module = parse("import { useState, useEffect } from 'react';");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.specifier, "react");
        assert_eq!(imp.kind, ImportKind::Named);
        assert!(imp.bindings.contains(&"useState".to_string()));
        assert!(imp.bindings.contains(&"useEffect".to_string()));
        assert!(!imp.is_dynamic);
    }

    #[test]
    fn test_extract_default_import() {
        let module = parse("import React from 'react';");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.specifier, "react");
        assert_eq!(imp.kind, ImportKind::Default);
        assert_eq!(imp.bindings, vec!["React"]);
        assert!(!imp.is_dynamic);
    }

    #[test]
    fn test_extract_namespace_import() {
        let module = parse("import * as lodash from 'lodash';");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.specifier, "lodash");
        assert_eq!(imp.kind, ImportKind::Namespace);
        assert_eq!(imp.bindings, vec!["lodash"]);
        assert!(!imp.is_dynamic);
    }

    #[test]
    fn test_extract_side_effect_import() {
        let module = parse("import 'reflect-metadata';");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.specifier, "reflect-metadata");
        assert_eq!(imp.kind, ImportKind::SideEffect);
        assert!(imp.bindings.is_empty());
        assert!(!imp.is_dynamic);
    }

    #[test]
    fn test_extract_mixed_import() {
        // `import React, { useState } from 'react'` — one entry, both bindings,
        // kind=Named because Named overrides Default.
        let module = parse("import React, { useState } from 'react';");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1, "expected a single Import entry");
        let imp = &imports[0];
        assert_eq!(imp.specifier, "react");
        assert_eq!(imp.kind, ImportKind::Named);
        assert!(imp.bindings.contains(&"React".to_string()));
        assert!(imp.bindings.contains(&"useState".to_string()));
        assert!(!imp.is_dynamic);
    }

    #[test]
    fn test_extract_dynamic_import_literal() {
        let module = parse("async function load() { return import('./heavy'); }");
        let imports = extract_imports(&module);
        assert_eq!(imports.len(), 1);
        let imp = &imports[0];
        assert_eq!(imp.specifier, "./heavy");
        assert_eq!(imp.kind, ImportKind::Dynamic);
        assert!(imp.bindings.is_empty());
        assert!(imp.is_dynamic);
    }

    #[test]
    fn test_dynamic_import_non_literal_ignored() {
        let module = parse("async function load(name) { return import(name); }");
        let imports = extract_imports(&module);
        assert_eq!(
            imports.len(),
            0,
            "non-literal dynamic import should be ignored"
        );
    }
}
