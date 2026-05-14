use std::collections::HashMap;
use wundler_pgo::types::{ChunkHint, PgoHints, SessionRecord};

#[test]
fn session_record_constructible() {
    let record = SessionRecord {
        session_id: "sess-abc".to_string(),
        entry_point: "/app".to_string(),
        chunk_sequence: vec![
            "chunk-a".to_string(),
            "chunk-b".to_string(),
            "chunk-c".to_string(),
        ],
        timestamp_ms: 1_700_000_000_000,
    };

    assert_eq!(record.chunk_sequence.len(), 3);
    assert_eq!(record.chunk_sequence[0], "chunk-a");
    assert_eq!(record.chunk_sequence[1], "chunk-b");
    assert_eq!(record.chunk_sequence[2], "chunk-c");
}

#[test]
fn pgo_hints_round_trips_json() {
    let mut chunk_hints = HashMap::new();
    chunk_hints.insert(
        "chunk-a".to_string(),
        ChunkHint {
            co_request_score: 0.83,
            median_load_order: 1.0,
            suggested_merge: Some("chunk-b".to_string()),
        },
    );
    chunk_hints.insert(
        "chunk-c".to_string(),
        ChunkHint {
            co_request_score: 0.21,
            median_load_order: 3.5,
            suggested_merge: None,
        },
    );

    let hints = PgoHints {
        build_id: "build-42".to_string(),
        chunk_hints,
    };

    let json = serde_json::to_string(&hints).expect("serialize");
    let roundtripped: PgoHints = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(roundtripped.build_id, "build-42");

    let hint_a = roundtripped.chunk_hints.get("chunk-a").expect("chunk-a present");
    assert!((hint_a.co_request_score - 0.83).abs() < 1e-9);
    assert_eq!(hint_a.suggested_merge, Some("chunk-b".to_string()));

    let hint_c = roundtripped.chunk_hints.get("chunk-c").expect("chunk-c present");
    assert_eq!(hint_c.suggested_merge, None);
}

#[test]
fn chunk_hint_serializes_none_merge_as_null() {
    let hint = ChunkHint {
        co_request_score: 0.5,
        median_load_order: 2.0,
        suggested_merge: None,
    };

    let json = serde_json::to_string(&hint).expect("serialize");
    assert!(
        json.contains("\"suggested_merge\":null"),
        "expected null in JSON, got: {json}"
    );
}
