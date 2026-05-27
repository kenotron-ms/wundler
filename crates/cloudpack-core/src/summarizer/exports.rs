//! Export extractor — walks a parsed SWC `Module` and produces a flat list of `Export` items.

use swc_core::ecma::ast::{
    Decl, ExportSpecifier, Module, ModuleDecl, ModuleExportName, ModuleItem, ObjectPatProp, Pat,
    TsModuleName,
};

use crate::types::{Export, ExportKind};

/// Extract all export bindings from a parsed SWC `Module`.
pub fn extract_exports(module: &Module) -> Vec<Export> {
    let mut exports = Vec::new();

    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };

        match decl {
            ModuleDecl::ExportDecl(export_decl) => {
                collect_decl_names(&export_decl.decl, &mut exports);
            }
            ModuleDecl::ExportDefaultDecl(_) | ModuleDecl::ExportDefaultExpr(_) => {
                exports.push(Export {
                    name: "default".to_string(),
                    kind: ExportKind::Default,
                    source: None,
                });
            }
            ModuleDecl::ExportNamed(named_export) => {
                let source = named_export
                    .src
                    .as_ref()
                    .map(|s| s.value.to_string_lossy().into_owned());
                let kind = if source.is_some() {
                    ExportKind::ReExport
                } else {
                    ExportKind::Named
                };
                for spec in &named_export.specifiers {
                    if let ExportSpecifier::Named(named_spec) = spec {
                        let name = if let Some(exported) = &named_spec.exported {
                            module_export_name_to_str(exported)
                        } else {
                            module_export_name_to_str(&named_spec.orig)
                        };
                        exports.push(Export {
                            name,
                            kind: kind.clone(),
                            source: source.clone(),
                        });
                    }
                }
            }
            ModuleDecl::ExportAll(export_all) => {
                exports.push(Export {
                    name: "*".to_string(),
                    kind: ExportKind::StarExport,
                    source: Some(export_all.src.value.to_string_lossy().into_owned()),
                });
            }
            _ => {}
        }
    }

    exports
}

fn collect_decl_names(decl: &Decl, exports: &mut Vec<Export>) {
    match decl {
        Decl::Fn(fn_decl) => exports.push(named(fn_decl.ident.sym.to_string())),
        Decl::Class(class_decl) => exports.push(named(class_decl.ident.sym.to_string())),
        Decl::Var(var_decl) => {
            for declarator in &var_decl.decls {
                collect_pat_names(&declarator.name, exports);
            }
        }
        Decl::Using(using_decl) => {
            for declarator in &using_decl.decls {
                collect_pat_names(&declarator.name, exports);
            }
        }
        Decl::TsInterface(ts_interface) => exports.push(named(ts_interface.id.sym.to_string())),
        Decl::TsTypeAlias(ts_type_alias) => exports.push(named(ts_type_alias.id.sym.to_string())),
        Decl::TsEnum(ts_enum) => exports.push(named(ts_enum.id.sym.to_string())),
        Decl::TsModule(ts_module) => {
            let name = match &ts_module.id {
                TsModuleName::Ident(ident) => ident.sym.to_string(),
                TsModuleName::Str(str_val) => str_val.value.to_string_lossy().into_owned(),
            };
            exports.push(named(name));
        }
    }
}

fn collect_pat_names(pat: &Pat, exports: &mut Vec<Export>) {
    match pat {
        Pat::Ident(binding_ident) => exports.push(named(binding_ident.id.sym.to_string())),
        Pat::Array(array_pat) => {
            for p in array_pat.elems.iter().flatten() {
                collect_pat_names(p, exports);
            }
        }
        Pat::Object(object_pat) => {
            for prop in &object_pat.props {
                match prop {
                    ObjectPatProp::KeyValue(kv) => collect_pat_names(&kv.value, exports),
                    ObjectPatProp::Assign(assign) => {
                        exports.push(named(assign.key.id.sym.to_string()))
                    }
                    ObjectPatProp::Rest(rest) => collect_pat_names(&rest.arg, exports),
                }
            }
        }
        Pat::Rest(rest_pat) => collect_pat_names(&rest_pat.arg, exports),
        Pat::Assign(assign_pat) => collect_pat_names(&assign_pat.left, exports),
        Pat::Invalid(_) | Pat::Expr(_) => {}
    }
}

/// Convert a `ModuleExportName` node to its string representation.
fn module_export_name_to_str(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::Ident(ident) => ident.sym.to_string(),
        ModuleExportName::Str(str_val) => str_val.value.to_string_lossy().into_owned(),
    }
}

/// Convenience constructor for a `Named` export with no source.
fn named(name: String) -> Export {
    Export {
        name,
        kind: ExportKind::Named,
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::parser::parse_module;
    use std::path::Path;

    fn parse(src: &str) -> Module {
        parse_module(src, Path::new("test.ts")).expect("parse failed")
    }

    #[test]
    fn test_extract_named_exports() {
        let module = parse(
            "export const VERSION = '1.0'; export function greet() {} export class Greeter {}",
        );
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 3);
        assert!(exports
            .iter()
            .any(|e| e.name == "VERSION" && e.kind == ExportKind::Named && e.source.is_none()));
        assert!(exports
            .iter()
            .any(|e| e.name == "greet" && e.kind == ExportKind::Named && e.source.is_none()));
        assert!(exports
            .iter()
            .any(|e| e.name == "Greeter" && e.kind == ExportKind::Named && e.source.is_none()));
    }

    #[test]
    fn test_extract_default_export_function() {
        let module = parse("export default function App() {}");
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
        assert!(exports[0].source.is_none());
    }

    #[test]
    fn test_extract_default_export_expression() {
        let module = parse("const x = 1; export default x;");
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "default");
        assert_eq!(exports[0].kind, ExportKind::Default);
    }

    #[test]
    fn test_extract_reexports_from_module() {
        let module = parse("export { useState, useEffect } from 'react';");
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 2);
        assert!(exports.iter().all(|e| e.kind == ExportKind::ReExport));
        assert!(exports
            .iter()
            .all(|e| e.source == Some("react".to_string())));
        assert!(exports.iter().any(|e| e.name == "useState"));
        assert!(exports.iter().any(|e| e.name == "useEffect"));
    }

    #[test]
    fn test_extract_reexport_with_rename() {
        let module = parse("export { greet as greetUser } from './greeter';");
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "greetUser");
        assert_eq!(exports[0].kind, ExportKind::ReExport);
        assert_eq!(exports[0].source, Some("./greeter".to_string()));
    }

    #[test]
    fn test_extract_star_export() {
        let module = parse("export * from './utils';");
        let exports = extract_exports(&module);
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "*");
        assert_eq!(exports[0].kind, ExportKind::StarExport);
        assert_eq!(exports[0].source, Some("./utils".to_string()));
    }

    #[test]
    fn test_no_exports_returns_empty() {
        let module = parse("const x = 1; function foo() {}");
        let exports = extract_exports(&module);
        assert!(exports.is_empty());
    }
}
