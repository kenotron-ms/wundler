use cloudpack_pgo::clustering::C3Config;
use cloudpack_pgo::hints::compute_hints;
use cloudpack_pgo::store::PgoStore;
use cloudpack_pgo::types::SessionRecord;

fn mk_record(session: &str, chunks: &[&str]) -> SessionRecord {
    SessionRecord {
        session_id: session.to_string(),
        entry_point: "home".to_string(),
        chunk_sequence: chunks.iter().map(|s| s.to_string()).collect(),
        timestamp_ms: 0,
    }
}

/// 100 sessions: chunks [a, b, c] in that order.
fn hundred_session_store() -> PgoStore {
    let store = PgoStore::open_in_memory().unwrap();
    for i in 0..100usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["a", "b", "c"])).unwrap();
    }
    store
}

#[test]
fn hints_struct_has_entry_for_all_chunks() {
    let store = hundred_session_store();
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let hints = compute_hints(&store, "build-001", &chunk_ids, &C3Config::default()).unwrap();

    assert_eq!(hints.build_id, "build-001");
    assert!(hints.chunk_hints.contains_key("a"));
    assert!(hints.chunk_hints.contains_key("b"));
    assert!(hints.chunk_hints.contains_key("c"));
}

#[test]
fn co_request_score_for_first_position_chunk() {
    let store = hundred_session_store();
    // 'a' is always at position 0, within_n=3 → all 100 sessions → score = 1.0
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let hints = compute_hints(&store, "build-001", &chunk_ids, &C3Config::default()).unwrap();

    let score_a = hints.chunk_hints["a"].co_request_score;
    assert!(
        (score_a - 1.0).abs() < 1e-9,
        "score for 'a' (always at pos 0) should be 1.0, got {score_a}"
    );
}

#[test]
fn median_load_order_reflects_position() {
    let store = hundred_session_store();
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let hints = compute_hints(&store, "build-001", &chunk_ids, &C3Config::default()).unwrap();

    assert_eq!(hints.chunk_hints["a"].median_load_order, 0.0);
    assert_eq!(hints.chunk_hints["b"].median_load_order, 1.0);
    assert_eq!(hints.chunk_hints["c"].median_load_order, 2.0);
}

#[test]
fn empty_store_returns_zeroed_hints() {
    let store = PgoStore::open_in_memory().unwrap();
    let chunk_ids: Vec<String> = vec!["a".into()];
    let hints = compute_hints(&store, "bid", &chunk_ids, &C3Config::default()).unwrap();

    let h = &hints.chunk_hints["a"];
    assert_eq!(h.co_request_score, 0.0);
    assert_eq!(h.median_load_order, 0.0);
    assert_eq!(h.suggested_merge, None);
}
