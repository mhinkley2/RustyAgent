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
    /// Which provider this is, in the spelling `agent_profiles.provider` uses.
    ///
    /// Cost accounting needs to tell "this model is not in the price table"
    /// from "this provider charges nothing", and those look identical from the
    /// model id alone — a local `llama3:8b` is as absent from `PRICES` as a
    /// DeepSeek model is. The provider is the only thing that separates them,
    /// and asking the object that made the call beats threading a string
    /// through every construction site.
    fn provider_id(&self) -> &'static str;

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
