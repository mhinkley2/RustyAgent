// Cross-provider contract tests.
//
// Every provider speaks a different wire format, but the runtime is written
// against one normalized `StreamEvent` sequence. These tests feed each decoder
// the same *logical* turn — a sentence of prose, then a `file_write` tool call,
// then a terminal event — and assert they all normalize to the same shape.
//
// Provider-specific detail that legitimately differs (stop-reason vocabulary,
// tool-call id assignment) is excluded from the contract; everything the
// runtime actually branches on is included.

use serde_json::json;
use std::collections::HashMap;

use crate::types::StreamEvent;
use crate::{anthropic, ollama, openai_sse};

/// The shape the runtime consumes, with provider-specific detail normalized out.
#[derive(Debug, PartialEq)]
enum Shape {
    Text(String),
    Tool { name: String, input: serde_json::Value },
    Done,
    Error(String),
}

fn shapes(events: &[StreamEvent]) -> Vec<Shape> {
    events
        .iter()
        .map(|e| match e {
            StreamEvent::TextDelta(t) => Shape::Text(t.clone()),
            StreamEvent::ToolCallDelta(c) => Shape::Tool {
                name: c.name.clone(),
                input: c.input.clone(),
            },
            StreamEvent::Done { .. } => Shape::Done,
            StreamEvent::Error(m) => Shape::Error(m.clone()),
        })
        .collect()
}

/// The one turn every provider fixture below encodes.
fn expected_turn() -> Vec<Shape> {
    vec![
        Shape::Text("Working.".to_string()),
        Shape::Tool {
            name: "file_write".to_string(),
            input: json!({ "path": "a.txt" }),
        },
        Shape::Done,
    ]
}

// -- per-provider drivers ---------------------------------------------------

fn drive_anthropic(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut tools: HashMap<u32, anthropic::ToolAccum> = HashMap::new();
    let mut leftover = String::new();
    let mut out = Vec::new();
    for chunk in chunks {
        leftover.push_str(chunk);
        for block in anthropic::split_sse_blocks(&mut leftover) {
            out.extend(anthropic::decode_block(&block, &mut tools).expect("anthropic decode"));
        }
    }
    out
}

fn drive_openai(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut decoder = openai_sse::OpenAiSseDecoder::new();
    let mut leftover = String::new();
    let mut out = Vec::new();
    for chunk in chunks {
        leftover.push_str(chunk);
        for line in openai_sse::split_lines(&mut leftover) {
            let decoded = decoder.decode_line(&line).expect("openai decode");
            out.extend(decoded.events);
            if decoded.terminal {
                return out;
            }
        }
    }
    out
}

fn drive_ollama(chunks: &[&str]) -> Vec<StreamEvent> {
    let mut leftover = String::new();
    let mut seq = 0;
    let mut out = Vec::new();
    for chunk in chunks {
        leftover.push_str(chunk);
        for line in ollama::split_lines(&mut leftover) {
            let (events, done) = ollama::decode_line(&line, &mut seq).expect("ollama decode");
            out.extend(events);
            if done {
                return out;
            }
        }
    }
    out
}

// -- fixtures: the same turn in four wire formats ---------------------------

const ANTHROPIC_TURN: &[&str] = &[
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Working.\"}}\n\n",
    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"file_write\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a.txt\\\"}\"}}\n\n",
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
];

/// DeepSeek and OpenRouter share this format (and, since the refactor, the
/// decoder itself).
const OPENAI_TURN: &[&str] = &[
    "data: {\"choices\":[{\"delta\":{\"content\":\"Working.\"},\"finish_reason\":null}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_write\",\"arguments\":\"\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n",
];

const OLLAMA_TURN: &[&str] = &[
    "{\"message\":{\"role\":\"assistant\",\"content\":\"Working.\"},\"done\":false}\n",
    "{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"file_write\",\"arguments\":{\"path\":\"a.txt\"}}}]},\"done\":false}\n",
    "{\"done\":true,\"done_reason\":\"stop\"}\n",
];

// -- the contract ------------------------------------------------------------

#[test]
fn anthropic_normalizes_the_reference_turn() {
    assert_eq!(shapes(&drive_anthropic(ANTHROPIC_TURN)), expected_turn());
}

#[test]
fn openai_compatible_providers_normalize_the_reference_turn() {
    assert_eq!(shapes(&drive_openai(OPENAI_TURN)), expected_turn());
}

#[test]
fn ollama_normalizes_the_reference_turn() {
    assert_eq!(shapes(&drive_ollama(OLLAMA_TURN)), expected_turn());
}

#[test]
fn all_providers_agree_on_the_reference_turn() {
    let anthropic = shapes(&drive_anthropic(ANTHROPIC_TURN));
    let openai = shapes(&drive_openai(OPENAI_TURN));
    let ollama = shapes(&drive_ollama(OLLAMA_TURN));

    assert_eq!(anthropic, openai, "anthropic and openai-compatible diverge");
    assert_eq!(openai, ollama, "openai-compatible and ollama diverge");
}

#[test]
fn every_provider_assigns_a_non_empty_tool_call_id() {
    // The runtime correlates tool results back by id, so a blank id silently
    // breaks the round trip regardless of which provider produced it.
    for (provider, events) in [
        ("anthropic", drive_anthropic(ANTHROPIC_TURN)),
        ("openai", drive_openai(OPENAI_TURN)),
        ("ollama", drive_ollama(OLLAMA_TURN)),
    ] {
        let call = events
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolCallDelta(c) => Some(c),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{provider} produced no tool call"));

        assert!(!call.id.is_empty(), "{provider} produced a blank tool id");
    }
}

#[test]
fn every_provider_terminates_the_turn_exactly_once() {
    for (provider, events) in [
        ("anthropic", drive_anthropic(ANTHROPIC_TURN)),
        ("openai", drive_openai(OPENAI_TURN)),
        ("ollama", drive_ollama(OLLAMA_TURN)),
    ] {
        let done = events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Done { .. }))
            .count();

        assert_eq!(done, 1, "{provider} emitted {done} Done events");
    }
}

#[test]
fn every_provider_emits_the_tool_call_before_the_done_event() {
    // The runtime stops the turn on Done; a tool call arriving after it would
    // be dropped.
    for (provider, events) in [
        ("anthropic", drive_anthropic(ANTHROPIC_TURN)),
        ("openai", drive_openai(OPENAI_TURN)),
        ("ollama", drive_ollama(OLLAMA_TURN)),
    ] {
        let tool = events
            .iter()
            .position(|e| matches!(e, StreamEvent::ToolCallDelta(_)))
            .unwrap_or_else(|| panic!("{provider} produced no tool call"));
        let done = events
            .iter()
            .position(|e| matches!(e, StreamEvent::Done { .. }))
            .unwrap_or_else(|| panic!("{provider} produced no Done"));

        assert!(tool < done, "{provider} emitted its tool call after Done");
    }
}

#[test]
fn a_text_only_turn_agrees_across_providers() {
    let expected = vec![Shape::Text("Hello".to_string()), Shape::Done];

    let anthropic = shapes(&drive_anthropic(&[
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    ]));
    let openai = shapes(&drive_openai(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n",
        "data: [DONE]\n",
    ]));
    let ollama = shapes(&drive_ollama(&[
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"done\":false}\n",
        "{\"done\":true}\n",
    ]));

    assert_eq!(anthropic, expected);
    assert_eq!(openai, expected);
    assert_eq!(ollama, expected);
}
