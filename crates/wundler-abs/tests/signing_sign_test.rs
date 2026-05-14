//! Integration tests that pin the deterministic-bytes contract for
//! [`wundler_abs::signing::manifest_signature_bytes`].
//!
//! These tests verify that:
//!
//! 1. Signing bytes are independent of the order of the `chunks` Vec.
//! 2. Changing `build_id` changes the signing bytes.
//! 3. Changing a chunk's content hash changes the signing bytes.
//! 4. Changing PGO advisory fields (`co_request_score`) does **not** change
//!    the signing bytes — those fields can be updated by the optimisation
//!    pipeline without invalidating existing signatures.
//! 5. `ManifestSigner::sign_manifest` produces a 64-byte ed25519 signature.

use std::collections::HashMap;

use wundler_abs::signing::{generate_keypair, manifest_signature_bytes, ManifestSigner};
use wundler_core::types::ContentHash;
use wundler_graph::{Chunk, ChunkManifest, LoadCondition};

// ---------------------------------------------------------------------------
// Test helper
// ---------------------------------------------------------------------------

/// Build a minimal [`ChunkManifest`] for the given `build_id`.
///
/// Layout
/// ------
/// * `entry_chunks = { "home": ["a", "b"] }`
/// * Chunk `"a"`: one module with hash `1111`, chunk hash `aaaa1111`.
/// * Chunk `"b"`: one module with hash `2222`, chunk hash `aaaa2222`.
/// * `module_index = { "1111" => "a", "2222" => "b" }`
/// * All PGO advisory fields (`co_request_score`, `median_load_order`,
///   `suggested_merge`) set to `None`.
fn make_manifest(build_id: &str) -> ChunkManifest {
    let mod_a = ContentHash("1111".to_string());
    let mod_b = ContentHash("2222".to_string());
    let hash_a = ContentHash("aaaa1111".to_string());
    let hash_b = ContentHash("aaaa2222".to_string());

    let chunk_a = Chunk {
        id: "a".to_string(),
        modules: vec![mod_a.clone()],
        hash: hash_a,
        load_condition: LoadCondition::Lazy,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };

    let chunk_b = Chunk {
        id: "b".to_string(),
        modules: vec![mod_b.clone()],
        hash: hash_b,
        load_condition: LoadCondition::Lazy,
        co_request_score: None,
        median_load_order: None,
        suggested_merge: None,
    };

    let mut entry_chunks = HashMap::new();
    entry_chunks.insert(
        "home".to_string(),
        vec!["a".to_string(), "b".to_string()],
    );

    let mut module_index = HashMap::new();
    module_index.insert(mod_a, "a".to_string());
    module_index.insert(mod_b, "b".to_string());

    ChunkManifest {
        build_id: build_id.to_string(),
        chunks: vec![chunk_a, chunk_b],
        entry_chunks,
        module_index,
    }
}

// ---------------------------------------------------------------------------
// Test 1: deterministic regardless of chunk Vec order
// ---------------------------------------------------------------------------

/// Signing bytes must be identical no matter how the `chunks` Vec is ordered.
///
/// The implementation sorts chunks by ID before serialising, so reversing the
/// Vec must produce the same canonical bytes.
#[test]
fn signing_bytes_are_deterministic_regardless_of_chunk_order() {
    let m1 = make_manifest("b1");
    let mut m2 = make_manifest("b1");
    m2.chunks.reverse(); // chunks are now [b, a] instead of [a, b]

    assert_eq!(
        manifest_signature_bytes(&m1),
        manifest_signature_bytes(&m2),
        "signing bytes must be identical regardless of chunk Vec order"
    );
}

// ---------------------------------------------------------------------------
// Test 2: build_id change propagates to signing bytes
// ---------------------------------------------------------------------------

/// Changing `build_id` must produce different signing bytes.
#[test]
fn signing_bytes_change_when_build_id_changes() {
    let m1 = make_manifest("b1");
    let m2 = make_manifest("b2");

    assert_ne!(
        manifest_signature_bytes(&m1),
        manifest_signature_bytes(&m2),
        "signing bytes must differ when build_id differs"
    );
}

// ---------------------------------------------------------------------------
// Test 3: chunk hash change propagates to signing bytes
// ---------------------------------------------------------------------------

/// Changing a chunk's content hash must produce different signing bytes.
#[test]
fn signing_bytes_change_when_chunk_hash_changes() {
    let m1 = make_manifest("b1");
    let mut m2 = make_manifest("b1");
    // Replace chunk "a"'s hash with a different value.
    m2.chunks[0].hash = ContentHash("bbbb1111".to_string());

    assert_ne!(
        manifest_signature_bytes(&m1),
        manifest_signature_bytes(&m2),
        "signing bytes must differ when a chunk's content hash changes"
    );
}

// ---------------------------------------------------------------------------
// Test 4: PGO advisory fields are excluded from signing bytes
// ---------------------------------------------------------------------------

/// `co_request_score` is a PGO advisory field.  It may be updated by the
/// optimisation pipeline at any time without changing chunk integrity.
/// Therefore it must **not** influence the canonical signing bytes.
#[test]
fn signing_bytes_ignore_co_request_score_changes() {
    let m1 = make_manifest("b1");
    let mut m2 = make_manifest("b1");
    m2.chunks[0].co_request_score = Some(0.95);

    assert_eq!(
        manifest_signature_bytes(&m1),
        manifest_signature_bytes(&m2),
        "signing bytes must be identical when only co_request_score changes \
         (PGO advice must not invalidate signatures)"
    );
}

// ---------------------------------------------------------------------------
// Test 5: ManifestSigner produces a 64-byte ed25519 signature
// ---------------------------------------------------------------------------

/// A freshly generated key pair must produce a valid 64-byte ed25519 signature
/// over the canonical manifest bytes.
#[test]
fn signer_produces_valid_signature() {
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem)
        .expect("ManifestSigner::from_pem must succeed for a freshly generated key");

    let manifest = make_manifest("b1");
    let sig = signer.sign_manifest(&manifest);

    assert_eq!(
        sig.to_bytes().len(),
        64,
        "ed25519 signature must be exactly 64 bytes"
    );
}
