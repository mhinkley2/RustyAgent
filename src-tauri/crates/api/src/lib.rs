// LLM provider abstraction and clients (Anthropic, OpenRouter, Ollama).
// See RUSTYAGE-4 for implementation details.

pub mod error;
pub mod keychain;
pub mod types;
pub mod pricing;
pub mod provider;
pub mod anthropic;
pub(crate) mod openai_sse;
pub mod openrouter;
pub mod ollama;
pub mod deepseek;
pub mod mock;

#[cfg(test)]
mod contract_tests;

pub use error::ApiError;
pub use keychain::ApiKeyStore;
pub use types::{
    ChatMessage, MessageRole, ToolDefinition, ToolCall, ToolResult,
    CompletionConfig, StreamEvent, ContentBlock, Usage,
};
pub use pricing::{estimate_cost_usd, ModelPrice};
pub use provider::LlmProvider;
pub use mock::MockLlmProvider;
pub use anthropic::AnthropicClient;
pub use openrouter::OpenRouterClient;
pub use ollama::OllamaClient;
pub use deepseek::DeepSeekClient;
