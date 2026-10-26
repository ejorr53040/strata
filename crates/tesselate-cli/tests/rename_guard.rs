//! Guard for the rename to tesselate: the old project name must not appear in the tree,
//! neither in a file's path nor in its text.

use std::path::{Path, PathBuf};

const OLD_NAME: &str = "strata";

/// Files that legitimately keep the old name, relative to the repo root.
const ALLOWED: &[&str] = &[
    // Records the rename itself.
    "docs/adr/0003-rename-to-tesselate.md",
    // Spells the old name out in order to look for it.
    "crates/tesselate-cli/tests/rename_guard.rs",
    // Spells the old name out to prove it no longer does anything.
    "crates/tesselate-cli/tests/rename.rs",
    // Append-only measurement series: rows are never rewritten.
    "docs/metrics/history.jsonl",
    "docs/metrics/history-cirrocumulus.jsonl",
    "docs/metrics/chart.svg",
    "docs/metrics/chart-cirrocumulus.svg",
];

/// Directories that are not project content: VCS data, build output, agent worktrees.
const SKIPPED_DIRS: &[&str] = &[".git", "target", ".claude", ".scratch"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<crate> sits two levels below the repo root")
        .to_path_buf()
}

fn collect(dir: &Path, root: &Path, offenders: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Also covers the `.git` pointer file a worktree has.
        if SKIPPED_DIRS.contains(&name.as_str()) {
            continue;
        }
        if ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        if name.to_lowercase().contains(OLD_NAME) {
            offenders.push(format!("{rel} (name)"));
        }
        if path.is_dir() {
            collect(&path, root, offenders);
            continue;
        }
        // Binary files are not project text.
        if let Ok(text) = std::fs::read_to_string(&path) {
            let hits = text.to_lowercase().matches(OLD_NAME).count();
            if hits > 0 {
                offenders.push(format!("{rel} ({hits})"));
            }
        }
    }
}

#[test]
fn the_old_project_name_appears_nowhere_in_the_tree() {
    let root = repo_root();
    let mut offenders = Vec::new();
    collect(&root, &root, &mut offenders);
    offenders.sort();
    assert!(
        offenders.is_empty(),
        "`{OLD_NAME}` still appears in {} places:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
