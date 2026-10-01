#[path = "../../strata-agent/tests/common/mod.rs"]
mod common;

use common::FakeServer;
use std::process::{Command, Output};

const TEXT: &str = include_str!("../../strata-agent/tests/fixtures/text.sse");

fn smoke(server: &str, xdg: &std::path::Path, key: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strata"));
    cmd.arg("smoke")
        .args(args)
        .env_remove("ANTHROPIC_API_KEY")
        .env("XDG_CONFIG_HOME", xdg)
        .env("STRATA_ANTHROPIC_BASE_URL", server);
    if let Some(k) = key {
        cmd.env("ANTHROPIC_API_KEY", k);
    }
    cmd.output().unwrap()
}

#[test]
fn smoke_streams_a_reply_and_reports_usage_and_cost_without_printing_the_key() {
    let xdg = tempfile::tempdir().unwrap();
    let server = FakeServer::once(200, TEXT);
    let out = smoke(
        &server.url,
        xdg.path(),
        Some("sk-smoke-secret"),
        &["say hi"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stdout.contains("Hello, world!"), "{stdout}");
    assert!(
        stderr.contains("usage: in=25 out=15 cache_write=1000 cache_read=2000 cost=$0.0031"),
        "{stderr}"
    );
    assert!(!stdout.contains("sk-smoke-secret") && !stderr.contains("sk-smoke-secret"));

    let req = server.request();
    assert_eq!(req.header("x-api-key"), Some("sk-smoke-secret"));
    assert_eq!(req.body["messages"][0]["content"][0]["text"], "say hi");
}

#[test]
fn smoke_without_a_key_fails_and_says_how_to_provide_one() {
    let xdg = tempfile::tempdir().unwrap();
    let out = smoke("http://127.0.0.1:9", xdg.path(), None, &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ANTHROPIC_API_KEY"));
}

#[test]
fn smoke_reports_api_errors() {
    let xdg = tempfile::tempdir().unwrap();
    let body =
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    let server = FakeServer::once(401, body);
    let out = smoke(&server.url, xdg.path(), Some("bad"), &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("authentication_error"));
}
