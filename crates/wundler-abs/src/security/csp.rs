//! Content-Security-Policy helpers.
//!
//! Phase 2 emits **report-only** CSP — never enforcement mode.

use axum::http::{HeaderName, HeaderValue};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use wundler_graph::ChunkManifest;

/// Convert a hex-encoded SHA-256 digest (CAS format) to the base64 form
/// that CSP `script-src` directives expect inside `'sha256-…'`.
///
/// # Panics
/// Panics if `hex` is not valid hexadecimal.
pub fn hex_to_sri_b64(hex: &str) -> String {
    let bytes = hex::decode(hex).expect("CAS hash must be valid hex");
    STANDARD.encode(bytes)
}

/// Build a `Content-Security-Policy-Report-Only` header for the given manifest.
///
/// Format: `script-src 'sha256-<b64>' ... ; report-uri /csp-report`
/// The header name returned is always `content-security-policy-report-only`.
pub fn build_csp_report_only(manifest: &ChunkManifest) -> (HeaderName, HeaderValue) {
    let mut hashes: Vec<String> = manifest
        .chunks
        .iter()
        .map(|c| format!("'sha256-{}'", hex_to_sri_b64(c.hash.as_str())))
        .collect();
    hashes.sort();

    let value = if hashes.is_empty() {
        "script-src 'none'; report-uri /csp-report".to_string()
    } else {
        format!("script-src {}; report-uri /csp-report", hashes.join(" "))
    };

    let name = HeaderName::from_static("content-security-policy-report-only");
    let value = HeaderValue::from_str(&value)
        .expect("CSP header value must be ASCII (b64+digits+spaces only)");
    (name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use wundler_core::types::ContentHash;
    use wundler_graph::types::{Chunk, ChunkManifest, LoadCondition};

    fn manifest_with_chunks(hashes: &[&str]) -> ChunkManifest {
        let chunks = hashes
            .iter()
            .enumerate()
            .map(|(i, h)| Chunk {
                id: format!("chunk-{i}"),
                modules: vec![],
                hash: ContentHash((*h).to_string()),
                load_condition: LoadCondition::Initial,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            })
            .collect();
        ChunkManifest {
            build_id: "b0".to_string(),
            chunks,
            entry_chunks: HashMap::new(),
            module_index: HashMap::new(),
        }
    }

    #[test]
    fn hex_to_sri_b64_matches_known_value() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(hex_to_sri_b64(hex), "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=");
    }

    #[test]
    fn build_csp_returns_report_only_header_name() {
        let m = manifest_with_chunks(&[]);
        let (name, _) = build_csp_report_only(&m);
        assert_eq!(name.as_str(), "content-security-policy-report-only");
    }

    #[test]
    fn empty_manifest_uses_script_src_none() {
        let m = manifest_with_chunks(&[]);
        let (_, value) = build_csp_report_only(&m);
        let s = value.to_str().unwrap();
        assert!(s.contains("script-src 'none'"));
        assert!(s.contains("report-uri /csp-report"));
    }

    #[test]
    fn non_empty_manifest_emits_one_sha256_per_chunk() {
        let m = manifest_with_chunks(&[
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447",
        ]);
        let (_, value) = build_csp_report_only(&m);
        let s = value.to_str().unwrap();
        assert_eq!(s.matches("'sha256-").count(), 2);
        assert!(s.contains("'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU='"));
        assert!(s.ends_with("; report-uri /csp-report"));
    }
}
