use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    /// Plain text content.
    pub content: String,
    /// Populated when role == Tool; the tool call this is responding to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Populated when role == Assistant and the LLM requested tool calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: MessageRole::System, content: content.into(), tool_call_id: None, tool_calls: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: MessageRole::User, content: content.into(), tool_call_id: None, tool_calls: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: MessageRole::Assistant, content: content.into(), tool_call_id: None, tool_calls: None }
    }
    pub fn assistant_with_tool_calls(text: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: text.into(),
            tool_call_id: None,
            tool_calls: Some(calls),
        }
    }
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the input parameters.
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON-encoded tool input.
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

// ---------------------------------------------------------------------------
// Content blocks (for multi-part assistant messages)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
}

// ---------------------------------------------------------------------------
// Completion config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionConfig {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

impl CompletionConfig {
    pub fn new(model: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            model: model.into(),
            max_tokens,
            temperature: None,
            system_prompt: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Token usage
// ---------------------------------------------------------------------------

/// Token counts for one provider call.
///
/// The field semantics are Anthropic's, and every provider client normalises
/// onto them: `input_tokens` counts only the input billed at the full rate,
/// with cache reads and cache writes reported *separately*. OpenAI-compatible
/// providers report cached tokens inside `prompt_tokens`, so those clients
/// subtract before filling this in — otherwise cached input would be counted
/// twice and priced at the uncached rate.
///
/// Providers that report nothing leave the whole struct absent
/// (`StreamEvent::Done { usage: None }`) rather than reporting zeros, so a
/// caller can tell "no tokens" apart from "not measured".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self { input_tokens, output_tokens, ..Self::default() }
    }

    /// Every input token the provider read on this call, cached or not.
    ///
    /// This — not `input_tokens` — is the size of the context that was sent,
    /// and so the number a context budget has to be measured against.
    pub fn total_input_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_input_tokens)
            .saturating_add(self.cache_creation_input_tokens)
    }

    /// Every token this call touched, in either direction.
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens().saturating_add(self.output_tokens)
    }

    pub fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

impl std::ops::Add for Usage {
    type Output = Self;

    /// Sums two calls' usage. Saturating, because a run's total must never
    /// wrap into a nonsense number — a bad estimate beats a negative one.
    fn add(self, rhs: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(rhs.input_tokens),
            output_tokens: self.output_tokens.saturating_add(rhs.output_tokens),
            cache_read_input_tokens: self
                .cache_read_input_tokens
                .saturating_add(rhs.cache_read_input_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_add(rhs.cache_creation_input_tokens),
        }
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

// ---------------------------------------------------------------------------
// Streaming events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Partial text token.
    TextDelta(String),
    /// The assistant wants to call a tool.
    ToolCallDelta(ToolCall),
    /// The full run is complete; stop_reason is provider-specific ("end_turn", "tool_use", etc.)
    ///
    /// `usage` is `None` when the provider reported no token counts for the
    /// call — an older endpoint, a stream cut short, or a fixture that predates
    /// usage reporting. Consumers must not read that as zero tokens.
    Done { stop_reason: String, usage: Option<Usage> },
    /// A recoverable error mid-stream.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_usage_sums_every_counter() {
        let a = Usage { input_tokens: 1, output_tokens: 2, cache_read_input_tokens: 3, cache_creation_input_tokens: 4 };
        let b = Usage { input_tokens: 10, output_tokens: 20, cache_read_input_tokens: 30, cache_creation_input_tokens: 40 };

        assert_eq!(
            a + b,
            Usage { input_tokens: 11, output_tokens: 22, cache_read_input_tokens: 33, cache_creation_input_tokens: 44 }
        );
    }

    #[test]
    fn add_assign_accumulates_across_calls() {
        let mut total = Usage::default();
        for _ in 0..3 {
            total += Usage::new(5, 1);
        }

        assert_eq!(total.input_tokens, 15);
        assert_eq!(total.output_tokens, 3);
    }

    #[test]
    fn summing_saturates_rather_than_wrapping() {
        // A run's total must never come back negative-looking because a
        // provider reported something absurd.
        let huge = Usage { input_tokens: u64::MAX, ..Usage::default() };

        assert_eq!((huge + huge).input_tokens, u64::MAX);
    }

    #[test]
    fn total_input_counts_cached_tokens_too() {
        // The context that was sent is everything the provider read, whether or
        // not it was billed at the full rate — that is what a budget is
        // measured against.
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 9,
            cache_read_input_tokens: 800,
            cache_creation_input_tokens: 100,
        };

        assert_eq!(usage.total_input_tokens(), 1000);
        assert_eq!(usage.total_tokens(), 1009);
    }

    #[test]
    fn a_default_usage_is_zero_and_says_so() {
        assert!(Usage::default().is_zero());
        assert!(!Usage::new(0, 1).is_zero());
    }
}
