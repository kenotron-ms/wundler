// Parser module — SWC-based source file parser.

use std::path::Path;

use anyhow::Result;
use swc_core::common::{sync::Lrc, FileName, SourceMap};
use swc_core::ecma::ast::Module;
use swc_core::ecma::parser::{lexer::Lexer, EsSyntax, Parser, StringInput, Syntax, TsSyntax};

/// Parse a JavaScript/TypeScript source string into an SWC `Module` AST.
///
/// The syntax mode is selected from the file extension:
/// - `.ts`  → TypeScript (no JSX, decorators enabled)
/// - `.tsx` → TypeScript JSX (decorators enabled)
/// - `.jsx` → ES with JSX
/// - other  → ES without JSX
pub fn parse_module(source: &str, path: &Path) -> Result<Module> {
    let cm: Lrc<SourceMap> = Default::default();
    let source_file = cm.new_source_file(
        FileName::Real(path.to_path_buf()).into(),
        source.to_string(),
    );

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let syntax = match ext {
        "ts" => Syntax::Typescript(TsSyntax {
            tsx: false,
            decorators: true,
            ..Default::default()
        }),
        "tsx" => Syntax::Typescript(TsSyntax {
            tsx: true,
            decorators: true,
            ..Default::default()
        }),
        "jsx" => Syntax::Es(EsSyntax {
            jsx: true,
            ..Default::default()
        }),
        _ => Syntax::Es(EsSyntax {
            jsx: false,
            ..Default::default()
        }),
    };

    let lexer = Lexer::new(
        syntax,
        Default::default(),
        StringInput::from(&*source_file),
        None,
    );
    let mut parser = Parser::new_from(lexer);

    parser
        .parse_module()
        .map_err(|e| anyhow::anyhow!("Parse error in {}: {:?}", path.display(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parse_typescript_succeeds() {
        let source = "export const greeting: string = 'hello';";
        let path = Path::new("test.ts");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_parse_tsx_succeeds() {
        let source = "export default function App() { return <div />; }";
        let path = Path::new("test.tsx");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_parse_javascript_succeeds() {
        let source = "import React from 'react';";
        let path = Path::new("test.js");
        let result = parse_module(source, path);
        assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_parse_invalid_syntax_returns_err() {
        let source = "function @@@broken() {}";
        let path = Path::new("broken.ts");
        let result = parse_module(source, path);
        assert!(result.is_err(), "Expected Err for invalid syntax, got Ok");
    }
}
