use wundler_pgo::clustering::{compute_clusters, C3Config};
use wundler_pgo::store::PgoStore;
use wundler_pgo::types::SessionRecord;

fn mk_record(session: &str, chunks: &[&str]) -> SessionRecord {
    SessionRecord {
        session_id: session.to_string(),
        entry_point: "home".to_string(),
        chunk_sequence: chunks.iter().map(|s| s.to_string()).collect(),
        timestamp_ms: 0,
    }
}

/// Build a store where `a` and `b` are always co-requested (200 sessions)
/// and `c` is independent (never requested with `a`).
fn co_requested_store() -> PgoStore {
    let store = PgoStore::open_in_memory().unwrap();
    for i in 0..200usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["a", "b"])).unwrap();
    }
    // 50 sessions with c only
    for i in 200..250usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["c"])).unwrap();
    }
    store
}

#[test]
fn below_min_sessions_returns_all_none() {
    // Only 99 sessions — below default min_sessions of 100
    let store = PgoStore::open_in_memory().unwrap();
    for i in 0..99usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["a", "b"])).unwrap();
    }
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into()];
    let config = C3Config::default();
    let result = compute_clusters(&store, &chunk_ids, &config).unwrap();
    assert!(result["a"].is_none());
    assert!(result["b"].is_none());
}

#[test]
fn always_co_requested_pair_gets_merge_suggestion() {
    let store = co_requested_store();
    // P(b|a) = 200/200 = 1.0, P(a|b) = 200/200 = 1.0 — both > 0.70
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let config = C3Config::default();
    let result = compute_clusters(&store, &chunk_ids, &config).unwrap();

    // bidirectional: a → b and b → a
    assert_eq!(result["a"], Some("b".to_string()), "a should suggest merge with b");
    assert_eq!(result["b"], Some("a".to_string()), "b should suggest merge with a");
}

#[test]
fn independent_chunk_gets_no_merge() {
    let store = co_requested_store();
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let config = C3Config::default();
    let result = compute_clusters(&store, &chunk_ids, &config).unwrap();

    // c is never loaded with a → P(c|a) = 0 < 0.70
    assert_eq!(result["c"], None, "c should have no merge suggestion");
}

#[test]
fn pair_below_threshold_is_not_merged() {
    // a and b co-requested in 60 out of 100 sessions → P = 0.60 < 0.70
    let store = PgoStore::open_in_memory().unwrap();
    for i in 0..60usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["a", "b"])).unwrap();
    }
    for i in 60..100usize {
        let id = format!("s{i}");
        store.insert_session(&mk_record(&id, &["a"])).unwrap();
    }
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into()];
    let config = C3Config::default();
    let result = compute_clusters(&store, &chunk_ids, &config).unwrap();

    assert_eq!(result["a"], None);
    assert_eq!(result["b"], None);
}

#[test]
fn results_keyed_for_all_input_chunk_ids() {
    let store = co_requested_store();
    let chunk_ids: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    let config = C3Config::default();
    let result = compute_clusters(&store, &chunk_ids, &config).unwrap();

    // All three chunks must have an entry in the map.
    assert!(result.contains_key("a"));
    assert!(result.contains_key("b"));
    assert!(result.contains_key("c"));
}
