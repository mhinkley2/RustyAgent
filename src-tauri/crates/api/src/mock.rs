use std::sync::{Arc, Mutex};
use async_stream::try_stream;
use async_trait::async_trait;

use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, StreamEvent, ToolCall, ToolDefinition},
};

/// A scripted response for use with [`MockLlmProvider`].
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Emit a text message and finish.
    Text(String),
    /// Emit a tool call and finish with stop_reason "tool_use".
    ToolCall { id: String, name: String, input: serde_json::Value },
    /// Emit an error mid-stream.
    Error(String),
}

impl MockResponse {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ToolCall { id: id.into(), name: name.into(), input }
    }
}

/// A deterministic [`LlmProvider`] for unit testing.
///
/// Call [`MockLlmProvider::script`] with a list of responses. Each call to
/// `stream_completion` consumes the next response in order.
///
/// # Example
/// ```rust,no_run
/// use api::MockLlmProvider;
/// use api::mock::MockResponse;
/// let mock = MockLlmProvider::script(vec![
///     MockResponse::text("Hello from mock"),
///     MockResponse::tool_call("call_1", "get_story", serde_json::json!({"id": "abc"})),
/// ]);
/// ```
pub struct MockLlmProvider {
    queue: Arc<Mutex<Vec<MockResponse>>>,
    available_models: Vec<String>,
}

impl MockLlmProvider {
    pub fn script(responses: Vec<MockResponse>) -> Self {
        Self {
            queue: Arc::new(Mutex::new(responses)),
            available_models: vec!["mock-model".to_string()],
        }
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.available_models = models;
        self
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn stream_completion(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Vec<ToolDefinition>,
        _config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        let response = {
            let mut queue = self.queue.lock().expect("mock queue poisoned");
            if queue.is_empty() {
                return Err(ApiError::Provider(
                    "MockLlmProvider: no more scripted responses".to_string(),
                ));
            }
            queue.remove(0)
        };

        let stream = try_stream! {
            match response {
                MockResponse::Text(text) => {
                    yield StreamEvent::TextDelta(text);
                    yield StreamEvent::Done { stop_reason: "end_turn".to_string() };
                }
                MockResponse::ToolCall { id, name, input } => {
                    yield StreamEvent::ToolCallDelta(ToolCall { id, name, input });
                    yield StreamEvent::Done { stop_reason: "tool_use".to_string() };
                }
                MockResponse::Error(msg) => {
                    yield StreamEvent::Error(msg);
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        Ok(self.available_models.clone())
    }
}
