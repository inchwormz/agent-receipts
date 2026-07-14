#[allow(dead_code)]
#[path = "../build_support.rs"]
mod build_support;

#[test]
fn cargo_registry_archive_retains_exact_git_identity() {
    let expected = "21d1d8cf12f152d6f10c2efe160bead4371216f6";
    let vcs = format!(r#"{{"git":{{"sha1":"{expected}"}},"path_in_vcs":"receipts-compiler"}}"#);
    assert_eq!(
        build_support::cargo_vcs_head_text(&vcs).as_deref(),
        Some(expected)
    );
    assert_eq!(
        build_support::cargo_vcs_head_text(r#"{"git":{"sha1":"unresolved"}}"#),
        None
    );
}
