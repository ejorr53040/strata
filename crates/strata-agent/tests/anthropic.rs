mod common;

use common::FakeServer;
use strata_agent::{
    Anthropic, Block, Message, Provider, ProviderError, Request, StreamEvent, Tool, ToolCall,
};

const TEXT: &str = include_str!("fixtures/text.sse");
const TOOL_USE: &str = include_str!("fixtures/tool_use.sse");

fn hello() -> Request {
    Request {
        model: "claude-sonnet-5-5".into(),
        max_tokens: 256,
        system: None,
        tools: vec![],
        messages: vec![Message::user_text("hi")],
    }
}

#[test]
fn streams_text_deltas_and_sends_a_well_formed_request() {
    let server = FakeServer::once(200, TEXT);
    let provider = Anthropic::new("sk-test-key", &server.url);

    let mut seen = Vec::new();
    let done = provider.stream(&hello(), &mut |e| seen.push(e)).unwrap();

    assert_eq!(
        seen,
        vec![
            StreamEvent::Text("Hello".into()),
            StreamEvent::Text(", world!".into())
        ]
    );
    assert_eq!(done.text, "Hello, world!");
    assert_eq!(done.stop_reason, "end_turn");

    let req = server.request();
    assert_eq!(req.path, "/v1/messages");
    assert_eq!(req.header("x-api-key"), Some("sk-test-key"));
    assert_eq!(req.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(req.body["stream"], true);
    assert_eq!(req.body["model"], "claude-sonnet-5-5");
    assert_eq!(req.body["max_tokens"], 256);
    assert_eq!(req.body["messages"][0]["role"], "user");
    assert_eq!(req.body["messages"][0]["content"][0]["text"], "hi");
}

#[test]
fn sets_cache_breakpoints_on_system_last_tool_and_last_message_block() {
    let server = FakeServer::once(200, TEXT);
    let provider = Anthropic::new("k", &server.url);
    let tool = |n: &str| Tool {
        name: n.into(),
        description: "d".into(),
        input_schema: serde_json::json!({"type": "object"}),
    };
    let mut req = hello();
    req.system = Some("You are strata.".into());
    req.tools = vec![tool("a"), tool("b")];
    req.messages.push(Message {
        role: "assistant",
        content: vec![Block::Text("ok".into())],
    });
    req.messages.push(Message {
        role: "user",
        content: vec![Block::Text("one".into()), Block::Text("two".into())],
    });
    provider.stream(&req, &mut |_| {}).unwrap();

    let body = server.request().body;
    let ephemeral = serde_json::json!({"type": "ephemeral"});
    assert_eq!(body["system"][0]["text"], "You are strata.");
    assert_eq!(body["system"][0]["cache_control"], ephemeral);
    assert!(body["tools"][0].get("cache_control").is_none());
    assert_eq!(body["tools"][1]["cache_control"], ephemeral);
    assert!(body["messages"][2]["content"][0]
        .get("cache_control")
        .is_none());
    assert_eq!(
        body["messages"][2]["content"][1]["cache_control"],
        ephemeral
    );

    let count = body.to_string().matches("cache_control").count();
    assert!(count <= 4, "API allows at most 4 breakpoints, got {count}");
}

#[test]
fn http_errors_surface_the_api_error_type_and_message() {
    let body =
        r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    let server = FakeServer::once(401, body);
    let provider = Anthropic::new("sk-very-secret", &server.url);
    let err = provider.stream(&hello(), &mut |_| {}).unwrap_err();
    match &err {
        ProviderError::Api {
            status,
            kind,
            message,
        } => {
            assert_eq!(*status, Some(401));
            assert_eq!(kind, "authentication_error");
            assert_eq!(message, "invalid x-api-key");
        }
        other => panic!("{other:?}"),
    }
    assert!(!err.to_string().contains("sk-very-secret"));
}

#[test]
fn in_stream_error_events_are_errors() {
    let server = FakeServer::once(200, include_str!("fixtures/error.sse"));
    let provider = Anthropic::new("k", &server.url);
    let err = provider.stream(&hello(), &mut |_| {}).unwrap_err();
    assert!(
        matches!(&err, ProviderError::Api { status: None, kind, .. } if kind == "overloaded_error"),
        "{err:?}"
    );
}

#[test]
fn reports_usage_and_cost_including_cache_tokens() {
    let server = FakeServer::once(200, TEXT);
    let provider = Anthropic::new("k", &server.url);
    let done = provider.stream(&hello(), &mut |_| {}).unwrap();

    let u = done.usage;
    assert_eq!(
        (
            u.input_tokens,
            u.output_tokens,
            u.cache_creation_input_tokens,
            u.cache_read_input_tokens
        ),
        (25, 15, 1000, 2000)
    );
    // Sonnet 5.5, $/MTok: input 2, 5m cache write 2.50, cache read 0.20, output 10.
    // 25*2 + 1000*2.5 + 2000*0.2 + 15*10 = 3100 micro-dollars.
    let cost = u.cost_usd("claude-sonnet-5-5").unwrap();
    assert!((cost - 0.0031).abs() < 1e-9, "{cost}");
    // Opus 5.5 ($4/$20) reads cache at 0.05x: 25*4 + 1000*5 + 2000*0.2 + 15*20 = 5800.
    let cost = u.cost_usd("claude-opus-5-5").unwrap();
    assert!((cost - 0.0058).abs() < 1e-9, "{cost}");
    assert_eq!(u.cost_usd("some-future-model"), None);
}

#[test]
fn streams_tool_use_with_input_assembled_from_json_deltas() {
    let server = FakeServer::once(200, TOOL_USE);
    let provider = Anthropic::new("k", &server.url);

    let mut seen = Vec::new();
    let done = provider.stream(&hello(), &mut |e| seen.push(e)).unwrap();

    let call = ToolCall {
        id: "toolu_01T1x1fJ34qAmk2tNTrN7Up6".into(),
        name: "get_weather".into(),
        input: serde_json::json!({"location": "San Francisco, CA"}),
    };
    assert_eq!(
        seen,
        vec![
            StreamEvent::Text("Checking:".into()),
            StreamEvent::ToolUse(call.clone())
        ]
    );
    assert_eq!(done.tool_calls, vec![call]);
    assert_eq!(done.stop_reason, "tool_use");
}

#[test]
fn request_carries_tool_definitions_and_tool_results() {
    let server = FakeServer::once(200, TEXT);
    let provider = Anthropic::new("k", &server.url);
    let mut req = hello();
    req.tools = vec![Tool {
        name: "get_weather".into(),
        description: "Weather".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {"location": {"type": "string"}}}),
    }];
    req.messages.push(Message {
        role: "assistant",
        content: vec![Block::ToolUse(ToolCall {
            id: "toolu_1".into(),
            name: "get_weather".into(),
            input: serde_json::json!({"location": "SF"}),
        })],
    });
    req.messages.push(Message {
        role: "user",
        content: vec![Block::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: "60F".into(),
        }],
    });
    provider.stream(&req, &mut |_| {}).unwrap();

    let body = server.request().body;
    assert_eq!(body["tools"][0]["name"], "get_weather");
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
    assert_eq!(body["messages"][1]["content"][0]["input"]["location"], "SF");
    let result = &body["messages"][2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    assert_eq!(result["tool_use_id"], "toolu_1");
    assert_eq!(result["content"], "60F");
}
