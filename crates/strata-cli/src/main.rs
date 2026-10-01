use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "strata", version)]
struct Cli {
    /// Override a config value for this invocation (repeatable): --set agent.model=x
    #[arg(long = "set", value_name = "KEY=VALUE", global = true)]
    set: Vec<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Inspect configuration
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// List files and their index state
    Files {
        /// Only files that were skipped, with the reason
        #[arg(long)]
        unindexed: bool,
    },
    /// Find symbols by name substring
    Symbols {
        query: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print effective config with where each value came from
    Show,
}

fn config_home() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(Into::into)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default()
}

fn effective_config(
    root: &std::path::Path,
    set: &[String],
) -> Result<std::collections::BTreeMap<String, strata_core::config::Entry>, String> {
    let flags = set
        .iter()
        .map(|s| {
            s.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("--set expects KEY=VALUE, got `{s}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let layers = strata_core::config::load(&config_home(), root, std::env::vars(), flags)?;
    strata_core::config::resolve(&layers)
}

fn walk_options(
    root: &std::path::Path,
    set: &[String],
) -> Result<strata_core::walk::WalkOptions, String> {
    let cfg = effective_config(root, set)?;
    Ok(strata_core::walk::WalkOptions::from_config_value(
        &cfg["index.include"].value,
    ))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    match cli.cmd {
        Cmd::Config {
            cmd: ConfigCmd::Show,
        } => {
            for (k, e) in effective_config(&root, &cli.set)? {
                println!("{k} = {} ({})", e.value, e.source.label());
            }
        }
        Cmd::Files { unindexed } => {
            if !unindexed {
                return Err("only `files --unindexed` is supported for now".into());
            }
            for s in strata_core::walk::walk(&root, &walk_options(&root, &cli.set)?).unindexed {
                println!("unindexed {} {}", s.rel, s.reason.label());
            }
        }
        Cmd::Symbols { query, json } => {
            let opts = walk_options(&root, &cli.set)?;
            let syms = strata_core::index::symbols_with(&root, &query, &opts);
            if json {
                println!("{}", serde_json::to_string(&syms).unwrap());
            } else {
                for s in syms {
                    println!("{} {} {}:{}", s.kind, s.name, s.path, s.line);
                }
            }
        }
    }
    Ok(())
}
