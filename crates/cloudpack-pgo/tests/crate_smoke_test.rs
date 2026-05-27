#[test]
fn crate_is_reachable() {
    // Compilation of this test confirms the crate is wired into the workspace.
    assert_eq!(cloudpack_pgo::CRATE_NAME, "cloudpack-pgo");
}
