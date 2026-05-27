use cloudpack_pgo::store::PgoStore;
use cloudpack_pgo::types::SessionRecord;

fn rec(session: &str, entry: &str, chunks: &[&str]) -> SessionRecord {
    SessionRecord {
        session_id: session.to_string(),
        entry_point: entry.to_string(),
        chunk_sequence: chunks.iter().map(|s| s.to_string()).collect(),
        timestamp_ms: 1_000_000,
    }
}

fn five_session_store() -> PgoStore {
    // s1: [a, b, c]   positions 0,1,2
    // s2: [a, b, d]   positions 0,1,2
    // s3: [a, c, d]   positions 0,1,2
    // s4: [b, c]      positions 0,1
    // s5: [d]         position  0
    let store = PgoStore::open_in_memory().unwrap();
    store.insert_session(&rec("s1", "home", &["a", "b", "c"])).unwrap();
    store.insert_session(&rec("s2", "home", &["a", "b", "d"])).unwrap();
    store.insert_session(&rec("s3", "home", &["a", "c", "d"])).unwrap();
    store.insert_session(&rec("s4", "home", &["b", "c"])).unwrap();
    store.insert_session(&rec("s5", "home", &["d"])).unwrap();
    store
}

// --- co_load_count ---

#[test]
fn co_load_count_both_present() {
    let store = five_session_store();
    // a and b co-appear in s1, s2 → 2
    assert_eq!(store.co_load_count("a", "b").unwrap(), 2);
}

#[test]
fn co_load_count_symmetric() {
    let store = five_session_store();
    assert_eq!(
        store.co_load_count("a", "b").unwrap(),
        store.co_load_count("b", "a").unwrap(),
        "co_load_count must be symmetric"
    );
}

#[test]
fn co_load_count_one_never_loaded() {
    let store = five_session_store();
    assert_eq!(store.co_load_count("a", "z").unwrap(), 0);
}

#[test]
fn co_load_count_c_and_d() {
    let store = five_session_store();
    // c and d co-appear in s3 → 1
    assert_eq!(store.co_load_count("c", "d").unwrap(), 1);
}

// --- median_load_order ---

#[test]
fn median_load_order_chunk_a() {
    let store = five_session_store();
    // chunk 'a' load_orders: 0,0,0 → median = 0.0
    assert_eq!(store.median_load_order("a").unwrap(), 0.0);
}

#[test]
fn median_load_order_chunk_b() {
    let store = five_session_store();
    // chunk 'b' load_orders: 1,1,0 → sorted: 0,1,1 → median = 1.0
    assert_eq!(store.median_load_order("b").unwrap(), 1.0);
}

#[test]
fn median_load_order_unknown_chunk() {
    let store = five_session_store();
    // no data → 0.0
    assert_eq!(store.median_load_order("z").unwrap(), 0.0);
}

// --- initial_load_count ---

#[test]
fn initial_load_count_within_1() {
    let store = five_session_store();
    // load_order < 1 means position 0 only
    // 'a' is at position 0 in s1,s2,s3 → 3
    assert_eq!(store.initial_load_count("a", 1).unwrap(), 3);
    // 'b' is at position 0 in s4, position 1 in s1,s2 → within_1 = only s4 → 1
    assert_eq!(store.initial_load_count("b", 1).unwrap(), 1);
}

#[test]
fn initial_load_count_within_3() {
    let store = five_session_store();
    // 'b' positions: s1→1, s2→1, s4→0 — all < 3 → 3
    assert_eq!(store.initial_load_count("b", 3).unwrap(), 3);
    // 'd' positions: s2→2, s3→2, s5→0 — all < 3 → 3
    assert_eq!(store.initial_load_count("d", 3).unwrap(), 3);
}

#[test]
fn initial_load_count_zero_for_unknown() {
    let store = five_session_store();
    assert_eq!(store.initial_load_count("z", 3).unwrap(), 0);
}

// --- all_chunk_ids ---

#[test]
fn all_chunk_ids_returns_sorted_distinct() {
    let store = five_session_store();
    let ids = store.all_chunk_ids().unwrap();
    assert_eq!(ids, vec!["a", "b", "c", "d"]);
}

#[test]
fn all_chunk_ids_empty_store() {
    let store = PgoStore::open_in_memory().unwrap();
    assert_eq!(store.all_chunk_ids().unwrap(), Vec::<String>::new());
}
