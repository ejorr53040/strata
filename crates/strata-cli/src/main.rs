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
        Cmd::Smoke { prompt, model } => smoke(&root, &cli.set, &prompt, model)?,
        Cmd::Symbols { query, json } => {
            let syms = strata_core::index::symbols(&root, &query);
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
