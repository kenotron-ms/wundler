use wundler_abs::types::{ManifestRequest, ManifestResponse, TelemetryEvent};
use wundler_core::types::ContentHash;

#[test]
fn manifest_request_round_trips_json() {
    let req = ManifestRequest {
        entry_point: "src/index.ts".to_string(),
        cached_hashes: vec![ContentHash("abc123".to_string())],
        build_id: Some("build-42".to_string()),
    };
    let json = serde_json::to_string(&req).expect("serialize ManifestRequest");
    let decoded: ManifestRequest =
        serde_json::from_str(&json).expect("deserialize ManifestRequest");
    assert_eq!(decoded, req);
}

#[test]
fn manifest_request_accepts_missing_build_id() {
    let json = r#"{"entry_point":"src/main.ts","cached_hashes":[]}"#;
    let req: ManifestRequest = serde_json::from_str(json).expect("deserialize without build_id");
    assert_eq!(req.build_id, None);
    assert_eq!(req.entry_point, "src/main.ts");
}

#[test]
fn manifest_response_round_trips_json() {
    let resp = ManifestResponse {
        build_id: "build-7".to_string(),
        fetch_urls: vec!["https://cdn.example.com/chunk-a.js".to_string()],
        prefetch_urls: vec!["https://cdn.example.com/chunk-b.js".to_string()],
        ttl: 3600,
    };
    let json = serde_json::to_string(&resp).expect("serialize ManifestResponse");
    let decoded: ManifestResponse =
        serde_json::from_str(&json).expect("deserialize ManifestResponse");
    assert_eq!(decoded, resp);
}

#[test]
fn telemetry_event_round_trips_json() {
    let event = TelemetryEvent {
        session_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        entry_point: "src/index.ts".to_string(),
        chunks_served: vec!["chunk-a".to_string(), "chunk-b".to_string()],
        client_had: vec![ContentHash("deadbeef".to_string())],
        timestamp_ms: 1_700_000_000_000,
    };
    let json = serde_json::to_string(&event).expect("serialize TelemetryEvent");
    let decoded: TelemetryEvent =
        serde_json::from_str(&json).expect("deserialize TelemetryEvent");
    assert_eq!(decoded, event);
}
