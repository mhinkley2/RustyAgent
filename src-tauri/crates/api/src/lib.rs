// LLM provider abstraction and clients (Anthropic, OpenRouter, Ollama).
// See RUSTYAGE-4 for implementation details.

pub mod error;
pub mod keychain;
pub mod types;
pub mod provider;
pub mod anthropic;
pub mod openrouter;
pub mod ollama;
pub mod deepseek;
pub mod mock;

pub use error::ApiError;
pub use keychain::ApiKeyStore;
pub use types::{
    ChatMessage, MessageRole, ToolDefinition, ToolCall, ToolResult,
    CompletionConfig, StreamEvent, ContentBlock,
};
pub use provider::LlmProvider;
pub use mock::MockLlmProvider;
pub use anthropic::AnthropicClient;
pub use openrouter::OpenRouterClient;
pub use ollama::OllamaClient;
pub use deepseek::DeepSeekClient;
