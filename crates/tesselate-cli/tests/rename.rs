//! The rename from strata to tesselate is a hard break: the new names work and the old ones
//! do nothing. (The old name is spelled out here to prove it is dead, so the guard allows this
//! file.)

mod common;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn call_repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/call_repo")
}

/// A runtime dir only this test uses, so its daemons are its own.
fn private_runtime() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

fn stdout(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn tssl_in_runtime(runtime: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tssl"));
    c.env("XDG_RUNTIME_DIR", runtime)
        .env("TESSELATE_DAEMON_IDLE_TIMEOUT_SECS", "5");
    c
}

#[test]
fn tssl_answers_exactly_like_tesselate() {
    let rt = private_runtime();
    let repo = call_repo();
    for args in [
        &["symbols", "helper"][..],
        &["callers", "app.util.helper"],
        &["map", "--level", "1"],
    ] {
        let full = stdout(
            common::tesselate_in_runtime(rt.path())
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap(),
        );
        let short = stdout(
            tssl_in_runtime(rt.path())
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap(),
        );
        assert!(!full.is_empty(), "{args:?} printed nothing");
        assert_eq!(short, full, "{args:?}");
    }
}

#[test]
fn the_daemon_keeps_its_socket_under_a_tesselate_directory() {
    let rt = private_runtime();
    stdout(
        common::tesselate_in_runtime(rt.path())
            .args(["symbols", "helper"])
            .current_dir(call_repo())
            .output()
            .unwrap(),
    );
    let sockets: Vec<_> = std::fs::read_dir(rt.path().join("tesselate"))
        .expect("a tesselate runtime directory")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sock"))
        .collect();
    assert_eq!(sockets.len(), 1);
    assert!(!rt.path().join("strata").exists());
}

#[test]
fn a_daemon_left_over_from_the_old_name_is_never_contacted() {
    let rt = private_runtime();
    let repo = call_repo();
    let query = |rt: &Path| {
        stdout(
            common::tesselate_in_runtime(rt)
                .args(["symbols", "helper"])
                .current_dir(&repo)
                .output()
                .unwrap(),
        )
    };
    let want = query(rt.path());
    // The old name's socket for this same repo would sit at the same hash in `strata/`.
    let sock = std::fs::read_dir(rt.path().join("tesselate"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "sock"))
        .unwrap();
    stdout(
        common::tesselate_in_runtime(rt.path())
            .args(["daemon", "stop"])
            .current_dir(&repo)
            .output()
            .unwrap(),
    );
    let old_dir = rt.path().join("strata");
    std::fs::create_dir(&old_dir).unwrap();
    let old = UnixListener::bind(old_dir.join(sock.file_name().unwrap())).unwrap();
    old.set_nonblocking(true).unwrap();
    assert_eq!(query(rt.path()), want);
    assert!(
        old.accept().is_err(),
        "the client connected to an old-name daemon's socket"
    );
}

/// A throwaway repo with `scratch/tmp.py` defining `tmp_fn`, and one ignore file hiding it.
fn repo_ignoring_scratch_with(ignore_file: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("scratch")).unwrap();
    std::fs::write(
        dir.path().join("scratch/tmp.py"),
        "def tmp_fn():\n    pass\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("a.py"), "def kept_fn():\n    pass\n").unwrap();
    std::fs::write(dir.path().join(ignore_file), "scratch/\n").unwrap();
    dir
}

fn symbols(repo: &Path, query: &str) -> String {
    let rt = private_runtime();
    stdout(
        common::tesselate_in_runtime(rt.path())
            .args(["symbols", query])
            .current_dir(repo)
            .output()
            .unwrap(),
    )
}

#[test]
fn the_new_ignore_file_hides_files_and_the_old_one_does_not() {
    let new = repo_ignoring_scratch_with(".tesselateignore");
    assert_eq!(symbols(new.path(), "tmp_fn"), "");
    let old = repo_ignoring_scratch_with(".strataignore");
    assert!(
        symbols(old.path(), "tmp_fn").contains("scratch/tmp.py"),
        "the old ignore file still hides files"
    );
}

/// `config show` for a repo with a clean home, an optional config file, and some environment.
fn config_show(config_file: Option<(&str, &str)>, env: &[(&str, &str)]) -> String {
    let home = tempfile::tempdir().unwrap();
    if let Some((dir, text)) = config_file {
        std::fs::create_dir_all(home.path().join(dir)).unwrap();
        std::fs::write(home.path().join(dir).join("config.toml"), text).unwrap();
    }
    let mut c = common::tesselate_in_runtime(home.path());
    c.args(["config", "show"])
        .current_dir(call_repo())
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path());
    for (k, v) in env {
        c.env(k, v);
    }
    stdout(c.output().unwrap())
}

#[test]
fn only_tesselate_environment_variables_are_read() {
    assert!(config_show(None, &[("TESSELATE_AGENT_MODEL", "new-model")]).contains("new-model"));
    assert!(!config_show(None, &[("STRATA_AGENT_MODEL", "old-model")]).contains("old-model"));
}

#[test]
fn only_the_tesselate_config_directory_is_read() {
    let text = "[agent]\nmodel = \"file-model\"\n";
    assert!(config_show(Some(("tesselate", text)), &[]).contains("file-model"));
    assert!(!config_show(Some(("strata", text)), &[]).contains("file-model"));
}
