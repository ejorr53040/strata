use std::collections::BTreeMap;

/// Known keys and their built-in defaults.
const DEFAULTS: &[(&str, &str)] = &[
    ("agent.provider", "anthropic"),
    ("agent.model", "claude-sonnet-5-5"),
    ("daemon.idle_timeout_secs", "600"),
    ("index.include", ""),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    Global,
    Repo,
    Env,
    Flag,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::Global => "global",
            Source::Repo => "repo",
            Source::Env => "env",
            Source::Flag => "flag",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub value: String,
    pub source: Source,
}

/// Raw inputs from each layer. `global`/`repo` are TOML text; `env` is (NAME, value) pairs;
/// `flags` are (dotted.key, value) pairs.
#[derive(Debug, Clone, Default)]
pub struct Layers {
    pub global: Option<String>,
    pub repo: Option<String>,
    pub env: Vec<(String, String)>,
    pub flags: Vec<(String, String)>,
}

/// Resolve effective config: default < global < repo < env < flag.
pub fn resolve(layers: &Layers) -> Result<BTreeMap<String, Entry>, String> {
    let mut out: BTreeMap<String, Entry> = DEFAULTS
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                Entry {
                    value: v.to_string(),
                    source: Source::Default,
                },
            )
        })
        .collect();

    if let Some(g) = &layers.global {
        for (k, v) in flatten(g, "global config")? {
            out.insert(
                k,
                Entry {
                    value: v,
                    source: Source::Global,
                },
            );
        }
    }
    if let Some(r) = &layers.repo {
        for (k, v) in flatten(r, "repo config")? {
            if k.ends_with("api_key") {
                return Err(format!(
                    "`{k}` found in per-repo config; API keys belong in credentials.toml, not in the repo"
                ));
            }
            out.insert(
                k,
                Entry {
                    value: v,
                    source: Source::Repo,
                },
            );
        }
    }
    for (name, v) in &layers.env {
        let known = DEFAULTS.iter().find(|(k, _)| env_name(k) == *name);
        if let Some((k, _)) = known {
            out.insert(
                k.to_string(),
                Entry {
                    value: v.clone(),
                    source: Source::Env,
                },
            );
        }
    }
    for (k, v) in &layers.flags {
        out.insert(
            k.clone(),
            Entry {
                value: v.clone(),
                source: Source::Flag,
            },
        );
    }
    Ok(out)
}

pub fn env_name(key: &str) -> String {
    format!("STRATA_{}", key.replace('.', "_").to_uppercase())
}

fn flatten(text: &str, what: &str) -> Result<Vec<(String, String)>, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("{what}: {e}"))?;
    let mut out = Vec::new();
    fn go(prefix: &str, t: &toml::Table, out: &mut Vec<(String, String)>) {
        for (k, v) in t {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                toml::Value::Table(inner) => go(&key, inner, out),
                toml::Value::String(s) => out.push((key, s.clone())),
                toml::Value::Array(items) => {
                    let parts: Vec<String> = items
                        .iter()
                        .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_string))
                        .collect();
                    out.push((key, parts.join(",")));
                }
                other => out.push((key, other.to_string())),
            }
        }
    }
    go("", &table, &mut out);
    Ok(out)
}

/// Read all layers from disk and the process environment.
/// `config_home` is the user config dir (e.g. `$XDG_CONFIG_HOME`); files live in `<config_home>/strata/`.
pub fn load(
    config_home: &std::path::Path,
    repo_root: &std::path::Path,
    env: impl Iterator<Item = (String, String)>,
    flags: Vec<(String, String)>,
) -> Result<Layers, String> {
    let dir = config_home.join("strata");
    check_credentials(&dir.join("credentials.toml"))?;
    Ok(Layers {
        global: read_opt(&dir.join("config.toml"))?,
        repo: read_opt(&repo_root.join(".strata/config.toml"))?,
        env: env.filter(|(k, _)| k.starts_with("STRATA_")).collect(),
        flags,
    })
}

fn read_opt(path: &std::path::Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Read `<config_home>/strata/credentials.toml` (empty if absent), refusing it unless mode 0600.
pub fn read_credentials(config_home: &std::path::Path) -> Result<toml::Table, String> {
    let path = config_home.join("strata/credentials.toml");
    check_credentials(&path)?;
    match read_opt(&path)? {
        // Never echo file contents: a parse error from `toml` can quote the offending line.
        Some(text) => text
            .parse()
            .map_err(|_| format!("{}: not valid TOML", path.display())),
        None => Ok(toml::Table::new()),
    }
}

/// credentials.toml must be readable by the owner only.
fn check_credentials(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) => {
            let mode = m.permissions().mode() & 0o777;
            if mode != 0o600 {
                return Err(format!(
                    "{} has mode {mode:04o}; it must be mode 0600 (chmod 600 {})",
                    path.display(),
                    path.display()
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Identifies one build of the daemon binary at `exe` (its mtime). The client sends the id of
/// the binary next to it; a daemon started from an older build sees a mismatch and steps aside.
pub fn build_id(exe: &std::path::Path) -> Result<String, String> {
    let modified = std::fs::metadata(exe)
        .and_then(|m| m.modified())
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let since = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(since.as_nanos().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn precedence_is_default_then_global_then_repo_then_env_then_flag() {
        let env = [("STRATA_AGENT_MODEL".to_string(), "from-env".to_string())];
        let flags = [("agent.model".to_string(), "from-flag".to_string())];

        let only_default = resolve(&Layers::default()).unwrap();
        assert_eq!(only_default["agent.provider"].value, "anthropic");
        assert_eq!(only_default["agent.provider"].source, Source::Default);

        let g = Layers {
            global: raw("[agent]\nmodel = \"g\""),
            ..Default::default()
        };
        assert_eq!(resolve(&g).unwrap()["agent.model"].source, Source::Global);

        let r = Layers {
            repo: raw("[agent]\nmodel = \"r\""),
            ..g.clone()
        };
        let got = &resolve(&r).unwrap()["agent.model"];
        assert_eq!((got.value.as_str(), got.source), ("r", Source::Repo));

        let e = Layers {
            env: env.to_vec(),
            ..r.clone()
        };
        let got = &resolve(&e).unwrap()["agent.model"];
        assert_eq!((got.value.as_str(), got.source), ("from-env", Source::Env));

        let f = Layers {
            flags: flags.to_vec(),
            ..e
        };
        let got = &resolve(&f).unwrap()["agent.model"];
        assert_eq!(
            (got.value.as_str(), got.source),
            ("from-flag", Source::Flag)
        );
    }

    #[test]
    fn api_key_in_repo_config_is_rejected() {
        let l = Layers {
            repo: raw("[agent]\napi_key = \"sk-123\""),
            ..Default::default()
        };
        let err = resolve(&l).unwrap_err();
        assert!(err.contains("api_key"), "{err}");
    }
}
