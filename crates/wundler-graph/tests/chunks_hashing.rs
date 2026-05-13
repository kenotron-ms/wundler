//! Tests for `hash_chunk` — SHA-256 over sorted member hashes.
//!
//! Covers: deterministic, order-independent, changes-on-mutation, empty-chunk
//! known value, and 64-char hex output format.

use wundler_core::types::ContentHash;
use wundler_graph::chunks::hash_chunk;

// ---------------------------------------------------------------------------
// Test 1: Empty chunk produces the SHA-256 digest of empty input
// ---------------------------------------------------------------------------

/// SHA-256 of an empty byte sequence is the known constant
/// `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
#[test]
fn empty_chunk_equals_sha256_of_empty_input() {
    let result = hash_chunk(&[]);
    assert_eq!(
        result.0, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "empty chunk must produce SHA-256 of empty input"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Output is 64-char lowercase ASCII hex
// ---------------------------------------------------------------------------

#[test]
fn output_is_64_char_lowercase_hex() {
    let members = vec![ContentHash::from_source("some-module")];
    let result = hash_chunk(&members);
    assert_eq!(
        result.0.len(),
        64,
        "hash must be exactly 64 characters, got: {}",
        result.0
    );
    assert!(
        result.0.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "hash must contain only lowercase hex digits, got: {}",
        result.0
    );
}

// ---------------------------------------------------------------------------
// Test 3: Deterministic — same input yields same output
// ---------------------------------------------------------------------------

#[test]
fn deterministic_same_input_same_output() {
    let members = vec![
        ContentHash::from_source("module_a"),
        ContentHash::from_source("module_b"),
    ];
    let first = hash_chunk(&members);
    let second = hash_chunk(&members);
    assert_eq!(first, second, "hash_chunk must be deterministic");
}

// ---------------------------------------------------------------------------
// Test 4: Order-independent — shuffling members yields the same hash
// ---------------------------------------------------------------------------

#[test]
fn order_independent() {
    let a = ContentHash::from_source("module_a");
    let b = ContentHash::from_source("module_b");
    let c = ContentHash::from_source("module_c");

    let forward = hash_chunk(&[a.clone(), b.clone(), c.clone()]);
    let reversed = hash_chunk(&[c.clone(), b.clone(), a.clone()]);
    let shuffled = hash_chunk(&[b.clone(), a.clone(), c.clone()]);

    assert_eq!(
        forward, reversed,
        "hash must be order-independent (forward vs reversed)"
    );
    assert_eq!(
        forward, shuffled,
        "hash must be order-independent (forward vs shuffled)"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Hash changes when members change
// ---------------------------------------------------------------------------

#[test]
fn changes_when_members_change() {
    let a = ContentHash::from_source("module_a");
    let b = ContentHash::from_source("module_b");

    let hash_ab = hash_chunk(&[a.clone(), b.clone()]);
    let hash_a_only = hash_chunk(std::slice::from_ref(&a));
    let hash_b_only = hash_chunk(std::slice::from_ref(&b));

    assert_ne!(hash_ab, hash_a_only, "adding a member must change the hash");
    assert_ne!(
        hash_ab, hash_b_only,
        "different member sets must produce different hashes"
    );
    assert_ne!(
        hash_a_only, hash_b_only,
        "different single members must produce different hashes"
    );
}
