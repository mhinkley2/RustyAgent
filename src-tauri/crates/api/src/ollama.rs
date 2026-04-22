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
    types::{ChatMessage, CompletionConfig, MessageRole, StreamEvent, ToolDefinition},
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

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    message: Option<OllamaMessage>,
    done: bool,
    done_reason: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
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
        let api_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User | MessageRole::Tool => "user",
                    MessageRole::Assistant => "assistant",
                };
                json!({ "role": role, "content": m.content })
            })
            .collect();

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

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = leftover.find('\n') {
                    let line = leftover[..pos].trim().to_string();
                    leftover = leftover[pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let parsed: OllamaStreamChunk = serde_json::from_str(&line)
                        .map_err(|e| ApiError::Serialization(e))?;

                    if let Some(msg) = parsed.message {
                        if !msg.content.is_empty() {
                            yield StreamEvent::TextDelta(msg.content);
                        }
                    }

                    if parsed.done {
                        let reason = parsed.done_reason.unwrap_or_else(|| "stop".to_string());
                        yield StreamEvent::Done { stop_reason: reason };
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
