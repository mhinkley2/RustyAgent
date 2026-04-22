use std::pin::Pin;
use futures::Stream;
use async_trait::async_trait;
use crate::{ApiError, ChatMessage, CompletionConfig, StreamEvent, ToolDefinition};

pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ApiError>> + Send>>;

/// Core abstraction for all LLM backends.
///
/// Each implementation drives streaming SSE/chunked-transfer from a specific
/// provider and normalises the events into [`StreamEvent`].
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stream a chat completion.
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError>;

    /// List available models for this provider.
    /// Returns a best-effort list; errors are non-fatal.
    async fn list_models(&self) -> Result<Vec<String>, ApiError>;
}
