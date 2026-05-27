//! JSON config archetype — emits a small object whose `filler` string field
//! is grown to absorb the target padding.

/// Generates a JSON object string sized to roughly `target_bytes`.
pub fn generate(variant_seed: u64, target_bytes: u64) -> String {
    let head = format!(
        "{{\n  \"name\": \"pkg_{seed}\",\n  \"version\": \"0.0.{seed}\",\n  \"filler\": \"",
        seed = variant_seed
    );
    let tail = "\"\n}\n";
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
    fn parses_as_json() {
        let s = generate(0, 300);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v.is_object());
    }

    #[test]
    fn near_target_size() {
        for target in [150u64, 500, 1_500] {
            let s = generate(3, target);
            let diff = (s.len() as i64 - target as i64).abs();
            assert!(diff <= 8, "target={target}, actual={}", s.len());
        }
    }

    #[test]
    fn deterministic() {
        assert_eq!(generate(42, 400), generate(42, 400));
    }
}
