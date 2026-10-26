use std::fs;
use std::os::unix::fs::PermissionsExt;
use tesselate_agent::resolve_api_key;

fn creds(dir: &tempfile::TempDir, text: &str, mode: u32) {
    let p = dir.path().join("tesselate/credentials.toml");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, text).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
}

fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn env_var_wins_over_credentials_file() {
    let dir = tempfile::tempdir().unwrap();
    creds(&dir, "anthropic_api_key = \"from-file\"\n", 0o600);
    let key = resolve_api_key(
        dir.path(),
        env(&[("ANTHROPIC_API_KEY", "from-env")]).into_iter(),
    );
    assert_eq!(key.unwrap(), "from-env");
}

#[test]
fn falls_back_to_credentials_file() {
    let dir = tempfile::tempdir().unwrap();
    creds(&dir, "anthropic_api_key = \"from-file\"\n", 0o600);
    assert_eq!(
        resolve_api_key(dir.path(), env(&[]).into_iter()).unwrap(),
        "from-file"
    );
}

#[test]
fn credentials_file_with_loose_mode_is_refused_without_leaking_the_key() {
    let dir = tempfile::tempdir().unwrap();
    creds(&dir, "anthropic_api_key = \"sk-leaky\"\n", 0o644);
    let err = resolve_api_key(dir.path(), env(&[]).into_iter()).unwrap_err();
    assert!(err.contains("0600"), "{err}");
    assert!(!err.contains("sk-leaky"));
}

#[test]
fn missing_key_says_where_to_put_it() {
    let dir = tempfile::tempdir().unwrap();
    let err = resolve_api_key(dir.path(), env(&[]).into_iter()).unwrap_err();
    assert!(
        err.contains("ANTHROPIC_API_KEY") && err.contains("credentials.toml"),
        "{err}"
    );
}
