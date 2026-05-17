//! Hand-rolled Prometheus text exposition format.
//!
//! No SDK — ~100 LoC of format! calls per the design spec.

use std::sync::atomic::Ordering;

use super::Metrics;

/// Render the Prometheus text exposition for `metrics`.
///
/// Produces the standard `# HELP`, `# TYPE`, and metric lines expected by
/// Prometheus scrapers. Format: text/plain; version=0.0.4
pub fn render(metrics: &Metrics) -> String {
    let mut out = String::new();

    // Manifest requests
    let manifest_requests = metrics.manifest_requests_total.load(Ordering::Relaxed);
    out.push_str("# HELP wundler_manifest_requests_total Manifest requests served.\n");
    out.push_str("# TYPE wundler_manifest_requests_total counter\n");
    out.push_str(&format!("wundler_manifest_requests_total {manifest_requests}\n"));

    // Manifest bytes
    let manifest_bytes = metrics.manifest_bytes_total.load(Ordering::Relaxed);
    out.push_str("# HELP wundler_manifest_bytes_total Total bytes served in manifest responses.\n");
    out.push_str("# TYPE wundler_manifest_bytes_total counter\n");
    out.push_str(&format!("wundler_manifest_bytes_total {manifest_bytes}\n"));

    // Cache hits / misses
    let cache_hits = metrics.cache_hits_total.load(Ordering::Relaxed);
    out.push_str("# HELP wundler_cache_hits_total Cache hits.\n");
    out.push_str("# TYPE wundler_cache_hits_total counter\n");
    out.push_str(&format!("wundler_cache_hits_total {cache_hits}\n"));

    let cache_misses = metrics.cache_misses_total.load(Ordering::Relaxed);
    out.push_str("# HELP wundler_cache_misses_total Cache misses.\n");
    out.push_str("# TYPE wundler_cache_misses_total counter\n");
    out.push_str(&format!("wundler_cache_misses_total {cache_misses}\n"));

    // Chunk errors — one labelled counter per (build_id, chunk_id, error_type)
    out.push_str("# HELP wundler_chunk_errors_total Chunk load errors reported by SW.\n");
    out.push_str("# TYPE wundler_chunk_errors_total counter\n");
    for entry in metrics.chunk_errors.iter() {
        let key = entry.key();
        let val = entry.value().load(Ordering::Relaxed);
        let error_type = error_type_label(&key.error_type);
        // Escape label values: only quote, newline, backslash, and carriage return
        // need escaping in the text format; build_id and chunk_id are hex/alphanumeric.
        let build_id = escape_label_value(&key.build_id);
        let chunk_id = escape_label_value(&key.chunk_id);
        out.push_str(&format!(
            "wundler_chunk_errors_total{{build_id=\"{build_id}\",chunk_id=\"{chunk_id}\",error_type=\"{error_type}\"}} {val}\n"
        ));
    }

    out
}

fn error_type_label(et: &super::ErrorType) -> &'static str {
    match et {
        super::ErrorType::LoadFailed => "load_failed",
        super::ErrorType::NetworkTimeout => "network_timeout",
        super::ErrorType::IntegrityMismatch => "integrity_mismatch",
    }
}

/// Minimal label-value escaping for Prometheus text format.
fn escape_label_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{ChunkErrorKey, ErrorType, Metrics};

    #[test]
    fn render_includes_manifest_request_counter() {
        let m = Metrics::new();
        m.manifest_requests_total.fetch_add(42, std::sync::atomic::Ordering::Relaxed);
        let out = render(&m);
        assert!(out.contains("wundler_manifest_requests_total 42"));
    }

    #[test]
    fn render_includes_chunk_error_with_labels() {
        let m = Metrics::new();
        m.increment_chunk_error(ChunkErrorKey {
            build_id: "abc123".to_string(),
            chunk_id: "main-chunk".to_string(),
            error_type: ErrorType::LoadFailed,
        });
        let out = render(&m);
        assert!(out.contains("wundler_chunk_errors_total{"), "should have labelled metric");
        assert!(out.contains("build_id=\"abc123\""));
        assert!(out.contains("error_type=\"load_failed\""));
    }

    #[test]
    fn render_empty_metrics_has_zero_counters() {
        let m = Metrics::new();
        let out = render(&m);
        assert!(out.contains("wundler_manifest_requests_total 0"));
        // No chunk_error lines when no errors recorded.
        assert!(!out.contains("wundler_chunk_errors_total{"));
    }

    #[test]
    fn label_value_escapes_special_chars() {
        assert_eq!(escape_label_value(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }
}
