mod client;

use clap::{Parser, Subcommand};
use serde_json::json;

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
    /// Call sites that target a symbol (resolved and candidate edges)
    Callers {
        symbol: String,
        #[arg(long)]
        json: bool,
    },
    /// Every connection (call, type use, ...) whose targets include a symbol
    Refs {
        symbol: String,
        #[arg(long)]
        json: bool,
    },
    /// Call sites inside a symbol, with each edge's state
    Callees {
        symbol: String,
        #[arg(long)]
        json: bool,
    },
    /// Print the agent tool catalog (JSON: name, description, input_schema)
    Tools,
    /// Run one agent tool: `strata call callees '{"symbol":"graph.build"}'`
    Call {
        tool: String,
        /// JSON object of arguments (default `{}`)
        #[arg(default_value = "{}")]
        args: String,
    },
    /// The whole repo as compact text: modules, signatures, docs and connections
    Map {
        /// Detail 0 (definitions only) to 3 (every connection)
        #[arg(long)]
        level: Option<u8>,
        /// Pick the richest level that fits this many tokens (estimated)
        #[arg(long)]
        budget: Option<usize>,
    },
    /// Print how agents should use strata
    Guide,
    /// Find symbols by name substring
    Symbols {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// The per-repo daemon that holds the index (started by the first query)
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// Send one message to the model provider and stream the reply (checks key + connectivity)
    Smoke {
        #[arg(default_value = "Say hello in five words.")]
        prompt: String,
        /// Model to use (default: effective `agent.model`)
        #[arg(long)]
        model: Option<String>,
    },
}

fn smoke(
    root: &std::path::Path,
    set: &[String],
    prompt: &str,
    model: Option<String>,
) -> Result<(), String> {
    use std::io::Write;
    use strata_agent::{Anthropic, Message, Provider, Request, StreamEvent};

    let key = strata_agent::resolve_api_key(&config_home(), std::env::vars())?;
    let model = match model {
        Some(m) => m,
        None => effective_config(root, set)?["agent.model"].value.clone(),
    };
    let base = std::env::var("STRATA_ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
    let req = Request {
        model: model.clone(),
        max_tokens: 256,
        system: None,
        tools: vec![],
        messages: vec![Message::user_text(prompt)],
    };
    let done = Anthropic::new(&key, &base)
        .stream(&req, &mut |e| {
            if let StreamEvent::Text(t) = e {
                print!("{t}");
                let _ = std::io::stdout().flush();
            }
        })
        .map_err(|e| e.to_string())?;
    println!();
    let u = done.usage;
    let cost = u
        .cost_usd(&model)
        .map_or("unknown".to_string(), |c| format!("${c:.4}"));
    eprintln!(
        "usage: in={} out={} cache_write={} cache_read={} cost={cost}",
        u.input_tokens, u.output_tokens, u.cache_creation_input_tokens, u.cache_read_input_tokens
    );
    Ok(())
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Whether a daemon runs for this repo: pid, indexed files, index age
    Status,
    /// Stop this repo's daemon (the next query starts a fresh one)
    Stop,
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
) -> Result<std::collections::BTreeMap<String, strata_config::Entry>, String> {
    let flags = set
        .iter()
        .map(|s| {
            s.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| format!("--set expects KEY=VALUE, got `{s}`"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let layers = strata_config::load(&config_home(), root, std::env::vars(), flags)?;
    strata_config::resolve(&layers)
}

/// The effective `index.include`, which picks the daemon's snapshot.
fn include(root: &std::path::Path, set: &[String]) -> Result<String, String> {
    Ok(effective_config(root, set)?["index.include"].value.clone())
}

fn query(
    root: &std::path::Path,
    set: &[String],
    cmd: &str,
    args: serde_json::Value,
) -> Result<(), String> {
    print!("{}", client::query(root, &include(root, set)?, cmd, args)?);
    Ok(())
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
    let set = &cli.set;
    match cli.cmd {
        Cmd::Config {
            cmd: ConfigCmd::Show,
        } => {
            for (k, e) in effective_config(&root, set)? {
                println!("{k} = {} ({})", e.value, e.source.label());
            }
        }
        Cmd::Files { unindexed } => {
            if !unindexed {
                return Err("only `files --unindexed` is supported for now".into());
            }
            query(&root, set, "files_unindexed", json!({}))?
        }
        Cmd::Callers { symbol, json } => query(
            &root,
            set,
            "callers",
            json!({"symbol": symbol, "json": json}),
        )?,
        Cmd::Refs { symbol, json } => {
            query(&root, set, "refs", json!({"symbol": symbol, "json": json}))?
        }
        Cmd::Callees { symbol, json } => query(
            &root,
            set,
            "callees",
            json!({"symbol": symbol, "json": json}),
        )?,
        Cmd::Smoke { prompt, model } => smoke(&root, set, &prompt, model)?,
        Cmd::Tools => query(&root, set, "tools", json!({}))?,
        Cmd::Map { level, budget } => {
            query(&root, set, "map", json!({"level": level, "budget": budget}))?
        }
        Cmd::Guide => print!("{}", include_str!("../../../docs/agents/using-strata.md")),
        Cmd::Call { tool, args } => {
            let args: serde_json::Value =
                serde_json::from_str(&args).map_err(|e| format!("arguments are not JSON: {e}"))?;
            query(&root, set, "call", json!({"tool": tool, "args": args}))?
        }
        Cmd::Symbols { query: q, json } => {
            query(&root, set, "symbols", json!({"query": q, "json": json}))?
        }
        Cmd::Daemon { cmd } => {
            let cmd = match cmd {
                DaemonCmd::Status => "status",
                DaemonCmd::Stop => "stop",
            };
            print!("{}", client::control(&root, &include(&root, set)?, cmd)?);
        }
    }
    Ok(())
}
