//! CORS allowlist layer for the Asset Bundling Server.
//!
//! Builds a `tower_http::cors::CorsLayer` from a resolved security config.
//! Empty allowlist = no `Access-Control-Allow-Origin` header is ever set →
//! browsers block the response. There is no wildcard mode.

use std::time::Duration;

use axum::http::{header, HeaderValue, Method};
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::ResolvedSecurity;

/// Build the CORS layer for a given resolved security config.
///
/// * When `sec.allowed_origins` is empty, the returned layer **adds no
///   CORS headers**.
/// * When non-empty, the layer matches the request `Origin` against the
///   exact list and reflects the match into `Access-Control-Allow-Origin`.
///
/// Fixed policy: GET/POST, Authorization/Content-Type, 5-min preflight cache.
/// Credentials: never set.
pub fn build_cors(sec: &ResolvedSecurity) -> CorsLayer {
    if sec.allowed_origins.is_empty() {
        return CorsLayer::new();
    }

    let origins: Vec<HeaderValue> = sec.allowed_origins.clone();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(300))
    // NOTE: deliberately no `.allow_credentials(true)`
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecurityConfig;

    fn resolved_with(origins: &[&str]) -> ResolvedSecurity {
        let cfg = SecurityConfig {
            bearer_token_file: None,
            allowed_origins: origins.iter().map(|s| s.to_string()).collect(),
        };
        ResolvedSecurity::from_config(&cfg).expect("resolve")
    }

    #[test]
    fn empty_allowlist_returns_a_layer() {
        let sec = resolved_with(&[]);
        let _layer: CorsLayer = build_cors(&sec);
    }

    #[test]
    fn non_empty_allowlist_returns_a_layer() {
        let sec = resolved_with(&["https://app.example.com"]);
        let _layer: CorsLayer = build_cors(&sec);
    }

    #[test]
    fn multi_origin_allowlist_returns_a_layer() {
        let sec = resolved_with(&[
            "https://app.example.com",
            "https://staging.example.com",
        ]);
        let _layer: CorsLayer = build_cors(&sec);
    }
}
