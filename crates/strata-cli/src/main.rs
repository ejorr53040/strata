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

fn graph_of(root: &std::path::Path, set: &[String]) -> Result<strata_core::graph::Graph, String> {
    let sources = strata_core::graph::load_with(root, &walk_options(root, set)?);
    Ok(strata_core::graph::build(&sources, &[]))
}

fn print_edges(edges: Vec<&strata_core::graph::Edge>, json: bool) {
    if json {
        println!("{}", serde_json::to_string(&edges).unwrap());
    } else {
        for e in edges {
            println!("{}", e.to_line());
        }
    }
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
        Cmd::Callers { symbol, json } => {
            let graph = graph_of(&root, &cli.set)?;
            print_edges(graph.callers(&symbol), json);
        }
        Cmd::Refs { symbol, json } => {
            let graph = graph_of(&root, &cli.set)?;
            print_edges(graph.references(&symbol), json);
        }
        Cmd::Callees { symbol, json } => {
            let graph = graph_of(&root, &cli.set)?;
            print_edges(graph.callees(&symbol), json);
        }
        Cmd::Smoke { prompt, model } => smoke(&root, &cli.set, &prompt, model)?,
        Cmd::Tools => println!(
            "{}",
            serde_json::to_string_pretty(&strata_core::tools::catalog()).unwrap()
        ),
        Cmd::Map { level, budget } => {
            let opts = walk_options(&root, &cli.set)?;
            print!(
                "{}",
                strata_core::map::build(&root, &opts, level, budget).text
            );
        }
        Cmd::Guide => print!("{}", include_str!("../../../docs/agents/using-strata.md")),
        Cmd::Call { tool, args } => {
            let args: serde_json::Value =
                serde_json::from_str(&args).map_err(|e| format!("arguments are not JSON: {e}"))?;
            let opts = walk_options(&root, &cli.set)?;
            let out = strata_core::tools::call(&root, &opts, &tool, &args)?;
            println!("{}", serde_json::to_string(&out).unwrap());
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
