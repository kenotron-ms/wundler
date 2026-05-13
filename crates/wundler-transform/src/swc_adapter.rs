//! SWC-based dead export stripping for a single module.

use std::collections::HashSet;

use anyhow::Result;
use swc_core::ecma::ast::{Decl, ExportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, Pat};

use crate::swc_util::{emit_module, parse_source};

/// Strip dead exports from `src`.
///
/// * Named exports (`export function foo`, `export const foo`, `export class Foo`) are
///   removed when the binding name is in `dead`.
/// * Re-export specifiers (`export { a, b } from './x'`) have dead specifiers removed;
///   if all specifiers become dead, the entire statement is removed.
/// * `export default ...` is **never** stripped, regardless of what is in `dead`.
pub fn strip_dead_exports(src: &str, dead: &HashSet<String>) -> Result<String> {
    let (cm, mut module) = parse_source("<input>.ts", src)?;
    if dead.is_empty() {
        return emit_module(cm, &module);
    }
    module.body = module
        .body
        .into_iter()
        .filter_map(|item| match item {
            ModuleItem::ModuleDecl(decl) => {
                filter_export_decl(decl, dead).map(ModuleItem::ModuleDecl)
            }
            other => Some(other),
        })
        .collect();
    emit_module(cm, &module)
}

fn filter_export_decl(decl: ModuleDecl, dead: &HashSet<String>) -> Option<ModuleDecl> {
    match decl {
        // Default exports are never stripped, regardless of what is in `dead`.
        d @ ModuleDecl::ExportDefaultDecl(_) => Some(d),
        d @ ModuleDecl::ExportDefaultExpr(_) => Some(d),

        // Named export declaration: `export function foo() {}`, `export const bar = 1`, etc.
        ModuleDecl::ExportDecl(export) => {
            let remove = decl_binding_name(&export.decl)
                .map(|name| dead.contains(&name))
                .unwrap_or(false);
            if remove {
                None
            } else {
                Some(ModuleDecl::ExportDecl(export))
            }
        }

        // Re-export bindings: `export { a, b } from './x'`
        ModuleDecl::ExportNamed(mut named) => {
            named.specifiers.retain(|spec| {
                let exported_name = export_specifier_name(spec);
                !dead.contains(&exported_name)
            });
            if named.specifiers.is_empty() {
                None
            } else {
                Some(ModuleDecl::ExportNamed(named))
            }
        }

        // `export * from './x'` — never stripped.
        d @ ModuleDecl::ExportAll(_) => Some(d),

        // All other module items (import declarations, etc.) — keep as-is.
        other => Some(other),
    }
}

/// Returns the exported name of a specifier (the name visible to importers).
fn export_specifier_name(spec: &ExportSpecifier) -> String {
    match spec {
        ExportSpecifier::Named(n) => {
            // `exported` is the alias; if absent the exported name equals the original.
            let name = n.exported.as_ref().unwrap_or(&n.orig);
            module_export_name_str(name)
        }
        ExportSpecifier::Default(_) => "default".to_string(),
        ExportSpecifier::Namespace(ns) => module_export_name_str(&ns.name),
    }
}

fn module_export_name_str(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::Ident(i) => i.sym.to_string(),
        // `Str::value` is a `Wtf8Atom` which lacks `Display`; convert via `Atom` first.
        ModuleExportName::Str(s) => s.value.to_atom_lossy().to_string(),
    }
}

/// Extract the single binding name from an export declaration.
///
/// For `export function foo()` → `Some("foo")`.
/// For `export const a = 1, b = 2` → `Some("a")` (first binding only).
///
/// **Known limitation:** for multi-binding `export const a = 1, b = 2` where only
/// `b` is dead, the entire statement is kept (returned as `Some`), not partially stripped.
fn decl_binding_name(decl: &Decl) -> Option<String> {
    match decl {
        Decl::Fn(f) => Some(f.ident.sym.to_string()),
        Decl::Class(c) => Some(c.ident.sym.to_string()),
        Decl::Var(v) => v.decls.first().and_then(|d| {
            if let Pat::Ident(ident) = &d.name {
                Some(ident.id.sym.to_string())
            } else {
                None
            }
        }),
        _ => None,
    }
}
