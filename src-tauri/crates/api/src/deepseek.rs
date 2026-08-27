// DeepSeek API client — OpenAI-compatible streaming endpoint.
// Docs: https://platform.deepseek.com/api-docs

use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

use crate::{
    error::ApiError,
    openai_sse::{split_lines, OpenAiSseDecoder},
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, MessageRole, ToolDefinition},
};

const BASE_URL: &str = "https://api.deepseek.com/v1";

pub struct DeepSeekClient {
    client: Client,
    api_key: String,
}

impl DeepSeekClient {
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
        let api_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System    => "system",
                    MessageRole::User      => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool      => "tool",
                };
                let mut msg = json!({ "role": role, "content": m.content });
                if let Some(id) = &m.tool_call_id {
                    msg["tool_call_id"] = json!(id);
                }
                // Serialize tool_calls in the OpenAI wire format.
                if let Some(calls) = &m.tool_calls {
                    let wire_calls: Vec<Value> = calls.iter().map(|c| json!({
                        "id": c.id,
                        "type": "function",
                        "function": {
                            "name": c.name,
                            "arguments": c.input.to_string(),
                        }
                    })).collect();
                    msg["tool_calls"] = json!(wire_calls);
                }
                msg
            })
            .collect();

        // Inject system prompt from config if not already in messages.
        let mut msgs = api_messages;
        if let Some(sys) = &config.system_prompt {
            let has_system = msgs.iter().any(|m| m["role"] == "system");
            if !has_system {
                msgs.insert(0, json!({ "role": "system", "content": sys }));
            }
        }

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

        let mut body = json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": true,
            "messages": msgs,
        });

        if let Some(temp) = config.temperature {
            body["temperature"] = json!(temp);
        }
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        body
    }
}

#[async_trait]
impl LlmProvider for DeepSeekClient {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        let body = Self::build_request_body(&messages, &tools, &config);

        let response = self
            .client
            .post(format!("{BASE_URL}/chat/completions"))
            .header("Authorization", format!("Bearer {}", self.api_key))
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
            let mut decoder = OpenAiSseDecoder::new();
            let mut leftover = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                for line in split_lines(&mut leftover) {
                    let decoded = decoder.decode_line(&line)?;
                    for event in decoded.events {
                        yield event;
                    }
                    if decoded.terminal {
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
            .get(format!("{BASE_URL}/models"))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(vec![
                "deepseek-chat".to_string(),
                "deepseek-reasoner".to_string(),
            ]);
        }

        let json: Value = response.json().await?;
        let models = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }
}
