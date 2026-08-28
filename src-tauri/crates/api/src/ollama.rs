use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, MessageRole, StreamEvent, ToolCall, ToolDefinition},
};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";

pub struct OllamaClient {
    client: Client,
    base_url: String,
}

impl OllamaClient {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }
}

impl Default for OllamaClient {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// NDJSON decoding
//
// Ollama streams newline-delimited JSON rather than SSE. Decoding is split out
// as a pure function so the wire format can be tested without a live server.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessage>,
    done: bool,
    done_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[allow(dead_code)]
    role: String,
    #[serde(default)]
    content: String,
    /// Ollama returns tool calls whole (not fragmented), so no accumulator is
    /// needed here — unlike the OpenAI-compatible providers.
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaToolFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaToolFunction {
    name: String,
    /// Ollama sends arguments as a JSON object, not an encoded string.
    #[serde(default)]
    arguments: Value,
}

/// Map one `ChatMessage` onto Ollama's `/api/chat` message shape.
///
/// Tool results keep the `tool` role rather than collapsing to `user`, so the
/// model can tell a tool's output apart from something the user typed.
fn build_api_message(m: &ChatMessage) -> Value {
    let role = match m.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };

    let mut msg = json!({ "role": role, "content": m.content });

    if let Some(calls) = &m.tool_calls {
        msg["tool_calls"] = json!(calls
            .iter()
            .map(|c| json!({
                "function": { "name": c.name, "arguments": c.input },
            }))
            .collect::<Vec<_>>());
    }

    msg
}

/// Drain every complete line from `buf`, leaving a trailing partial line behind.
pub(crate) fn split_lines(buf: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buf.find('\n') {
        lines.push(buf[..pos].trim().to_string());
        buf.drain(..pos + 1);
    }
    lines
}

