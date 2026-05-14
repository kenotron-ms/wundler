#[test]
fn crate_is_reachable() {
    // Compilation of this test confirms the crate is wired into the workspace.
    assert_eq!(wundler_pgo::CRATE_NAME, "wundler-pgo");
}
