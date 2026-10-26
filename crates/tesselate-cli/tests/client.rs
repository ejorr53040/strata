use std::path::PathBuf;

fn contains(bin: &PathBuf, needle: &[u8]) -> bool {
    std::fs::read(bin)
        .unwrap()
        .windows(needle.len())
        .any(|w| w == needle)
}

#[test]
fn the_client_binary_links_no_tree_sitter_grammar() {
    let client = PathBuf::from(env!("CARGO_BIN_EXE_tesselate"));
    let daemon = client.with_file_name("tesselate-daemon");
    // The daemon carries the grammars, which shows the needle is what a grammar leaves behind.
    assert!(contains(&daemon, b"tree_sitter_rust"));
    assert!(!contains(&client, b"tree_sitter"));
}
