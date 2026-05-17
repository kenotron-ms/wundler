//! File content archetypes used by `corpus_gen`. Each archetype emits a UTF-8
//! string padded to *approximately* a target byte size.

pub mod json_config;
pub mod markdown;
pub mod ts_module;

/// Pads `body` with a trailing block comment so the final string length (in
/// bytes) is as close as possible to `target_bytes`, while never returning
/// shorter than the original body.
pub fn pad_to_target(body: String, target_bytes: u64, style: PadStyle) -> String {
    let current = body.len() as u64;
    if current >= target_bytes {
        return body;
    }
    let needed = (target_bytes - current) as usize;

    let (open, close) = match style {
        PadStyle::CSlash => ("/* ", " */"),
        PadStyle::Hash => ("# ", ""),
        PadStyle::HtmlComment => ("<!-- ", " -->"),
        PadStyle::JsonStringField => {
            return body;
        }
    };

    let delim_len = open.len() + close.len();
    if needed <= delim_len {
        return body;
    }
    let fill_len = needed - delim_len;

    let mut out = String::with_capacity(body.len() + needed);
    out.push_str(&body);
    out.push_str(open);
    for _ in 0..fill_len {
        out.push('x');
    }
    out.push_str(close);
    out
}

#[derive(Debug, Clone, Copy)]
pub enum PadStyle {
    CSlash,
    Hash,
    HtmlComment,
    JsonStringField,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_to_target_extends_short_body() {
        let body = "let x = 1;\n".to_string();
        let out = pad_to_target(body.clone(), 200, PadStyle::CSlash);
        let diff = (out.len() as i64 - 200).abs();
        assert!(diff <= 1, "len={}, diff={}", out.len(), diff);
        assert!(out.starts_with(&body));
    }

    #[test]
    fn pad_to_target_does_not_shrink() {
        let body = "x".repeat(500);
        let out = pad_to_target(body.clone(), 100, PadStyle::CSlash);
        assert_eq!(out, body);
    }

    #[test]
    fn pad_to_target_small_diff_is_noop() {
        let body = "ab".to_string();
        let out = pad_to_target(body.clone(), 5, PadStyle::CSlash);
        assert_eq!(out, body);
    }
}
