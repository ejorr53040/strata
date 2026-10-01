use crate::{Block, Completion, Provider, ProviderError, Request, StreamEvent, ToolCall, Usage};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};

pub struct Anthropic {
    api_key: String,
    base_url: String,
    agent: ureq::Agent,
}

impl Anthropic {
    pub fn new(api_key: &str, base_url: &str) -> Anthropic {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Anthropic {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            agent,
        }
    }
}

fn body(req: &Request) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| {
            let content: Vec<Value> = m
                .content
                .iter()
                .map(|b| match b {
                    Block::Text(t) => json!({"type": "text", "text": t}),
                    Block::ToolUse(c) => {
                        json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.input})
                    }
                    Block::ToolResult { tool_use_id, content } => {
                        json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": content})
                    }
                })
                .collect();
            json!({"role": m.role, "content": content})
        })
        .collect();
    let mut out = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "messages": messages,
    });
    if !req.tools.is_empty() {
        out["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({"name": t.name, "description": t.description, "input_schema": t.input_schema})
            })
            .collect();
    }
    if let Some(system) = &req.system {
        out["system"] = json!([{"type": "text", "text": system}]);
    }
    // Cache breakpoints (3 of the 4 allowed): the end of each prefix that stays stable across
    // turns. Order is tools -> system -> messages, so each marks a growing prefix.
    let ephemeral = json!({"type": "ephemeral"});
    if let Some(last) = out["tools"].as_array_mut().and_then(|t| t.last_mut()) {
        last["cache_control"] = ephemeral.clone();
    }
    if let Some(last) = out["system"].as_array_mut().and_then(|s| s.last_mut()) {
        last["cache_control"] = ephemeral.clone();
    }
    if let Some(last) = out["messages"]
        .as_array_mut()
        .and_then(|m| m.last_mut())
        .and_then(|m| m["content"].as_array_mut())
        .and_then(|c| c.last_mut())
    {
        last["cache_control"] = ephemeral;
    }
    out
}

/// A tool_use block whose JSON input is still arriving as `input_json_delta` fragments.
struct PendingTool {
    id: String,
    name: String,
    json: String,
}

impl Provider for Anthropic {
    fn stream(
        &self,
        req: &Request,
        on_event: &mut dyn FnMut(StreamEvent),
    ) -> Result<Completion, ProviderError> {
        let resp = self
            .agent
            .post(&format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send(body(req).to_string())
            .map_err(|e| ProviderError::Transport(e.to_string()))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            return Err(api_error(Some(status), &v["error"], &text));
        }

        let reader = BufReader::new(resp.into_body().into_reader());
        let mut done = Completion::default();
        let mut data = String::new();
        let mut pending: Option<PendingTool> = None;
        for line in reader.lines() {
            let line = line.map_err(|e| ProviderError::Transport(e.to_string()))?;
            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            } else if line.is_empty() && !data.is_empty() {
                let ev: Value = serde_json::from_str(&data)
                    .map_err(|e| ProviderError::Transport(format!("bad SSE data: {e}")))?;
                data.clear();
                apply(&ev, &mut done, &mut pending, on_event)?;
            }
        }
        Ok(done)
    }
}

fn api_error(status: Option<u16>, err: &Value, raw: &str) -> ProviderError {
    ProviderError::Api {
        status,
        kind: err["type"].as_str().unwrap_or("unknown_error").to_string(),
        message: err["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| raw.to_string()),
    }
}

/// Usage fields are cumulative; later events overwrite only the fields they carry.
fn merge_usage(into: &mut Usage, u: &Value) {
    let set = |field: &mut u64, key: &str| {
        if let Some(n) = u[key].as_u64() {
            *field = n;
        }
    };
    set(&mut into.input_tokens, "input_tokens");
    set(&mut into.output_tokens, "output_tokens");
    set(
        &mut into.cache_creation_input_tokens,
        "cache_creation_input_tokens",
    );
    set(&mut into.cache_read_input_tokens, "cache_read_input_tokens");
}

fn apply(
    ev: &Value,
    done: &mut Completion,
    pending: &mut Option<PendingTool>,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<(), ProviderError> {
    match ev["type"].as_str() {
        Some("content_block_start") if ev["content_block"]["type"] == "tool_use" => {
            let b = &ev["content_block"];
            *pending = Some(PendingTool {
                id: b["id"].as_str().unwrap_or_default().to_string(),
                name: b["name"].as_str().unwrap_or_default().to_string(),
                json: String::new(),
            });
        }
        Some("content_block_delta") if ev["delta"]["type"] == "input_json_delta" => {
            if let Some(p) = pending {
                p.json
                    .push_str(ev["delta"]["partial_json"].as_str().unwrap_or_default());
            }
        }
        Some("content_block_stop") => {
            if let Some(p) = pending.take() {
                let input = if p.json.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&p.json).map_err(|e| {
                        ProviderError::Transport(format!("bad tool input JSON for {}: {e}", p.name))
                    })?
                };
                let call = ToolCall {
                    id: p.id,
                    name: p.name,
                    input,
                };
                done.tool_calls.push(call.clone());
                on_event(StreamEvent::ToolUse(call));
            }
        }
        Some("content_block_delta") if ev["delta"]["type"] == "text_delta" => {
            let t = ev["delta"]["text"].as_str().unwrap_or_default();
            done.text.push_str(t);
            on_event(StreamEvent::Text(t.to_string()));
        }
        Some("error") => return Err(api_error(None, &ev["error"], &ev.to_string())),
        Some("message_start") => merge_usage(&mut done.usage, &ev["message"]["usage"]),
        Some("message_delta") => {
            if let Some(r) = ev["delta"]["stop_reason"].as_str() {
                done.stop_reason = r.to_string();
            }
            merge_usage(&mut done.usage, &ev["usage"]);
        }
        _ => {}
    }
    Ok(())
}
