mod common;

use std::path::{Path, PathBuf};
use std::process::Output;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A private runtime dir, so each test sees only its own daemons.
struct Runtime(tempfile::TempDir);

impl Runtime {
    fn new() -> Runtime {
        Runtime(tempfile::tempdir().unwrap())
    }

    fn run(&self, repo: &Path, args: &[&str]) -> Output {
        common::strata_in_runtime(self.0.path())
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap()
    }

    fn ok(&self, repo: &Path, args: &[&str]) -> String {
        let out = self.run(repo, args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// The pid `strata daemon status` reports, or None when no daemon runs for `repo`.
    fn pid(&self, repo: &Path) -> Option<u32> {
        let status = self.ok(repo, &["daemon", "status"]);
        let pid = status
            .split_whitespace()
            .find_map(|w| w.strip_prefix("pid="));
        pid.map(|p| p.parse().unwrap())
    }
}

#[test]
fn first_query_starts_a_daemon_and_later_queries_reuse_it() {
    let rt = Runtime::new();
    let repo = fixture("call_repo");
    assert_eq!(rt.pid(&repo), None);
    let want = "resolved app.main.main -> app.util.helper app/main.py:8\n";
    assert_eq!(
        rt.ok(&repo, &["callers", "app.util.helper"]).lines().next(),
        want.lines().next()
    );
    let pid = rt.pid(&repo).expect("a daemon after the first query");
    rt.ok(&repo, &["callers", "app.util.helper"]);
    assert_eq!(rt.pid(&repo), Some(pid));
}
