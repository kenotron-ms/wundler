//! Shared SWC parse / emit helpers.

use anyhow::{Context, Result};
use swc_core::common::sync::Lrc;
use swc_core::common::{FileName, Globals, Mark, SourceMap, GLOBALS};
use swc_core::ecma::ast::{Module, Program};
use swc_core::ecma::codegen::{text_writer::JsWriter, Config, Emitter};
use swc_core::ecma::parser::{Parser, StringInput, Syntax, TsSyntax};
use swc_core::ecma::transforms::base::resolver;
use swc_core::common::comments::NoopComments;
use swc_core::ecma::transforms::react::{self as react_transform, Options as ReactOptions, Runtime};
use swc_core::ecma::transforms::typescript::strip;

/// Parse `src` into an SWC `Module` AST.
///
/// Uses TypeScript syntax for `.ts`/`.tsx` files, ES syntax otherwise.
pub fn parse_source(name: &str, src: &str) -> Result<(Lrc<SourceMap>, Module)> {
    let cm: Lrc<SourceMap> = Default::default();
    let filename = Lrc::new(FileName::Custom(name.to_owned()));
    let fm = cm.new_source_file(filename, src.to_owned());

    let is_tsx = name.ends_with(".tsx") || name.ends_with(".jsx");

    let syntax = if is_tsx {
        // TSX/JSX files: TypeScript syntax with JSX enabled.
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

/// Parse a TypeScript source file, strip all type annotations, convert JSX to
/// `React.createElement` calls (for `.tsx`/`.jsx` files), and emit clean JavaScript.
///
/// This is the correct function to use when serving `.ts`/`.tsx` files as native ESM —
/// it removes TypeScript-specific syntax so browsers can execute the result directly.
///
/// Transform order:
/// 1. `resolver` — scope analysis (marks bindings)
/// 2. JSX transform (`.tsx`/`.jsx` only) — converts JSX to `React.createElement`
///    using the **Classic** runtime so no `jsx-runtime` import is injected.
/// 3. `strip` — removes TypeScript type annotations.
///
/// Uses the SWC `resolver` + optional react + `strip` passes inside a `GLOBALS` context.
pub fn transform_ts_to_js(name: &str, src: &str) -> Result<String> {
    let (cm, module) = parse_source(name, src)?;

    let is_tsx = name.ends_with(".tsx") || name.ends_with(".jsx");

    // Wrap the parsed module in a Program so we can use `Program::apply`.
    let program = Program::Module(module);

    // Apply transforms inside a GLOBALS context (required by SWC).
    let stripped_program = GLOBALS.set(&Globals::default(), || {
        let unresolved_mark = Mark::new();
        let top_level_mark = Mark::new();

        // Step 1: Resolve marks (scope analysis).
        let program = program.apply(resolver(unresolved_mark, top_level_mark, true));

        // Step 2: JSX transform — MUST run before TypeScript stripping.
        // Classic runtime emits `React.createElement(...)` with no extra import.
        let program = if is_tsx {
            program.apply(react_transform::react::<NoopComments>(
                cm.clone(),
                None,
                ReactOptions {
                    runtime: Some(Runtime::Classic),
                    ..Default::default()
                },
                unresolved_mark,
                top_level_mark,
            ))
        } else {
            program
        };

        // Step 3: Strip TypeScript types.
        program.apply(strip(unresolved_mark, top_level_mark))
    });

    // Extract the inner Module from the Program and emit.
    let stripped_module = match stripped_program {
        Program::Module(m) => m,
        Program::Script(s) => {
            // Shouldn't happen for TypeScript files, but handle gracefully.
            anyhow::bail!("unexpected Script program from TS file {name}: {s:?}")
        }
    };

    emit_module(cm, &stripped_module)
}
