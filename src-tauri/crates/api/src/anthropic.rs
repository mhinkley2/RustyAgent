use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::debug;

use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, MessageRole, StreamEvent, ToolCall, ToolDefinition},
};

const BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    client: Client,
    api_key: String,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
        }
    }

    fn build_request_body(
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        config: &CompletionConfig,
    ) -> Value {
        // Extract system prompt from messages or config.
        let system: Option<String> = config.system_prompt.clone().or_else(|| {
            messages
                .iter()
                .find(|m| m.role == MessageRole::System)
                .map(|m| m.content.clone())
        });

        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| {
                match m.role {
                    MessageRole::Assistant => {
                        // If there are tool calls, build a content-block array.
                        if let Some(calls) = &m.tool_calls {
                            let mut blocks: Vec<Value> = Vec::new();
                            if !m.content.is_empty() {
                                blocks.push(json!({ "type": "text", "text": m.content }));
                            }
                            for c in calls {
                                blocks.push(json!({
                                    "type": "tool_use",
                                    "id": c.id,
                                    "name": c.name,
                                    "input": c.input,
                                }));
                            }
                            json!({ "role": "assistant", "content": blocks })
                        } else {
                            json!({ "role": "assistant", "content": m.content })
                        }
                    }
                    MessageRole::Tool => {
                        // Anthropic wraps tool results in a user message with tool_result blocks.
                        json!({
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "tool_use_id": m.tool_call_id,
                                "content": m.content,
                            }]
                        })
                    }
                    _ => json!({ "role": "user", "content": m.content }),
                }
            })
            .collect();

        let api_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let mut body = json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": true,
            "messages": api_messages,
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }
        if let Some(temp) = config.temperature {
            body["temperature"] = json!(temp);
        }
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        body
    }
}

// ---------------------------------------------------------------------------
// SSE decoding
//
// Split out from `stream_completion` as pure functions so the wire format can
// be tested without a network round-trip. `stream_completion` is a thin loop
// over these two.
// ---------------------------------------------------------------------------

/// Partial state for one `tool_use` content block.
///
/// Anthropic streams a tool call as `content_block_start` (carrying the id and
/// name) followed by any number of `input_json_delta` fragments that must be
/// concatenated before they parse as JSON, terminated by `content_block_stop`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ToolAccum {
    id: String,
    name: String,
    json: String,
}

/// Drain every complete SSE block (terminated by a blank line) from `buf`,
/// leaving any trailing partial block behind for the next chunk.
pub(crate) fn split_sse_blocks(buf: &mut String) -> Vec<String> {
    let mut blocks = Vec::new();
    while let Some(pos) = buf.find("\n\n") {
        blocks.push(buf[..pos].to_string());
        buf.drain(..pos + 2);
    }
    blocks
}

