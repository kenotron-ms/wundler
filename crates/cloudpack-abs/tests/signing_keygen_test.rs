use cloudpack_abs::signing::{generate_keypair, ManifestSigner, ManifestVerifier};

#[test]
fn generates_distinct_keys() {
    let kp1 = generate_keypair();
    let kp2 = generate_keypair();
    assert_ne!(
        kp1.signing_key_pem, kp2.signing_key_pem,
        "two generate_keypair() calls must produce different signing keys"
    );
    assert_ne!(
        kp1.verifying_key_pem, kp2.verifying_key_pem,
        "two generate_keypair() calls must produce different verifying keys"
    );
}

#[test]
fn signing_key_pem_format() {
    let kp = generate_keypair();
    assert!(
        kp.signing_key_pem.contains("BEGIN PRIVATE KEY"),
        "signing key PEM must contain 'BEGIN PRIVATE KEY', got:\n{}",
        kp.signing_key_pem
    );
    assert!(
        kp.signing_key_pem.contains("END PRIVATE KEY"),
        "signing key PEM must contain 'END PRIVATE KEY', got:\n{}",
        kp.signing_key_pem
    );
}

#[test]
fn verifying_key_pem_format() {
    let kp = generate_keypair();
    assert!(
        kp.verifying_key_pem.contains("BEGIN PUBLIC KEY"),
        "verifying key PEM must contain 'BEGIN PUBLIC KEY', got:\n{}",
        kp.verifying_key_pem
    );
    assert!(
        kp.verifying_key_pem.contains("END PUBLIC KEY"),
        "verifying key PEM must contain 'END PUBLIC KEY', got:\n{}",
        kp.verifying_key_pem
    );
}

#[test]
fn keypair_pem_roundtrip_via_parsers() {
    let kp = generate_keypair();
    let signer = ManifestSigner::from_pem(&kp.signing_key_pem)
        .expect("ManifestSigner::from_pem must succeed for a freshly generated signing key");
    let verifier = ManifestVerifier::from_pem(&kp.verifying_key_pem)
        .expect("ManifestVerifier::from_pem must succeed for a freshly generated verifying key");
    // Suppress unused variable warnings — the test just verifies construction succeeds.
    let _ = signer;
    let _ = verifier;
}
