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

use crate::mock::{MockResponse, DEFAULT_MOCK_USAGE};
use crate::types::{StreamEvent, Usage};
use crate::{anthropic, mock, ollama, openai_sse, provider::LlmProvider};

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
    let mut usage = None;
    let mut leftover = String::new();
    let mut out = Vec::new();
    for chunk in chunks {
        leftover.push_str(chunk);
        for block in anthropic::split_sse_blocks(&mut leftover) {
            out.extend(
                anthropic::decode_block(&block, &mut tools, &mut usage).expect("anthropic decode"),
            );
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
    out.extend(decoder.finish());
    out
}

/// Drive the mock provider end to end, the way the runtime does.
async fn drive_mock(response: MockResponse) -> Vec<StreamEvent> {
    use futures::StreamExt;

    let provider = mock::MockLlmProvider::script(vec![response]);
    let mut stream = provider
        .stream_completion(Vec::new(), Vec::new(), crate::CompletionConfig::new("m", 8))
        .await
        .expect("mock stream");

    let mut out = Vec::new();
    while let Some(event) = stream.next().await {
        out.push(event.expect("mock event"));
    }
    out
}

/// The usage reported on a turn's terminal event, if any.
fn reported_usage(events: &[StreamEvent]) -> Option<Usage> {
    events.iter().find_map(|e| match e {
        StreamEvent::Done { usage, .. } => Some(*usage),
        _ => None,
    })?
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
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":31,\"output_tokens\":1}}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Working.\"}}\n\n",
    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"file_write\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a.txt\\\"}\"}}\n\n",
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":12}}\n\n",
];

/// DeepSeek and OpenRouter share this format (and, since the refactor, the
/// decoder itself).
const OPENAI_TURN: &[&str] = &[
    "data: {\"choices\":[{\"delta\":{\"content\":\"Working.\"},\"finish_reason\":null}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_write\",\"arguments\":\"\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]}}]}\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":12}}\n",
    "data: [DONE]\n",
];

const OLLAMA_TURN: &[&str] = &[
    "{\"message\":{\"role\":\"assistant\",\"content\":\"Working.\"},\"done\":false}\n",
    "{\"message\":{\"role\":\"assistant\",\"content\":\"\",\"tool_calls\":[{\"function\":{\"name\":\"file_write\",\"arguments\":{\"path\":\"a.txt\"}}}]},\"done\":false}\n",
    "{\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":31,\"eval_count\":12}\n",
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

// -- the usage contract ------------------------------------------------------
//
// Run accounting is only as good as its worst provider: a single client that
// silently reports nothing turns a run's cost into an undercount with no
// symptom. So the contract is asserted per provider, not just for whichever one
// was implemented first.

#[test]
fn every_provider_reports_usage_on_its_reference_turn() {
    for (provider, events) in [
        ("anthropic", drive_anthropic(ANTHROPIC_TURN)),
        ("openai", drive_openai(OPENAI_TURN)),
        ("ollama", drive_ollama(OLLAMA_TURN)),
    ] {
        let usage = reported_usage(&events)
            .unwrap_or_else(|| panic!("{provider} reported no usage on a turn that carries it"));

        assert!(
            usage.total_input_tokens() > 0,
            "{provider} reported no input tokens"
        );
        assert!(usage.output_tokens > 0, "{provider} reported no output tokens");
    }
}

#[test]
fn all_providers_agree_on_the_token_counts_of_the_reference_turn() {
    // Each fixture encodes the same 31-in / 12-out turn in its own spelling.
    let anthropic = reported_usage(&drive_anthropic(ANTHROPIC_TURN)).expect("anthropic usage");
    let openai = reported_usage(&drive_openai(OPENAI_TURN)).expect("openai usage");
    let ollama = reported_usage(&drive_ollama(OLLAMA_TURN)).expect("ollama usage");

    assert_eq!(anthropic, openai, "anthropic and openai-compatible diverge");
    assert_eq!(openai, ollama, "openai-compatible and ollama diverge");
    assert_eq!(anthropic.total_input_tokens(), 31);
    assert_eq!(anthropic.output_tokens, 12);
}

#[tokio::test]
async fn the_mock_provider_reports_deterministic_usage() {
    // Deterministic across response kinds, so a runtime test can assert on a
    // run total without caring which shape of turn produced it.
    for response in [
        MockResponse::text("hi"),
        MockResponse::text_chunks(["a", "b"]),
        MockResponse::tool_call("c1", "noop", serde_json::json!({})),
        MockResponse::Error("recoverable".into()),
    ] {
        let events = drive_mock(response).await;

        assert_eq!(
            reported_usage(&events),
            Some(DEFAULT_MOCK_USAGE),
            "every mock turn reports the same counts"
        );
    }
}

#[tokio::test]
async fn the_mock_provider_can_stand_in_for_one_that_measures_nothing() {
    use futures::StreamExt;

    let provider = mock::MockLlmProvider::script(vec![MockResponse::text("hi")]).without_usage();
    let mut stream = provider
        .stream_completion(Vec::new(), Vec::new(), crate::CompletionConfig::new("m", 8))
        .await
        .expect("mock stream");

    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("mock event"));
    }

    assert_eq!(reported_usage(&events), None);
}

#[test]
fn no_provider_reports_usage_it_was_not_given() {
    // The same turns with every usage field stripped: a decoder that
    // manufactures zeros would be indistinguishable from a real zero-token
    // call, and would price an unmeasured run as free.
    let anthropic = drive_anthropic(&[
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
    ]);
    let openai = drive_openai(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n",
        "data: [DONE]\n",
    ]);
    let ollama = drive_ollama(&[
        "{\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"done\":false}\n",
        "{\"done\":true}\n",
    ]);

    for (provider, events) in [
        ("anthropic", anthropic),
        ("openai", openai),
        ("ollama", ollama),
    ] {
        assert_eq!(
            reported_usage(&events),
            None,
            "{provider} invented usage the wire never carried"
        );
    }
}
