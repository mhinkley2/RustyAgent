// Conservative token estimation for a request that has not been sent yet.
//
// A context budget has to be checked *before* the provider call, and the only
// figure available at that point is an estimate. No provider tokeniser is
// vendored here on purpose: shipping one per provider is a large dependency for
// a number that only has to be close enough to decide "does this still fit".
//
// The estimate therefore errs **high**. Under-counting is the dangerous
// direction — it sends an oversized request that the provider rejects, which is
// exactly the failure this exists to prevent. Over-counting only compacts
// slightly earlier than strictly necessary.
//
// Once a call completes the caller can reconcile: `Usage::total_input_tokens()`
// is the real size of what was sent, and the ratio against the estimate for the
// same message list calibrates the next one. See
// `ConversationRuntime::record_usage`.

use crate::types::{ChatMessage, ToolDefinition};

/// UTF-8 bytes assumed to make up one token.
///
/// Bytes rather than chars, and 3 rather than the usual 4, so the estimate
/// stays on the high side in both directions of the alphabet: English prose
/// runs ~3.7 ASCII bytes per token, so dividing by 3 over-counts by ~20%; CJK
/// runs close to one token per character and encodes at 3 bytes per character,
/// so dividing by 3 lands near 1:1 rather than wildly under.
pub const BYTES_PER_TOKEN: u64 = 3;

/// Flat cost of one message's role framing and content-block wrappers.
///
/// Every provider wraps a message in some envelope — Anthropic in a
/// `{"role": ..., "content": [...]}` object, tool results in a `tool_result`
/// block with an id. A long conversation of short messages is mostly envelope,
/// so ignoring it under-counts precisely where compaction matters most.
pub const MESSAGE_OVERHEAD_TOKENS: u64 = 8;

/// Flat cost of one tool definition's JSON envelope, on top of its text.
pub const TOOL_OVERHEAD_TOKENS: u64 = 8;

/// Tokens for a plain string, rounded up.
pub fn estimate_text(text: &str) -> u64 {
    // Rounding up, not down: a list of one- and two-byte messages must not
    // estimate as free.
    (text.len() as u64).div_ceil(BYTES_PER_TOKEN)
}

/// Tokens for one message, including its tool-call payloads and envelope.
pub fn estimate_message(message: &ChatMessage) -> u64 {
    let mut total = MESSAGE_OVERHEAD_TOKENS + estimate_text(&message.content);

    if let Some(id) = &message.tool_call_id {
        total += estimate_text(id);
    }

    // Tool call inputs are JSON and routinely dwarf the assistant's prose —
    // a file_write call carries the whole file body. Measuring only `content`
    // would make the largest messages in a run look like the smallest.
    for call in message.tool_calls.iter().flatten() {
        total += MESSAGE_OVERHEAD_TOKENS
            + estimate_text(&call.id)
            + estimate_text(&call.name)
            + estimate_text(&call.input.to_string());
    }

    total
}

/// Tokens for a whole message list.
pub fn estimate_messages(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(estimate_message).sum()
}

/// Tokens for one tool definition, schema included.
pub fn estimate_tool(tool: &ToolDefinition) -> u64 {
    TOOL_OVERHEAD_TOKENS
        + estimate_text(&tool.name)
        + estimate_text(&tool.description)
        + estimate_text(&tool.input_schema.to_string())
}

/// Tokens the request carries regardless of conversation length: the system
/// prompt and the tool schemas.
///
/// This is charged against the same budget as the messages — it is input the
/// provider reads — but it is not evictable, so a caller measures it apart
/// from the part it can compact.
pub fn estimate_overhead(system_prompt: Option<&str>, tools: &[ToolDefinition]) -> u64 {
    let system = system_prompt.map_or(0, |s| MESSAGE_OVERHEAD_TOKENS + estimate_text(s));
    system + tools.iter().map(estimate_tool).sum::<u64>()
}

/// Tokens for a complete request: overhead plus messages.
pub fn estimate_request(
    system_prompt: Option<&str>,
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
) -> u64 {
    estimate_overhead(system_prompt, tools) + estimate_messages(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCall;
    use serde_json::json;

    #[test]
    fn an_empty_string_costs_nothing() {
        assert_eq!(estimate_text(""), 0);
    }

    #[test]
    fn text_rounds_up_rather_than_truncating_to_zero() {
        // A one-byte string is one token, not zero — a list of tiny messages
        // must not estimate as free.
        assert_eq!(estimate_text("a"), 1);
        assert_eq!(estimate_text("abc"), 1);
        assert_eq!(estimate_text("abcd"), 2);
    }

    #[test]
    fn multibyte_text_is_measured_in_bytes_without_panicking() {
        // Three bytes per char in UTF-8, and roughly one token per char in a
        // real tokeniser — the byte rule lands near 1:1 instead of under.
        let cjk = "日本語";
        assert_eq!(cjk.len(), 9);
        assert_eq!(estimate_text(cjk), 3);
    }

    #[test]
    fn a_message_costs_more_than_its_bare_text() {
        // The role envelope is real input; ignoring it under-counts a long
        // conversation of short turns badly.
        let msg = ChatMessage::user("hi");
        assert!(estimate_message(&msg) > estimate_text("hi"));
        assert_eq!(estimate_message(&msg), MESSAGE_OVERHEAD_TOKENS + 1);
    }

    #[test]
    fn a_tool_call_payload_is_counted_not_just_the_assistant_text() {
        let big = "x".repeat(3000);
        let plain = ChatMessage::assistant("");
        let with_call = ChatMessage::assistant_with_tool_calls(
            "",
            vec![ToolCall {
                id: "c1".into(),
                name: "file_write".into(),
                input: json!({ "content": big }),
            }],
        );

        assert!(
            estimate_message(&with_call) > estimate_message(&plain) + 900,
            "the JSON input body must be measured, got {}",
            estimate_message(&with_call)
        );
    }

    #[test]
    fn a_tool_result_id_is_counted() {
        let anonymous = ChatMessage::assistant("result body");
        let correlated = ChatMessage::tool_result("call_abcdefghijkl", "result body");

        assert!(estimate_message(&correlated) > estimate_message(&anonymous));
    }

    #[test]
    fn an_empty_conversation_estimates_as_zero() {
        assert_eq!(estimate_messages(&[]), 0);
        assert_eq!(estimate_overhead(None, &[]), 0);
        assert_eq!(estimate_request(None, &[], &[]), 0);
    }

    #[test]
    fn the_system_prompt_and_tool_schemas_are_charged_to_the_request() {
        let tool = ToolDefinition {
            name: "file_read".into(),
            description: "reads a file".into(),
            input_schema: json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        };

        let bare = estimate_request(None, &[ChatMessage::user("hi")], &[]);
        let loaded = estimate_request(Some("you are an agent"), &[ChatMessage::user("hi")], &[tool]);

        assert!(loaded > bare, "{loaded} should exceed {bare}");
    }

    #[test]
    fn the_estimate_errs_high_against_a_real_tokeniser_for_english_prose() {
        // Real tokenisers land near 4 bytes/token on English prose. The
        // estimate must sit above that, never below — under-counting is what
        // sends an oversized request.
        let prose = "The quick brown fox jumps over the lazy dog. ".repeat(50);
        let realistic = prose.len() as u64 / 4;

        assert!(
            estimate_text(&prose) > realistic,
            "estimate {} must exceed a 4-byte/token reading of {realistic}",
            estimate_text(&prose)
        );
    }
}
