use std::sync::{Arc, Mutex};
use async_stream::try_stream;
use async_trait::async_trait;

use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, StreamEvent, ToolCall, ToolDefinition, Usage},
};

/// Token usage every scripted response reports unless overridden.
///
/// Fixed, and deliberately made of four distinct non-round numbers, so a test
/// asserting on a total can tell which field a wrong sum came from — and so a
/// multi-call run's total is unmistakably a sum rather than the last call's
/// figures.
pub const DEFAULT_MOCK_USAGE: Usage = Usage {
    input_tokens: 100,
    output_tokens: 20,
    cache_read_input_tokens: 7,
    cache_creation_input_tokens: 3,
};

/// A scripted response for use with [`MockLlmProvider`].
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// Emit a text message as a single delta and finish.
    Text(String),
    /// Emit text as several deltas, the way a real provider streams it, then
    /// finish. Use this to exercise a consumer's accumulation logic.
    TextChunks(Vec<String>),
    /// Emit a tool call and finish with stop_reason "tool_use".
    ToolCall { id: String, name: String, input: serde_json::Value },
    /// Emit several tool calls in one turn, then finish with "tool_use".
    ToolCalls(Vec<ToolCall>),
    /// Emit a recoverable `StreamEvent::Error` mid-stream. The stream still
    /// ends normally — this is not a transport failure.
    Error(String),
    /// Fail the `stream_completion` call itself, before any event is produced.
    ///
    /// `ApiError::Provider` — opaque, and so never retried.
    ProviderError(String),
    /// Fail the call with a rate limit carrying the provider's own backoff.
    RateLimited { retry_after_secs: u64 },
    /// Fail the call with an HTTP status, for exercising the 5xx / 4xx split.
    HttpFailure { status: u16 },
    /// Fail the call as if the connection died before the stream opened.
    StreamEnded,
}

impl MockResponse {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn text_chunks<I, S>(chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::TextChunks(chunks.into_iter().map(Into::into).collect())
    }

    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::ToolCall { id: id.into(), name: name.into(), input }
    }
}

/// One recorded `stream_completion` invocation.
#[derive(Debug, Clone)]
pub struct RecordedCall {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub config: CompletionConfig,
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
    calls: Arc<Mutex<Vec<RecordedCall>>>,
    available_models: Vec<String>,
    usage: Option<Usage>,
}

impl MockLlmProvider {
    pub fn script(responses: Vec<MockResponse>) -> Self {
        Self {
            queue: Arc::new(Mutex::new(responses)),
            calls: Arc::new(Mutex::new(Vec::new())),
            available_models: vec!["mock-model".to_string()],
            usage: Some(DEFAULT_MOCK_USAGE),
        }
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.available_models = models;
        self
    }

    /// Report `usage` on every scripted response instead of
    /// [`DEFAULT_MOCK_USAGE`].
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Report no usage at all, standing in for a provider or an endpoint that
    /// does not measure tokens.
    pub fn without_usage(mut self) -> Self {
        self.usage = None;
        self
    }

    /// Every `stream_completion` invocation so far, in order.
    ///
    /// Lets a test assert what was actually sent to the provider — the
    /// resolved system prompt, the permission-filtered tool list, and the
    /// tool-result messages fed back on the second turn.
    pub fn recorded_calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().expect("mock calls poisoned").clone()
    }

    /// How many times `stream_completion` has been invoked.
    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("mock calls poisoned").len()
    }

    /// Scripted responses not yet consumed.
    pub fn remaining(&self) -> usize {
        self.queue.lock().expect("mock queue poisoned").len()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        self.calls
            .lock()
            .expect("mock calls poisoned")
            .push(RecordedCall {
                messages,
                tools,
                config,
            });

        let response = {
            let mut queue = self.queue.lock().expect("mock queue poisoned");
            if queue.is_empty() {
                return Err(ApiError::Provider(
                    "MockLlmProvider: no more scripted responses".to_string(),
                ));
            }
            queue.remove(0)
        };

        // Failures of the call itself, before any event is produced. Each maps
        // to the `ApiError` variant a real provider would raise, so a consumer
        // testing against these is testing against the same classification.
        match response {
            MockResponse::ProviderError(msg) => return Err(ApiError::Provider(msg)),
            MockResponse::RateLimited { retry_after_secs } => {
                return Err(ApiError::RateLimited { retry_after_secs })
            }
            MockResponse::HttpFailure { status } => {
                return Err(ApiError::Http { status, body: format!("mock HTTP {status}") })
            }
            MockResponse::StreamEnded => return Err(ApiError::StreamEnded),
            _ => {}
        }

        let usage = self.usage;
        let stream = try_stream! {
            match response {
                MockResponse::Text(text) => {
                    yield StreamEvent::TextDelta(text);
                    yield StreamEvent::Done { stop_reason: "end_turn".to_string(), usage };
                }
                MockResponse::TextChunks(chunks) => {
                    for chunk in chunks {
                        yield StreamEvent::TextDelta(chunk);
                    }
                    yield StreamEvent::Done { stop_reason: "end_turn".to_string(), usage };
                }
                MockResponse::ToolCall { id, name, input } => {
                    yield StreamEvent::ToolCallDelta(ToolCall { id, name, input });
                    yield StreamEvent::Done { stop_reason: "tool_use".to_string(), usage };
                }
                MockResponse::ToolCalls(calls) => {
                    for call in calls {
                        yield StreamEvent::ToolCallDelta(call);
                    }
                    yield StreamEvent::Done { stop_reason: "tool_use".to_string(), usage };
                }
                MockResponse::Error(msg) => {
                    yield StreamEvent::Error(msg);
                    yield StreamEvent::Done { stop_reason: "error".to_string(), usage };
                }
                // Handled above, before the stream is built.
                // Every call-failure variant returned above, before the
                // stream was built.
                MockResponse::ProviderError(_)
                | MockResponse::RateLimited { .. }
                | MockResponse::HttpFailure { .. }
                | MockResponse::StreamEnded => unreachable!(),
            }
        };

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        Ok(self.available_models.clone())
    }
}
