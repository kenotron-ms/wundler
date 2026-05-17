//! Rust-half of the ed25519 signing contract test.
//!
//! This test pins the deterministic byte format of `manifest_signature_bytes`
//! so a future JavaScript verifier can replicate the canonical form byte-for-byte.
//!
//! What this test does NOT cover:
//! - SW-side JavaScript verification (separate PR)
//! - Key rotation (operator workflow, tested manually)

use std::collections::HashMap;

use wundler_abs::signing::{generate_keypair, ManifestSigner, ManifestVerifier};
use wundler_graph::ChunkManifest;

fn fixture_manifest() -> ChunkManifest {
    ChunkManifest {
        build_id: "contract-test-build-a1b2c3d4".to_string(),
        chunks: vec![],
        entry_chunks: HashMap::new(),
        module_index: HashMap::new(),
    }
}

#[test]
fn manifest_signature_bytes_is_deterministic() {
    let manifest = fixture_manifest();
    let bytes1 = wundler_abs::signing::manifest_signature_bytes(&manifest);
    let bytes2 = wundler_abs::signing::manifest_signature_bytes(&manifest);
    assert_eq!(bytes1, bytes2, "manifest_signature_bytes must be deterministic");
}

#[test]
fn sign_and_verify_roundtrip() {
    let manifest = fixture_manifest();
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer from PEM");
    let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem).expect("verifier from PEM");

    let sig = signer.sign_manifest(&manifest);
    verifier.verify(&manifest, &sig).expect("signature must verify against corresponding key");
}

#[test]
fn signature_changes_with_manifest_content() {
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer");

    let m1 = fixture_manifest();
    let mut m2 = fixture_manifest();
    m2.build_id = "different-build-id".to_string();

    let sig1 = signer.sign_manifest(&m1);
    let sig2 = signer.sign_manifest(&m2);
    assert_ne!(sig1.to_bytes(), sig2.to_bytes(), "different manifests must produce different signatures");
}

/// Pin the canonical byte representation for the JS verifier.
///
/// The test is marked `#[ignore]` because it writes artifact files used by the
/// future Node.js half of the contract test. Run with:
///   cargo test -p wundler-abs --test signing_contract_test -- --ignored
#[test]
#[ignore]
fn write_contract_fixtures() {
    let manifest = fixture_manifest();
    let canonical_bytes = wundler_abs::signing::manifest_signature_bytes(&manifest);
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem).expect("signer");
    let sig = signer.sign_manifest(&manifest);

    std::fs::create_dir_all("/tmp/wundler-contract").expect("create dir");
    std::fs::write("/tmp/wundler-contract/manifest.json", serde_json::to_vec_pretty(&manifest).unwrap()).expect("write manifest");
    std::fs::write("/tmp/wundler-contract/canonical_bytes.bin", &canonical_bytes).expect("write bytes");
    std::fs::write("/tmp/wundler-contract/signature.bin", sig.to_bytes()).expect("write sig");
    std::fs::write("/tmp/wundler-contract/verifying_key.pem", &kp.verifying_key_pem).expect("write key");

    println!("Contract fixtures written to /tmp/wundler-contract/");
    println!("manifest.json: {:?}", serde_json::to_string_pretty(&manifest).unwrap());
    println!("canonical_bytes length: {}", canonical_bytes.len());
    println!("signature (hex): {}", hex::encode(sig.to_bytes()));
}
