//! HTML rendering helpers for Cloudpack-bundled apps.

pub mod sri;

pub use sri::hex_to_sri_b64;

use cloudpack_graph::types::{ChunkManifest, LoadCondition};

/// Render `<script src="..." integrity="sha256-..." crossorigin defer>` tags
/// for every chunk referenced by `entry` in `manifest`.
///
/// `cdn_base_url` must NOT have a trailing slash.
///
/// Returns an empty string if the entry point is not found.
pub fn render_script_tags(manifest: &ChunkManifest, entry: &str, cdn_base_url: &str) -> String {
    let chunk_ids = match manifest.entry_chunks.get(entry) {
        Some(ids) => ids,
        None => return String::new(),
    };

    let mut out = String::new();
    for chunk_id in chunk_ids {
        // Find the chunk to get the hash.
        let chunk = match manifest.chunks.iter().find(|c| &c.id == chunk_id) {
            Some(c) => c,
            None => continue,
        };
        // Only emit script tags for Initial-load chunks (not Lazy or Prefetch).
        if !matches!(chunk.load_condition, LoadCondition::Initial) {
            continue;
        }
        let hash_hex = chunk.hash.as_str();
        let sri_b64 = hex_to_sri_b64(hash_hex);
        let url = format!("{cdn_base_url}/chunks/{hash_hex}.js");
        out.push_str(&format!(
            "<script src=\"{url}\" integrity=\"sha256-{sri_b64}\" crossorigin defer></script>\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use cloudpack_core::types::ContentHash;
    use cloudpack_graph::types::{Chunk, ChunkManifest, LoadCondition};

    fn manifest_with_chunks() -> ChunkManifest {
        let hash0 = ContentHash(
            "e3b0c44298fc1c149afbf4c8996fb924".to_string()
                + "27ae41e4649b934ca495991b7852b855",
        );
        let hash1 = ContentHash(
            "a948904f2f0f479b8f8197694b30184b".to_string()
                + "0d2ed1c1cd2a1ec0fb85d299a192a447",
        );

        let chunks = vec![
            Chunk {
                id: "chunk-a".to_string(),
                modules: vec![],
                hash: hash0.clone(),
                load_condition: LoadCondition::Initial,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            },
            Chunk {
                id: "chunk-b".to_string(),
                modules: vec![],
                hash: hash1.clone(),
                load_condition: LoadCondition::Lazy,
                co_request_score: None,
                median_load_order: None,
                suggested_merge: None,
            },
        ];

        let mut entry_chunks = HashMap::new();
        entry_chunks.insert(
            "app".to_string(),
            vec!["chunk-a".to_string(), "chunk-b".to_string()],
        );

        let mut module_index = HashMap::new();
        module_index.insert(hash0, "chunk-a".to_string());
        module_index.insert(hash1, "chunk-b".to_string());

        ChunkManifest {
            build_id: "test-build".to_string(),
            chunks,
            entry_chunks,
            module_index,
        }
    }

    #[test]
    fn emits_script_tag_for_initial_chunk() {
        let manifest = manifest_with_chunks();
        let html = render_script_tags(&manifest, "app", "https://cdn.example.com");
        assert!(
            html.contains("<script src=\"https://cdn.example.com/chunks/"),
            "should contain script tag"
        );
        assert!(html.contains("integrity=\"sha256-"), "should contain SRI hash");
        assert!(html.contains("crossorigin defer"), "should contain crossorigin and defer");
    }

    #[test]
    fn omits_lazy_chunks() {
        let manifest = manifest_with_chunks();
        let html = render_script_tags(&manifest, "app", "https://cdn.example.com");
        // chunk-b is Lazy — should not appear
        let hash1 = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";
        assert!(!html.contains(hash1), "lazy chunk must not appear in initial script tags");
    }

    #[test]
    fn unknown_entry_returns_empty_string() {
        let manifest = manifest_with_chunks();
        let html = render_script_tags(&manifest, "nonexistent", "https://cdn.example.com");
        assert!(html.is_empty());
    }

    #[test]
    fn sri_hash_matches_chunk_hash() {
        let manifest = manifest_with_chunks();
        let html = render_script_tags(&manifest, "app", "https://cdn.example.com");
        // The chunk-a hash encoded in base64:
        let hash0_hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let expected_b64 = sri::hex_to_sri_b64(hash0_hex);
        assert!(html.contains(&format!("sha256-{expected_b64}")));
    }
}