/// Decode one NDJSON line into events, plus whether the turn is finished.
pub(crate) fn decode_line(line: &str, call_seq: &mut u32) -> Result<(Vec<StreamEvent>, bool), ApiError> {
    if line.trim().is_empty() {
        return Ok((Vec::new(), false));
    }

    let parsed: OllamaStreamChunk = serde_json::from_str(line)?;
    let mut out = Vec::new();

    if let Some(msg) = parsed.message {
        if !msg.content.is_empty() {
            out.push(StreamEvent::TextDelta(msg.content));
        }
        for call in msg.tool_calls {
            // Ollama does not assign tool-call ids; synthesise a stable one so
            // results can be correlated back to the request.
            *call_seq += 1;
            out.push(StreamEvent::ToolCallDelta(ToolCall {
                id: format!("ollama_call_{call_seq}"),
                name: call.function.name,
                input: if call.function.arguments.is_null() {
                    json!({})
                } else {
                    call.function.arguments
                },
            }));
        }
    }

    if parsed.done {
        let reason = parsed.done_reason.unwrap_or_else(|| "stop".to_string());
        out.push(StreamEvent::Done { stop_reason: reason });
        return Ok((out, true));
    }

    Ok((out, false))
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

#[async_trait]
impl LlmProvider for OllamaClient {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        let api_messages: Vec<Value> = messages.iter().map(build_api_message).collect();

        let mut body = json!({
            "model": config.model,
            "messages": api_messages,
            "stream": true,
            "options": {
                "num_predict": config.max_tokens,
            }
        });

        if let Some(temp) = config.temperature {
            body["options"]["temperature"] = json!(temp);
        }

        // Ollama supports tools via the same OpenAI-compatible format.
        if !tools.is_empty() {
            let api_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(api_tools);
        }

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(ApiError::Http { status, body: body_text });
        }

        let mut byte_stream = response.bytes_stream();
        let stream = try_stream! {
            let mut leftover = String::new();
            let mut call_seq: u32 = 0;

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                for line in split_lines(&mut leftover) {
                    let (events, done) = decode_line(&line, &mut call_seq)?;
                    for event in events {
                        yield event;
                    }
                    if done {
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(vec![]);
        }

        let body: OllamaTagsResponse = response.json().await?;
        Ok(body.models.into_iter().map(|m| m.name).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(chunks: &[&str]) -> Vec<StreamEvent> {
        let mut leftover = String::new();
        let mut call_seq = 0;
        let mut out = Vec::new();
        for chunk in chunks {
            leftover.push_str(chunk);
            for line in split_lines(&mut leftover) {
                let (events, done) = decode_line(&line, &mut call_seq).expect("decode failed");
                out.extend(events);
                if done {
                    return out;
                }
            }
        }
        out
    }

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

    #[test]
    fn content_deltas_are_yielded_in_order() {
        let events = drive(&[
            r#"{"message":{"role":"assistant","content":"Hel"},"done":false}"#,
            "\n",
            r#"{"message":{"role":"assistant","content":"lo"},"done":false}"#,
            "\n",
        ]);

        assert_eq!(events.len(), 2);
        let joined: String = events.iter().map(as_text).collect();
        assert_eq!(joined, "Hello");
    }

    #[test]
    fn empty_content_is_skipped() {
        let events = drive(&[
            "{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":false}\n",
        ]);

        assert!(events.is_empty(), "got {events:?}");
    }

    #[test]
    fn tool_calls_in_message_are_yielded() {
        // Regression: `tool_calls` was never deserialized, so an Ollama agent
        // could not invoke a tool even though the request advertised them.
        let events = drive(&[
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"file_write","arguments":{"path":"a.txt","content":"hi"}}}]},"done":false}"#,
            "\n",
        ]);

        assert_eq!(events.len(), 1, "got {events:?}");
        let call = as_tool(&events[0]);
        assert_eq!(call.name, "file_write");
        assert_eq!(call.input, json!({ "path": "a.txt", "content": "hi" }));
        assert!(!call.id.is_empty(), "a tool call needs a correlatable id");
    }

    #[test]
    fn concurrent_tool_calls_get_distinct_ids() {
        let events = drive(&[
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"alpha","arguments":{}}},{"function":{"name":"beta","arguments":{}}}]},"done":false}"#,
            "\n",
        ]);

        assert_eq!(events.len(), 2);
        assert_ne!(as_tool(&events[0]).id, as_tool(&events[1]).id);
    }

    #[test]
    fn a_tool_call_without_arguments_yields_an_empty_object() {
        let events = drive(&[
            r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"list_stories"}}]},"done":false}"#,
            "\n",
        ]);

        assert_eq!(as_tool(&events[0]).input, json!({}));
    }

    #[test]
    fn text_and_tool_call_in_one_chunk_both_survive() {
        let events = drive(&[
            r#"{"message":{"role":"assistant","content":"Checking.","tool_calls":[{"function":{"name":"get_story","arguments":{"id":"s1"}}}]},"done":false}"#,
            "\n",
        ]);

        assert_eq!(events.len(), 2);
        assert_eq!(as_text(&events[0]), "Checking.");
        assert_eq!(as_tool(&events[1]).name, "get_story");
    }

    #[test]
    fn done_true_terminates_the_stream_with_its_reason() {
        let events = drive(&[
            "{\"message\":{\"role\":\"assistant\",\"content\":\"a\"},\"done\":false}\n",
            "{\"done\":true,\"done_reason\":\"length\"}\n",
            "{\"message\":{\"role\":\"assistant\",\"content\":\"never\"},\"done\":false}\n",
        ]);

        assert_eq!(events.len(), 2, "decoding must stop at done:true");
        assert_eq!(as_done(&events[1]), "length");
    }

    #[test]
    fn done_without_a_reason_defaults_to_stop() {
        let events = drive(&["{\"done\":true}\n"]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "stop");
    }

    #[test]
    fn a_line_split_across_two_chunks_is_joined() {
        let events = drive(&[
            "{\"message\":{\"role\":\"assistant\",\"cont",
            "ent\":\"joined\"},\"done\":false}\n",
        ]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "joined");
    }

    #[test]
    fn blank_lines_are_ignored() {
        let events = drive(&["\n", "   \n"]);
        assert!(events.is_empty(), "got {events:?}");
    }

    #[test]
    fn malformed_line_is_a_hard_error() {
        let mut seq = 0;
        let err = decode_line("{not json}", &mut seq)
            .expect_err("malformed JSON should abort the stream");

        assert!(matches!(err, ApiError::Serialization(_)), "got {err:?}");
    }

    // -- request body --------------------------------------------------------

    #[test]
    fn tool_results_keep_the_tool_role() {
        // Regression: `Tool` collapsed to `user`, so the model could not tell a
        // tool's output apart from something the user typed.
        let msg = build_api_message(&ChatMessage::tool_result("call_1", "file written"));

        assert_eq!(msg["role"], json!("tool"));
        assert_eq!(msg["content"], json!("file written"));
    }

    #[test]
    fn assistant_tool_calls_are_serialized_back() {
        let msg = build_api_message(&ChatMessage::assistant_with_tool_calls(
            "checking",
            vec![ToolCall {
                id: "call_1".into(),
                name: "get_story".into(),
                input: json!({ "id": "s1" }),
            }],
        ));

        assert_eq!(msg["role"], json!("assistant"));
        assert_eq!(
            msg["tool_calls"],
            json!([{ "function": { "name": "get_story", "arguments": { "id": "s1" } } }])
        );
    }

    #[test]
    fn plain_roles_map_directly_and_carry_no_tool_calls_key() {
        for (msg, expected) in [
            (ChatMessage::system("s"), "system"),
            (ChatMessage::user("u"), "user"),
            (ChatMessage::assistant("a"), "assistant"),
        ] {
            let built = build_api_message(&msg);
            assert_eq!(built["role"], json!(expected));
            assert!(built.get("tool_calls").is_none(), "built was {built}");
        }
    }
}
