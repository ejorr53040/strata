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

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A small Python repo in a tempdir, safe to edit.
fn scratch_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "app/util.py", "def helper():\n    return 1\n");
    write(
        dir.path(),
        "app/main.py",
        "from app.util import helper\n\n\ndef main():\n    helper()\n",
    );
    dir
}

#[test]
fn two_repos_get_two_daemons_with_their_own_answers() {
    let rt = Runtime::new();
    let (py, rs) = (fixture("call_repo"), fixture("rust_repo"));
    let py_syms = rt.ok(&py, &["symbols", ""]);
    let rs_syms = rt.ok(&rs, &["symbols", ""]);
    assert!(
        py_syms.contains("app/util.py") && !py_syms.contains(".rs:"),
        "{py_syms}"
    );
    assert!(
        rs_syms.contains(".rs:") && !rs_syms.contains(".py:"),
        "{rs_syms}"
    );
    assert_ne!(rt.pid(&py).unwrap(), rt.pid(&rs).unwrap());
}

#[test]
fn an_edited_file_shows_up_in_the_next_answer() {
    let rt = Runtime::new();
    let repo = scratch_repo();
    assert_eq!(
        rt.ok(repo.path(), &["callers", "app.util.helper"]),
        "resolved app.main.main -> app.util.helper app/main.py:5\n"
    );
    write(
        repo.path(),
        "app/main.py",
        "from app.util import helper\n\n\ndef main():\n    pass\n\n\ndef other():\n    helper()\n",
    );
    assert_eq!(
        rt.ok(repo.path(), &["callers", "app.util.helper"]),
        "resolved app.main.other -> app.util.helper app/main.py:9\n"
    );
}

#[test]
fn a_deleted_file_disappears_from_the_next_answer() {
    let rt = Runtime::new();
    let repo = scratch_repo();
    assert!(rt
        .ok(repo.path(), &["symbols", "helper"])
        .contains("app/util.py"));
    std::fs::remove_file(repo.path().join("app/util.py")).unwrap();
    assert_eq!(rt.ok(repo.path(), &["symbols", "helper"]), "");
}

#[test]
fn a_killed_daemon_is_replaced_on_the_next_query() {
    let rt = Runtime::new();
    let repo = fixture("call_repo");
    rt.ok(&repo, &["symbols", "helper"]);
    let pid = rt.pid(&repo).unwrap();
    // SIGKILL leaves the socket file behind, so this also covers a dead daemon's socket.
    assert!(std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap()
        .success());
    while std::path::Path::new(&format!("/proc/{pid}")).exists() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(rt.pid(&repo), None);
    assert!(rt.ok(&repo, &["symbols", "helper"]).contains("app/util.py"));
    assert_ne!(rt.pid(&repo), Some(pid));
}

#[test]
fn queries_with_another_index_include_get_answers_for_it() {
    let rt = Runtime::new();
    let repo = scratch_repo();
    write(
        repo.path(),
        "node_modules/lib/index.py",
        "def vendored_fn():\n    pass\n",
    );
    assert_eq!(rt.ok(repo.path(), &["symbols", "vendored_fn"]), "");
    let with = rt.ok(
        repo.path(),
        &[
            "--set",
            "index.include=node_modules",
            "symbols",
            "vendored_fn",
        ],
    );
    assert_eq!(with, "function vendored_fn node_modules/lib/index.py:1\n");
    assert_eq!(rt.ok(repo.path(), &["symbols", "vendored_fn"]), "");
}

#[test]
fn local_commands_start_no_daemon() {
    let rt = Runtime::new();
    let repo = fixture("call_repo");
    rt.ok(&repo, &["config", "show"]);
    rt.ok(&repo, &["guide"]);
    assert_eq!(rt.pid(&repo), None);
}

#[test]
fn daemon_stop_stops_it_and_the_next_query_starts_a_new_one() {
    let rt = Runtime::new();
    let repo = fixture("call_repo");
    rt.ok(&repo, &["symbols", "helper"]);
    let pid = rt.pid(&repo).unwrap();
    assert_eq!(rt.ok(&repo, &["daemon", "stop"]), "stopped\n");
    assert_eq!(rt.ok(&repo, &["daemon", "status"]), "not running\n");
    rt.ok(&repo, &["symbols", "helper"]);
    assert_ne!(rt.pid(&repo), Some(pid));
}

#[test]
fn tool_errors_come_back_as_errors() {
    let rt = Runtime::new();
    let out = rt.run(&fixture("call_repo"), &["call", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("error: unknown tool `nope`"), "{err}");
}

/// `strata` and `strata-daemon` copied into a tempdir, so a test can rebuild (touch) the
/// daemon or count its processes without disturbing other tests.
struct Install(tempfile::TempDir);

impl Install {
    fn new() -> Install {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_strata"));
        for name in ["strata", "strata-daemon"] {
            std::fs::copy(bin.with_file_name(name), dir.path().join(name)).unwrap();
        }
        Install(dir)
    }

    fn run(&self, rt: &Runtime, repo: &Path, args: &[&str]) -> Output {
        std::process::Command::new(self.0.path().join("strata"))
            .args(args)
            .current_dir(repo)
            .env("XDG_RUNTIME_DIR", rt.0.path())
            .env("STRATA_DAEMON_IDLE_SECS", "5")
            .output()
            .unwrap()
    }

    /// Live daemon processes started from this install.
    fn daemons(&self) -> usize {
        let exe = self.0.path().join("strata-daemon");
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter(|p| std::fs::read_link(p.path().join("exe")).ok().as_deref() == Some(&exe))
            .count()
    }
}

#[test]
fn a_daemon_from_another_build_is_replaced() {
    let (rt, install, repo) = (Runtime::new(), Install::new(), fixture("call_repo"));
    assert!(install
        .run(&rt, &repo, &["symbols", "helper"])
        .status
        .success());
    let pid = rt.pid(&repo);
    // A rebuild replaces the binary: a new file with a new mtime.
    let daemon = install.0.path().join("strata-daemon");
    let bytes = std::fs::read(&daemon).unwrap();
    std::fs::remove_file(&daemon).unwrap();
    std::fs::write(&daemon, bytes).unwrap();
    std::fs::set_permissions(&daemon, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let out = install.run(&rt, &repo, &["symbols", "helper"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("app/util.py"));
    assert_ne!(rt.pid(&repo), pid);
    let old = format!("/proc/{}", pid.unwrap());
    let start = std::time::Instant::now();
    while Path::new(&old).exists() && start.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!Path::new(&old).exists(), "the old daemon is still running");
    assert_eq!(install.daemons(), 1);
}

#[test]
fn concurrent_cold_queries_start_one_daemon() {
    let (rt, install, repo) = (Runtime::new(), Install::new(), fixture("call_repo"));
    let outs: Vec<Output> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| install.run(&rt, &repo, &["symbols", "helper"])))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for out in &outs {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("app/util.py"));
    }
    assert_eq!(install.daemons(), 1);
}

#[test]
fn a_runtime_dir_too_long_for_a_socket_is_named_in_the_error() {
    let rt = Runtime::new();
    let long = rt.0.path().join("x".repeat(100));
    std::fs::create_dir_all(&long).unwrap();
    let out = common::strata_in_runtime(&long)
        .args(["symbols", "helper"])
        .current_dir(fixture("call_repo"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("XDG_RUNTIME_DIR") && err.contains("too long"),
        "{err}"
    );
}
