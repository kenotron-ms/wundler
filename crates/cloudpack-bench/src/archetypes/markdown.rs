//! Markdown archetype — H1 followed by filler text.

/// Generates a Markdown document sized to roughly `target_bytes`.
pub fn generate(variant_seed: u64, target_bytes: u64) -> String {
    let head = format!("# Document {variant_seed}\n\n");
    let tail = "\n";
    let scaffold_len = (head.len() + tail.len()) as u64;
    let fill_len = target_bytes.saturating_sub(scaffold_len) as usize;

    let mut out = String::with_capacity(target_bytes as usize + 16);
    out.push_str(&head);
    for _ in 0..fill_len {
        out.push('x');
    }
    out.push_str(tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_h1() {
        let s = generate(0, 200);
        assert!(s.starts_with("# "));
    }

    #[test]
    fn near_target_size() {
        for target in [100u64, 400, 1_200] {
            let s = generate(1, target);
            let diff = (s.len() as i64 - target as i64).abs();
            assert!(diff <= 8, "target={target}, actual={}", s.len());
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(generate(9, 500), generate(9, 500));
    }
}
