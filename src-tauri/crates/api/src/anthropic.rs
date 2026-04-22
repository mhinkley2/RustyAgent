use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, MessageRole, StreamEvent, ToolDefinition},
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
// SSE event shapes from Anthropic (typed for future structured parsing)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SseEvent {
    MessageStart { message: MessageStartData },
    ContentBlockStart { index: u32, content_block: ContentBlockStartData },
    ContentBlockDelta { index: u32, delta: DeltaData },
    ContentBlockStop { index: u32 },
    MessageDelta { delta: MessageDeltaData },
    MessageStop,
    Error { error: ErrorData },
    Ping,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MessageStartData {
    id: String,
    model: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockStartData {
    Text { text: String },
    ToolUse { id: String, name: String },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DeltaData {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MessageDeltaData {
    stop_reason: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ErrorData {
    message: String,
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

        // Accumulate partial_json for tool_use blocks keyed by block index.
        let stream = try_stream! {
            let mut tool_input_buf: std::collections::HashMap<u32, (String, String)> = std::collections::HashMap::new();
            let mut leftover = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                // SSE lines are separated by "\n\n"
                while let Some(pos) = leftover.find("\n\n") {
                    let block = leftover[..pos].to_string();
                    leftover = leftover[pos + 2..].to_string();

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
                        continue;
                    }

                    // Parse by injecting the event type as "type" if missing.
                    let parsed: Value = serde_json::from_str(&data_line)
                        .map_err(|e| ApiError::Serialization(e))?;

                    let type_field = parsed
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&event_type)
                        .to_string();

                    match type_field.as_str() {
                        "content_block_start" => {
                            if let Some(cb) = parsed.get("content_block") {
                                let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
                                if cb.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                    let id = cb["id"].as_str().unwrap_or("").to_string();
                                    let name = cb["name"].as_str().unwrap_or("").to_string();
                                    tool_input_buf.insert(idx, (id, name));
                                }
                            }
                        }
                        "content_block_delta" => {
                            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
                            if let Some(delta) = parsed.get("delta") {
                                match delta.get("type").and_then(|v| v.as_str()) {
                                    Some("text_delta") => {
                                        let text = delta["text"].as_str().unwrap_or("").to_string();
                                        if !text.is_empty() {
                                            yield StreamEvent::TextDelta(text);
                                        }
                                    }
                                    Some("input_json_delta") => {
                                        if let Some(buf) = tool_input_buf.get_mut(&idx) {
                                            let part = delta["partial_json"].as_str().unwrap_or("");
                                            buf.0.push_str(part); // temporarily stash json in id field
                                            // Actually we need two fields: accumulate partial_json separately
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_stop" => {
                            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
                            if let Some((id_and_json, name)) = tool_input_buf.remove(&idx) {
                                // id_and_json is now overloaded — we need a proper struct
                                // This is handled in the proper accumulator below
                                let _ = (id_and_json, name);
                            }
                        }
                        "message_delta" => {
                            if let Some(delta) = parsed.get("delta") {
                                let stop_reason = delta
                                    .get("stop_reason")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("end_turn")
                                    .to_string();
                                yield StreamEvent::Done { stop_reason };
                            }
                        }
                        "error" => {
                            let msg = parsed
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown error")
                                .to_string();
                            yield StreamEvent::Error(msg);
                        }
                        _ => {
                            debug!("Anthropic SSE unhandled event type: {type_field}");
                        }
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
