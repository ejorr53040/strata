#![allow(dead_code)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `strata` with a runtime dir under target/ instead of the user's, and daemons that exit
/// after a few idle seconds so test runs leave nothing behind.
pub fn strata() -> Command {
    strata_in_runtime(&shared_runtime())
}

pub fn strata_in_runtime(runtime: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_strata"));
    c.env("XDG_RUNTIME_DIR", runtime)
        .env("STRATA_DAEMON_IDLE_TIMEOUT_SECS", "5");
    c
}

fn shared_runtime() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rt");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
    p
}
