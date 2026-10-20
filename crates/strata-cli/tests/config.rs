mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;

fn strata(repo: &Path, xdg: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    common::strata()
        .args(args)
        .current_dir(repo)
        .env("XDG_CONFIG_HOME", xdg)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn show(out: &Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn config_show_prints_each_value_with_its_source() {
    let repo = tempfile::tempdir().unwrap();
    let xdg = tempfile::tempdir().unwrap();
    write(
        &xdg.path().join("strata/config.toml"),
        "[agent]\nprovider = \"openai\"\nmodel = \"g\"\n",
    );
    write(
        &repo.path().join(".strata/config.toml"),
        "[agent]\nmodel = \"r\"\n",
    );

    let text = show(&strata(repo.path(), xdg.path(), &[], &["config", "show"]));
    assert!(text.contains("agent.provider = openai (global)"), "{text}");
    assert!(text.contains("agent.model = r (repo)"), "{text}");
    assert!(
        text.contains("daemon.idle_timeout_secs = 600 (default)"),
        "{text}"
    );

    let text = show(&strata(
        repo.path(),
        xdg.path(),
        &[("STRATA_AGENT_MODEL", "e")],
        &["config", "show"],
    ));
    assert!(text.contains("agent.model = e (env)"), "{text}");

    let text = show(&strata(
        repo.path(),
        xdg.path(),
        &[("STRATA_AGENT_MODEL", "e")],
        &["--set", "agent.model=f", "config", "show"],
    ));
    assert!(text.contains("agent.model = f (flag)"), "{text}");
}

#[test]
fn credentials_file_is_refused_unless_mode_0600() {
    let repo = tempfile::tempdir().unwrap();
    let xdg = tempfile::tempdir().unwrap();
    let creds = xdg.path().join("strata/credentials.toml");
    write(&creds, "anthropic_api_key = \"sk-secret\"\n");

    fs::set_permissions(&creds, fs::Permissions::from_mode(0o644)).unwrap();
    let out = strata(repo.path(), xdg.path(), &[], &["config", "show"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("0600"));

    fs::set_permissions(&creds, fs::Permissions::from_mode(0o600)).unwrap();
    let text = show(&strata(repo.path(), xdg.path(), &[], &["config", "show"]));
    assert!(!text.contains("sk-secret"), "secrets must never be printed");
}

#[test]
fn api_key_in_repo_config_is_an_error() {
    let repo = tempfile::tempdir().unwrap();
    let xdg = tempfile::tempdir().unwrap();
    write(
        &repo.path().join(".strata/config.toml"),
        "[agent]\napi_key = \"sk-leak\"\n",
    );
    let out = strata(repo.path(), xdg.path(), &[], &["config", "show"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("api_key") && !err.contains("sk-leak"), "{err}");
}
