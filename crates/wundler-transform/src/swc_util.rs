//! Shared SWC parse / emit helpers.

use anyhow::{Context, Result};
use swc_core::common::sync::Lrc;
use swc_core::common::{FileName, Globals, Mark, SourceMap, GLOBALS};
use swc_core::ecma::ast::{Module, Program};
use swc_core::ecma::codegen::{text_writer::JsWriter, Config, Emitter};
use swc_core::ecma::parser::{Parser, StringInput, Syntax, TsSyntax};
use swc_core::ecma::transforms::base::resolver;
use swc_core::ecma::transforms::typescript::strip;

/// Parse `src` into an SWC `Module` AST.
///
/// Uses TypeScript syntax for `.ts`/`.tsx` files, ES syntax otherwise.
pub fn parse_source(name: &str, src: &str) -> Result<(Lrc<SourceMap>, Module)> {
    let cm: Lrc<SourceMap> = Default::default();
    let filename = Lrc::new(FileName::Custom(name.to_owned()));
    let fm = cm.new_source_file(filename, src.to_owned());

    let syntax = if name.ends_with(".tsx") {
        // TSX files: TypeScript syntax with JSX enabled.
        Syntax::Typescript(TsSyntax {
            tsx: true,
            ..Default::default()
        })
    } else if name.ends_with(".ts") {
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

/// Parse a TypeScript source file, strip all type annotations, and emit clean JavaScript.
///
/// This is the correct function to use when serving `.ts`/`.tsx` files as native ESM —
/// it removes TypeScript-specific syntax so browsers can execute the result directly.
///
/// Uses the SWC `resolver` + `strip` passes inside a `GLOBALS` context.
pub fn transform_ts_to_js(name: &str, src: &str) -> Result<String> {
    let (cm, module) = parse_source(name, src)?;

    // Wrap the parsed module in a Program so we can use `Program::apply`.
    let program = Program::Module(module);

    // Apply resolver (scope analysis) then strip (type removal) inside a GLOBALS context.
    let stripped_program = GLOBALS.set(&Globals::default(), || {
        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();
        let program = program.apply(resolver(unresolved_mark, top_level_mark, true));
        program.apply(strip(unresolved_mark, top_level_mark))
    });

    // Extract the inner Module from the Program and emit.
    let stripped_module = match stripped_program {
        Program::Module(m) => m,
        Program::Script(s) => {
            // Shouldn't happen for TypeScript files, but handle gracefully.
            // Emit as-is by converting script body to a fake module.
            anyhow::bail!("unexpected Script program from TS file {name}: {s:?}")
        }
    };

    emit_module(cm, &stripped_module)
}
