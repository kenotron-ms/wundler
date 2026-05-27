//! TypeScript module archetype with three variants.

use crate::archetypes::{pad_to_target, PadStyle};

#[derive(Debug, Clone, Copy)]
pub enum Variant {
    Leaf,
    Intermediate,
    Barrel,
}

/// Generate TypeScript source roughly `target_bytes` long. Deterministic in
/// `(variant, variant_seed, target_bytes)`.
pub fn generate(variant: Variant, variant_seed: u64, target_bytes: u64) -> String {
    let body = match variant {
        Variant::Leaf => leaf(variant_seed),
        Variant::Intermediate => intermediate(variant_seed),
        Variant::Barrel => barrel(variant_seed),
    };
    pad_to_target(body, target_bytes, PadStyle::CSlash)
}

fn leaf(seed: u64) -> String {
    format!(
        "export const VALUE_{seed}: number = {seed};\nexport function compute_{seed}(input: number): number {{\n\treturn input + {seed};\n}}\n"
    )
}

fn intermediate(seed: u64) -> String {
    let n_imports = 1 + (seed % 3);
    let mut s = String::new();
    for i in 0..n_imports {
        s.push_str(&format!(
            "import {{ compute_{i} }} from \"./sibling_{i}\";\n"
        ));
    }
    s.push_str(&format!(
        "export function pipeline_{seed}(x: number): number {{\n\tlet v = x;\n"
    ));
    for i in 0..n_imports {
        s.push_str(&format!("\tv = compute_{i}(v);\n"));
    }
    s.push_str(&format!("\treturn v + {seed};\n}}\n"));
    s
}

fn barrel(seed: u64) -> String {
    let n = 2 + (seed % 4);
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("export * from \"./mod_{i}\";\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use swc_core::common::{sync::Lrc, FileName, SourceMap};
    use swc_core::ecma::parser::{Parser, StringInput, Syntax, TsSyntax};

    fn parses_as_ts(source: &str) -> bool {
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(Lrc::new(FileName::Anon), source.to_string());
        let mut p = Parser::new(
            Syntax::Typescript(TsSyntax::default()),
            StringInput::from(&*fm),
            None,
        );
        p.parse_module().is_ok()
    }

    #[test]
    fn leaf_variant_parses() {
        let src = generate(Variant::Leaf, 0, 200);
        assert!(parses_as_ts(&src), "Leaf did not parse:\n{src}");
    }

    #[test]
    fn intermediate_variant_parses() {
        let src = generate(Variant::Intermediate, 1, 400);
        assert!(parses_as_ts(&src), "Intermediate did not parse:\n{src}");
    }

    #[test]
    fn barrel_variant_parses() {
        let src = generate(Variant::Barrel, 2, 250);
        assert!(parses_as_ts(&src), "Barrel did not parse:\n{src}");
    }

    #[test]
    fn output_is_near_target_size() {
        for target in [120u64, 300, 800, 2_000] {
            let src = generate(Variant::Leaf, 7, target);
            let diff = (src.len() as i64 - target as i64).abs();
            assert!(
                diff <= 16,
                "target={target}, actual={}, diff={diff}",
                src.len()
            );
        }
    }

    #[test]
    fn deterministic_for_same_seed() {
        let a = generate(Variant::Intermediate, 99, 500);
        let b = generate(Variant::Intermediate, 99, 500);
        assert_eq!(a, b);
    }
}
