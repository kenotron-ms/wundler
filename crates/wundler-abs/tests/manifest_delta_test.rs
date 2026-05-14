use wundler_abs::manifest::compute_delta;
use wundler_abs::types::{ManifestRequest, ManifestResponse};
use wundler_core::types::ContentHash;
use wundler_graph::ChunkManifest;

/// Load the simple fixture manifest for all tests.
fn load_fixture() -> ChunkManifest {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifest_simple.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture at {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("failed to parse fixture JSON")
}

/// Helper: build a ManifestRequest for the given entry with some cached hashes.
fn req(entry: &str, hashes: Vec<&str>) -> ManifestRequest {
    ManifestRequest {
        entry_point: entry.to_string(),
        cached_hashes: hashes.into_iter().map(|s| ContentHash(s.to_string())).collect(),
        build_id: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: empty cache → all 3 chunks returned
// ---------------------------------------------------------------------------

#[test]
fn empty_cache_returns_all_chunks_for_entry() {
    let manifest = load_fixture();
    let request = req("teams.channel", vec![]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        3,
        "expected all 3 chunks when cache is empty, got: {:?}",
        response.fetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 2: client has both shell modules → shell excluded, 2 remain
// ---------------------------------------------------------------------------

#[test]
fn fully_cached_chunk_is_excluded() {
    let manifest = load_fixture();
    // Both shell modules are cached → shell chunk should be excluded
    let request = req("teams.channel", vec!["shell_mod_a", "shell_mod_b"]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        2,
        "expected 2 chunks (vendor + channel) when shell is fully cached, got: {:?}",
        response.fetch_urls
    );

    // Verify none of the returned URLs correspond to the shell chunk
    let shell_url_fragment = "aabbccdd";
    for url in &response.fetch_urls {
        assert!(
            !url.contains(shell_url_fragment),
            "shell chunk URL should not appear in fetch_urls, but got: {url}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: partial cache hit on channel → chunk still served
// ---------------------------------------------------------------------------

#[test]
fn partially_cached_chunk_is_still_served() {
    let manifest = load_fixture();
    // Only channel_mod_a is cached (not channel_mod_b) → channel chunk is still needed
    let request = req("teams.channel", vec!["channel_mod_a"]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        3,
        "expected all 3 chunks (partial cache hit doesn't exclude chunk), got: {:?}",
        response.fetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 4: unknown entry point → empty fetch_urls
// ---------------------------------------------------------------------------

#[test]
fn unknown_entry_point_returns_empty_fetch_urls() {
    let manifest = load_fixture();
    let request = req("teams.nonexistent", vec![]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert!(
        response.fetch_urls.is_empty(),
        "expected empty fetch_urls for unknown entry point, got: {:?}",
        response.fetch_urls
    );
    assert_eq!(
        response.build_id, manifest.build_id,
        "build_id should still match manifest's build_id"
    );
}

// ---------------------------------------------------------------------------
// Test 5: fetch URLs use the configured CDN base
// ---------------------------------------------------------------------------

#[test]
fn fetch_urls_use_configured_cdn_base() {
    let manifest = load_fixture();
    let request = req("teams.channel", vec![]);
    let cdn = "https://my-cdn.example.com";
    let response = compute_delta(&manifest, &request, cdn);

    assert_eq!(response.fetch_urls.len(), 3);
    for url in &response.fetch_urls {
        assert!(
            url.starts_with(&format!("{cdn}/chunks/")),
            "URL should start with '{cdn}/chunks/', got: {url}"
        );
        assert!(url.ends_with(".js"), "URL should end with .js, got: {url}");
    }
}

// ---------------------------------------------------------------------------
// Test 6: ttl is 0 (handler sets real ttl later)
// ---------------------------------------------------------------------------

#[test]
fn ttl_is_carried_through() {
    let manifest = load_fixture();
    let request = req("teams.channel", vec![]);
    let response: ManifestResponse =
        compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(response.ttl, 0, "compute_delta should return ttl=0");
    assert_eq!(
        response.build_id, "b8f3a1c2",
        "build_id should be carried from manifest"
    );
    assert!(
        response.prefetch_urls.is_empty(),
        "prefetch_urls should be empty (Task 5 adds it)"
    );
}
