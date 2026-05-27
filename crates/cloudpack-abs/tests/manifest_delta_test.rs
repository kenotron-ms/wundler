use cloudpack_abs::manifest::compute_delta;
use cloudpack_abs::types::{ManifestRequest, ManifestResponse};
use cloudpack_core::types::ContentHash;
use cloudpack_graph::ChunkManifest;

/// Load the simple fixture manifest for all tests.
fn load_fixture() -> ChunkManifest {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifest_simple.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture at {}: {}", path.display(), e));
    serde_json::from_str(&content).expect("failed to parse fixture JSON")
}

/// Load the PGO fixture manifest for prefetch tests.
fn load_pgo_fixture() -> ChunkManifest {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manifest_with_pgo.json");
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

/// Helper: build a ManifestRequest with an explicit build_id (Some or None).
fn req_with_bid(entry: &str, hashes: Vec<&str>, build_id: Option<&str>) -> ManifestRequest {
    ManifestRequest {
        entry_point: entry.to_string(),
        cached_hashes: hashes.into_iter().map(|s| ContentHash(s.to_string())).collect(),
        build_id: build_id.map(|s| s.to_string()),
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
// Test 2: client has both shell modules + matching build_id → shell excluded, 2 remain
// ---------------------------------------------------------------------------

#[test]
fn fully_cached_chunk_is_excluded() {
    let manifest = load_fixture();
    // Both shell modules are cached AND build_id matches → shell chunk should be excluded
    let request = req_with_bid("teams.channel", vec!["shell_mod_a", "shell_mod_b"], Some("b8f3a1c2"));
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

// ---------------------------------------------------------------------------
// Test 7: stale build_id ignores client cache
// ---------------------------------------------------------------------------

#[test]
fn stale_build_id_ignores_client_cache() {
    let manifest = load_fixture();
    // Client claims all shell modules cached but provides a stale build_id
    let request = req_with_bid(
        "teams.channel",
        vec!["shell_mod_a", "shell_mod_b"],
        Some("old_build_id"),
    );
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        3,
        "stale build_id should return all 3 chunks despite client claiming shell is cached, got: {:?}",
        response.fetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 8: missing build_id ignores client cache
// ---------------------------------------------------------------------------

#[test]
fn missing_build_id_ignores_client_cache() {
    let manifest = load_fixture();
    // No build_id — client claims shell modules cached
    let request = req("teams.channel", vec!["shell_mod_a", "shell_mod_b"]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        3,
        "missing build_id should return all 3 chunks despite client claiming shell is cached, got: {:?}",
        response.fetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 9: matching build_id honors client cache
// ---------------------------------------------------------------------------

#[test]
fn matching_build_id_honors_client_cache() {
    let manifest = load_fixture();
    // Correct build_id + shell fully cached → only 2 chunks needed
    let request = req_with_bid(
        "teams.channel",
        vec!["shell_mod_a", "shell_mod_b"],
        Some("b8f3a1c2"),
    );
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    assert_eq!(
        response.fetch_urls.len(),
        2,
        "matching build_id should honor client cache (2 chunks remain), got: {:?}",
        response.fetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 10: high co_request_score chunk appears in prefetch_urls
// ---------------------------------------------------------------------------

#[test]
fn high_co_request_score_chunks_appear_in_prefetch() {
    let manifest = load_pgo_fixture();
    // No build_id → cache not trusted, but settings-pane score=0.85 ≥ 0.7
    let request = req("home", vec![]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    let settings_pane_url = "https://cdn.example.com/chunks/bbbb2222.js";
    assert!(
        response.prefetch_urls.contains(&settings_pane_url.to_string()),
        "settings-pane (score=0.85) should appear in prefetch_urls, got: {:?}",
        response.prefetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 11: low co_request_score chunk is NOT in prefetch_urls
// ---------------------------------------------------------------------------

#[test]
fn low_co_request_score_chunks_are_not_prefetched() {
    let manifest = load_pgo_fixture();
    let request = req("home", vec![]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    let rarely_visited_url = "https://cdn.example.com/chunks/bbbb3333.js";
    assert!(
        !response.prefetch_urls.contains(&rarely_visited_url.to_string()),
        "rarely-visited (score=0.15) should NOT appear in prefetch_urls, got: {:?}",
        response.prefetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 12: chunks already in fetch_urls are excluded from prefetch_urls
// ---------------------------------------------------------------------------

#[test]
fn prefetch_excludes_chunks_already_in_fetch() {
    let mut manifest = load_pgo_fixture();
    // Set home-shell co_request_score=0.95 so it would qualify for prefetch
    for chunk in &mut manifest.chunks {
        if chunk.id == "home-shell" {
            chunk.co_request_score = Some(0.95);
        }
    }
    // home-shell is in entry_chunks["home"] → will appear in fetch_urls
    let request = req("home", vec![]);
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    let home_shell_url = "https://cdn.example.com/chunks/aaaa1111.js";
    assert!(
        response.fetch_urls.contains(&home_shell_url.to_string()),
        "home-shell should be in fetch_urls, got: {:?}",
        response.fetch_urls
    );
    assert!(
        !response.prefetch_urls.contains(&home_shell_url.to_string()),
        "home-shell should NOT appear in prefetch_urls (already in fetch), got: {:?}",
        response.prefetch_urls
    );
}

// ---------------------------------------------------------------------------
// Test 13: client already has chunk's modules → excluded from prefetch
// ---------------------------------------------------------------------------

#[test]
fn prefetch_excludes_chunks_client_already_has() {
    let manifest = load_pgo_fixture();
    // Client has settings_mod_a cached AND matching build_id → settings-pane fully cached
    let request = req_with_bid("home", vec!["settings_mod_a"], Some("p1g2o3"));
    let response = compute_delta(&manifest, &request, "https://cdn.example.com");

    let settings_pane_url = "https://cdn.example.com/chunks/bbbb2222.js";
    assert!(
        !response.prefetch_urls.contains(&settings_pane_url.to_string()),
        "settings-pane should NOT appear in prefetch_urls (client already has it), got: {:?}",
        response.prefetch_urls
    );
}
