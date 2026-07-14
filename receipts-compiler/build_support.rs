use std::path::Path;

pub fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn cargo_vcs_head(path: &Path) -> Option<String> {
    cargo_vcs_head_text(&std::fs::read_to_string(path).ok()?)
}

pub fn cargo_vcs_head_text(text: &str) -> Option<String> {
    // Cargo injects this file into registry archives. Read only the exact
    // SHA field so a crates.io build retains the source commit even though the
    // extracted package intentionally has no .git directory.
    let after_key = text.split_once("\"sha1\"")?.1;
    let after_colon = after_key.split_once(':')?.1.trim_start();
    let value = after_colon.strip_prefix('"')?.split_once('"')?.0;
    is_hex(value, 40).then(|| value.to_ascii_lowercase())
}