/// Turn one SSE block into zero or more `StreamEvent`s, updating the in-flight
/// tool accumulators keyed by content-block index.
pub(crate) fn decode_block(
    block: &str,
    tools: &mut HashMap<u32, ToolAccum>,
) -> Result<Vec<StreamEvent>, ApiError> {
    let mut event_type = String::new();
    let mut data_line = String::new();

    for line in block.lines() {
        if let Some(evt) = line.strip_prefix("event: ") {
            event_type = evt.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data_line = d.trim().to_string();
        }
    }

    if data_line.is_empty() || data_line == "[DONE]" {
        return Ok(Vec::new());
    }

    let parsed: Value = serde_json::from_str(&data_line)?;

    // The `data` payload normally repeats the type; fall back to the `event:`
    // line when it does not.
    let type_field = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or(&event_type)
        .to_string();

    let mut out = Vec::new();

    match type_field.as_str() {
        "content_block_start" => {
            if let Some(cb) = parsed.get("content_block") {
                if cb.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                    let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
                    tools.insert(
                        idx,
                        ToolAccum {
                            id: cb["id"].as_str().unwrap_or("").to_string(),
                            name: cb["name"].as_str().unwrap_or("").to_string(),
                            json: String::new(),
                        },
                    );
                }
            }
        }
        "content_block_delta" => {
            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
            if let Some(delta) = parsed.get("delta") {
                match delta.get("type").and_then(|v| v.as_str()) {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            out.push(StreamEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(acc) = tools.get_mut(&idx) {
                            acc.json
                                .push_str(delta["partial_json"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
            if let Some(acc) = tools.remove(&idx) {
                out.push(finish_tool_call(acc));
            }
        }
        "message_delta" => {
            if let Some(delta) = parsed.get("delta") {
                let stop_reason = delta
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("end_turn")
                    .to_string();
                out.push(StreamEvent::Done { stop_reason });
            }
        }
        "error" => {
            let msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            out.push(StreamEvent::Error(msg));
        }
        _ => {
            debug!("Anthropic SSE unhandled event type: {type_field}");
        }
    }

    Ok(out)
}

/// Parse an accumulated tool block into a `ToolCallDelta`.
///
/// A malformed input is reported as a recoverable `Error` rather than aborting
/// the stream — the rest of the turn is still worth delivering.
fn finish_tool_call(acc: ToolAccum) -> StreamEvent {
    // A tool invoked with no arguments emits no `input_json_delta` at all.
    let trimmed = acc.json.trim();
    if trimmed.is_empty() {
        return StreamEvent::ToolCallDelta(ToolCall {
            id: acc.id,
            name: acc.name,
            input: json!({}),
        });
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(input) => StreamEvent::ToolCallDelta(ToolCall {
            id: acc.id,
            name: acc.name,
            input,
        }),
        Err(e) => StreamEvent::Error(format!(
            "Failed to parse tool input for `{}` (id {}): {e}",
            acc.name, acc.id
        )),
    }
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        let body = Self::build_request_body(&messages, &tools, &config);

        let response = self
            .client
            .post(format!("{BASE_URL}/messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status == 429 {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            return Err(ApiError::RateLimited { retry_after_secs: retry });
        }
        if !response.status().is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(ApiError::Http { status, body: body_text });
        }

        let mut byte_stream = response.bytes_stream();

        let stream = try_stream! {
            // In-flight tool_use blocks, keyed by content-block index.
            let mut tools: HashMap<u32, ToolAccum> = HashMap::new();
            let mut leftover = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                for block in split_sse_blocks(&mut leftover) {
                    for event in decode_block(&block, &mut tools)? {
                        yield event;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        // Anthropic doesn't have a public list-models endpoint; return known models.
        Ok(vec![
            "claude-opus-4-5".to_string(),
            "claude-sonnet-4-5".to_string(),
            "claude-haiku-3-5".to_string(),
            "claude-3-opus-20240229".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- fixtures ----------------------------------------------------------

    /// Build one SSE block in Anthropic's wire format.
    fn sse(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    /// Feed chunks through the same split/decode loop `stream_completion` uses.
    fn drive(chunks: &[&str]) -> Vec<StreamEvent> {
        let mut tools: HashMap<u32, ToolAccum> = HashMap::new();
        let mut leftover = String::new();
        let mut out = Vec::new();
        for chunk in chunks {
            leftover.push_str(chunk);
            for block in split_sse_blocks(&mut leftover) {
                out.extend(decode_block(&block, &mut tools).expect("decode failed"));
            }
        }
        out
    }

    // -- accessors ---------------------------------------------------------

    fn as_text(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::TextDelta(t) => t,
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    fn as_tool(e: &StreamEvent) -> &ToolCall {
        match e {
            StreamEvent::ToolCallDelta(c) => c,
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }
    }

    fn as_done(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::Done { stop_reason } => stop_reason,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn as_error(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::Error(m) => m,
            other => panic!("expected Error, got {other:?}"),
        }
    }

    // -- text ---------------------------------------------------------------

    #[test]
    fn text_deltas_are_yielded_in_order() {
        let events = drive(&[
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo "}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#,
            ),
        ]);

        assert_eq!(events.len(), 3);
        let joined: String = events.iter().map(as_text).collect();
        assert_eq!(joined, "Hello world");
    }

    #[test]
    fn empty_text_delta_is_skipped() {
        let events = drive(&[&sse(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}"#,
        )]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- tool calls (regression: these were silently dropped) ----------------

    #[test]
    fn tool_use_block_yields_tool_call_delta() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_write","input":{}}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.txt\","}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"content\":\"hi\"}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
        ]);

        assert_eq!(events.len(), 1, "expected exactly one event, got {events:?}");
        let call = as_tool(&events[0]);
        assert_eq!(call.id, "toolu_1");
        assert_eq!(call.name, "file_write");
        assert_eq!(call.input, json!({ "path": "a.txt", "content": "hi" }));
    }

    #[test]
    fn tool_use_with_no_arguments_yields_empty_object_input() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"list_stories"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
        ]);

        assert_eq!(events.len(), 1);
        let call = as_tool(&events[0]);
        assert_eq!(call.name, "list_stories");
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn two_concurrent_tool_blocks_are_keyed_by_index() {
        // Anthropic interleaves deltas for parallel tool blocks; the
        // accumulators must not bleed into one another.
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_a","name":"alpha"}}"#,
            ),
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_b","name":"beta"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"b\":"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"2}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":2}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
        ]);

        assert_eq!(events.len(), 2);
        // Emitted in stop order: index 2 first.
        let beta = as_tool(&events[0]);
        assert_eq!(beta.id, "toolu_b");
        assert_eq!(beta.input, json!({ "b": 2 }));

        let alpha = as_tool(&events[1]);
        assert_eq!(alpha.id, "toolu_a");
        assert_eq!(alpha.input, json!({ "a": 1 }));
    }

    #[test]
    fn text_and_tool_use_in_the_same_message_both_survive() {
        let events = drive(&[
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me check."}}"#,
            ),
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_story"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"id\":\"s1\"}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
            &sse(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            ),
        ]);

        assert_eq!(events.len(), 3);
        assert_eq!(as_text(&events[0]), "Let me check.");
        assert_eq!(as_tool(&events[1]).name, "get_story");
        assert_eq!(as_done(&events[2]), "tool_use");
    }

    #[test]
    fn malformed_partial_json_yields_error_not_panic() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_x","name":"broken"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"unterminated\":"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
        ]);

        assert_eq!(events.len(), 1);
        let msg = as_error(&events[0]);
        assert!(msg.contains("broken"), "error should name the tool: {msg}");
        assert!(msg.contains("toolu_x"), "error should name the id: {msg}");
    }

    #[test]
    fn content_block_stop_for_a_text_block_yields_nothing() {
        let events = drive(&[&sse(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        )]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- terminal / error events --------------------------------------------

    #[test]
    fn message_delta_yields_done_with_stop_reason() {
        let events = drive(&[&sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "tool_use");
    }

    #[test]
    fn message_delta_without_stop_reason_defaults_to_end_turn() {
        let events = drive(&[&sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_sequence":null}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "end_turn");
    }

    #[test]
    fn error_event_yields_stream_event_error() {
        let events = drive(&[&sse(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_error(&events[0]), "Overloaded");
    }

    #[test]
    fn ping_and_unknown_event_types_are_ignored() {
        let events = drive(&[
            &sse("ping", r#"{"type":"ping"}"#),
            &sse(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-opus-4-5"}}"#,
            ),
            &sse("message_stop", r#"{"type":"message_stop"}"#),
            &sse("future_event", r#"{"type":"some_future_event"}"#),
        ]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- framing -------------------------------------------------------------

    #[test]
    fn sse_block_split_across_two_chunks() {
        // The block boundary falls mid-token; the leftover buffer must join them.
        let full = sse(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"joined"}}"#,
        );
        let (head, tail) = full.split_at(30);

        let events = drive(&[head, tail]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "joined");
    }

    #[test]
    fn a_partial_trailing_block_is_not_decoded_until_complete() {
        let mut buf = String::from("event: content_block_delta\ndata: {\"type\":\"cont");
        let blocks = split_sse_blocks(&mut buf);

        assert!(blocks.is_empty());
        assert_eq!(buf, "event: content_block_delta\ndata: {\"type\":\"cont");
    }

    #[test]
    fn multiple_blocks_in_one_chunk_are_all_decoded() {
        let chunk = format!(
            "{}{}",
            sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            ),
            sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
            ),
        );

        let events = drive(&[&chunk]);

        assert_eq!(events.len(), 2);
        assert_eq!(as_text(&events[0]), "a");
        assert_eq!(as_text(&events[1]), "b");
    }

    #[test]
    fn event_type_falls_back_to_the_event_line_when_data_omits_it() {
        let events = drive(&[&sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"text_delta","text":"fallback"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "fallback");
    }

    #[test]
    fn done_sentinel_and_empty_data_lines_are_skipped() {
        let events = drive(&["data: [DONE]\n\n", "event: ping\n\n"]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    #[test]
    fn malformed_data_line_is_a_hard_error() {
        let mut tools = HashMap::new();
        let err = decode_block("event: ping\ndata: {not json}", &mut tools)
            .expect_err("malformed JSON should abort the stream");

        assert!(matches!(err, ApiError::Serialization(_)), "got {err:?}");
    }

    // -- request body --------------------------------------------------------

    fn cfg() -> CompletionConfig {
        CompletionConfig::new("claude-opus-4-5", 1024)
    }

    #[test]
    fn build_request_body_maps_tool_role_to_user_tool_result_block() {
        let body = AnthropicClient::build_request_body(
            &[ChatMessage::tool_result("toolu_1", "file written")],
            &[],
            &cfg(),
        );

        assert_eq!(
            body["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "file written",
                }]
            })
        );
    }

    #[test]
    fn build_request_body_emits_tool_use_blocks_for_assistant_tool_calls() {
        let msg = ChatMessage::assistant_with_tool_calls(
            "checking",
            vec![ToolCall {
                id: "toolu_1".into(),
                name: "get_story".into(),
                input: json!({ "id": "s1" }),
            }],
        );

        let body = AnthropicClient::build_request_body(&[msg], &[], &cfg());

        assert_eq!(
            body["messages"][0],
            json!({
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "checking" },
                    { "type": "tool_use", "id": "toolu_1", "name": "get_story", "input": { "id": "s1" } },
                ]
            })
        );
    }

    #[test]
    fn build_request_body_prefers_config_system_prompt_over_system_message() {
        let mut config = cfg();
        config.system_prompt = Some("from config".into());

        let body = AnthropicClient::build_request_body(
            &[
                ChatMessage::system("from message"),
                ChatMessage::user("hello"),
            ],
            &[],
            &config,
        );

        assert_eq!(body["system"], json!("from config"));
        // The system message must not leak into the messages array.
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn build_request_body_falls_back_to_the_system_message() {
        let body = AnthropicClient::build_request_body(
            &[
                ChatMessage::system("from message"),
                ChatMessage::user("hello"),
            ],
            &[],
            &cfg(),
        );

        assert_eq!(body["system"], json!("from message"));
    }

    #[test]
    fn build_request_body_omits_tools_key_when_empty() {
        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &[], &cfg());

        assert!(body.get("tools").is_none(), "body was {body}");
        assert!(body.get("system").is_none(), "body was {body}");
        assert!(body.get("temperature").is_none(), "body was {body}");
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["max_tokens"], json!(1024));
    }

    #[test]
    fn build_request_body_includes_tool_definitions_as_input_schema() {
        let tools = vec![ToolDefinition {
            name: "get_story".into(),
            description: "Fetch a story".into(),
            input_schema: json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
        }];

        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &tools, &cfg());

        assert_eq!(
            body["tools"][0],
            json!({
                "name": "get_story",
                "description": "Fetch a story",
                "input_schema": { "type": "object", "properties": { "id": { "type": "string" } } },
            })
        );
    }
}
