//! Shared SWC parse / emit helpers.

use anyhow::{Context, Result};
use swc_core::common::sync::Lrc;
use swc_core::common::{FileName, SourceMap};
use swc_core::ecma::ast::Module;
use swc_core::ecma::codegen::{text_writer::JsWriter, Config, Emitter};
use swc_core::ecma::parser::{Parser, StringInput, Syntax, TsSyntax};

/// Parse `src` into an SWC `Module` AST.
///
/// Uses TypeScript syntax for `.ts`/`.tsx` files, ES syntax otherwise.
pub fn parse_source(name: &str, src: &str) -> Result<(Lrc<SourceMap>, Module)> {
    let cm: Lrc<SourceMap> = Default::default();
    let filename = Lrc::new(FileName::Custom(name.to_owned()));
    let fm = cm.new_source_file(filename, src.to_owned());

    let syntax = if name.ends_with(".ts") || name.ends_with(".tsx") {
        Syntax::Typescript(TsSyntax::default())
    } else {
        Syntax::Es(Default::default())
    };

    let mut parser = Parser::new(syntax, StringInput::from(&*fm), None);
    let module = parser
        .parse_module()
        .map_err(|e| anyhow::anyhow!("parse error: {:?}", e))?;

    Ok((cm, module))
}

/// Emit `module` back to a JavaScript/TypeScript string.
pub fn emit_module(cm: Lrc<SourceMap>, module: &Module) -> Result<String> {
    let mut buf: Vec<u8> = vec![];
    {
        let mut emitter = Emitter {
            cfg: Config::default(),
            comments: None,
            cm: cm.clone(),
            wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
        };
        emitter
            .emit_module(module)
            .context("SWC emit_module failed")?;
    }
    String::from_utf8(buf).context("emit produced non-UTF-8 output")
}
