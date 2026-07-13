fn main() {
    println!("cargo:rerun-if-env-changed=RECEIPTS_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=RECEIPTS_LOCK_DIGEST");
    let build_commit =
        std::env::var("RECEIPTS_BUILD_COMMIT").unwrap_or_else(|_| "unresolved".to_string());
    let lock_digest =
        std::env::var("RECEIPTS_LOCK_DIGEST").unwrap_or_else(|_| "unresolved".to_string());
    println!("cargo:rustc-env=RECEIPTS_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=RECEIPTS_LOCK_DIGEST={lock_digest}");
}
