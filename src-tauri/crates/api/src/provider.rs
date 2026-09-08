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

#[cfg(test)]
mod tests {
    use super::LlmProvider;

    /// The real clients report the spellings the rest of the app matches on.
    ///
    /// Everything about run cost hangs on these four strings. They have to
    /// equal what `agent_profiles.provider` stores — the `match` arms in
    /// `commands`, `pipeline`, `scheduler` and `src/lib.rs` that decide which
    /// client to build — and `pricing::is_free_provider` has to recognise
    /// `ollama` among them.
    ///
    /// Without this the mock is the only thing asserting a provider id, and it
    /// asserts a literal against a literal. Misspelling `ollama` in
    /// `ollama.rs` made every local run report "cost unknown" forever and no
    /// test anywhere noticed.
    #[test]
    fn every_client_reports_the_provider_id_the_app_builds_it_from() {
        let clients: Vec<(Box<dyn LlmProvider>, &str)> = vec![
            (Box::new(crate::anthropic::AnthropicClient::new("k")), "anthropic"),
            (Box::new(crate::openrouter::OpenRouterClient::new("k")), "openrouter"),
            (Box::new(crate::deepseek::DeepSeekClient::new("k")), "deepseek"),
            (Box::new(crate::ollama::OllamaClient::new()), "ollama"),
        ];

        for (client, expected) in clients {
            assert_eq!(
                client.provider_id(),
                expected,
                "this string is what the provider match arms and the price \
                 buckets key on; changing it silently repricies every run",
            );
        }
    }

    /// The one the cost path actually branches on.
    #[test]
    fn the_local_client_is_the_one_pricing_treats_as_free() {
        let ollama = crate::ollama::OllamaClient::new();

        assert!(
            crate::pricing::is_free_provider(ollama.provider_id()),
            "OllamaClient must be recognised as free by the id it reports",
        );
    }
}
