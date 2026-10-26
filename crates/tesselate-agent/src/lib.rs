//! Model provider abstraction for the tesselate agent. `Provider` is the seam; `Anthropic` is the
//! first implementation (streaming Messages API with tool use and prompt caching).

mod anthropic;
mod usage;

pub use anthropic::Anthropic;

/// Find the Anthropic API key: `ANTHROPIC_API_KEY` in `env`, else `anthropic_api_key` in
/// `<config_home>/tesselate/credentials.toml` (which must be mode 0600). Errors never contain the key.
pub fn resolve_api_key(
    config_home: &std::path::Path,
    mut env: impl Iterator<Item = (String, String)>,
) -> Result<String, String> {
    if let Some((_, v)) = env.find(|(k, v)| k == "ANTHROPIC_API_KEY" && !v.is_empty()) {
        return Ok(v);
    }
    let creds = tesselate_config::read_credentials(config_home)?;
    match creds.get("anthropic_api_key").and_then(|v| v.as_str()) {
        Some(k) if !k.is_empty() => Ok(k.to_string()),
        _ => Err("no Anthropic API key: set ANTHROPIC_API_KEY or add \
                  `anthropic_api_key = \"...\"` to ~/.config/tesselate/credentials.toml (mode 0600)"
            .to_string()),
    }
}
pub use usage::Usage;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text(String),
    ToolUse(ToolCall),
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: &'static str,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user_text(text: &str) -> Message {
        Message {
            role: "user",
            content: vec![Block::Text(text.to_string())],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub max_tokens: u32,
    pub system: Option<String>,
    pub tools: Vec<Tool>,
    pub messages: Vec<Message>,
}

/// Incremental output surfaced while a response streams in.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Text(String),
    /// A tool call whose input JSON has been fully received.
    ToolUse(ToolCall),
}

/// The fully assembled response once the stream ends.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: String,
    pub usage: Usage,
}

#[derive(Debug)]
pub enum ProviderError {
    Transport(String),
    /// The API reported an error: an HTTP error status, or an `error` event mid-stream
    /// (`status` is `None` then).
    Api {
        status: Option<u16>,
        kind: String,
        message: String,
    },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Transport(m) => write!(f, "{m}"),
            ProviderError::Api {
                status: Some(s),
                kind,
                message,
            } => {
                write!(f, "API error {s} ({kind}): {message}")
            }
            ProviderError::Api {
                status: None,
                kind,
                message,
            } => {
                write!(f, "API error ({kind}): {message}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

pub trait Provider {
    /// Send `req`, calling `on_event` for each streamed event, and return the assembled response.
    fn stream(
        &self,
        req: &Request,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion, ProviderError>;
}
