// Shared SSE decoder for OpenAI-compatible streaming endpoints (DeepSeek,
// OpenRouter).
//
// Split out from the clients as a pure state machine so the wire format can be
// tested without a network round-trip, and so both providers accumulate tool
// calls the same way. Tool arguments arrive as fragments across many chunks —
// the first carries `id` and `function.name`, the rest carry only
// `function.arguments` — so they must be concatenated before they parse.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::{
    error::ApiError,
    types::{StreamEvent, ToolCall},
};

/// Partial state for one streamed tool call, keyed by its `index`.
#[derive(Debug, Default, Clone, PartialEq)]
struct ToolAccum {
    id: String,
    name: String,
    args: String,
}

/// Result of decoding a single SSE line.
#[derive(Debug, Default)]
pub(crate) struct Decoded {
    pub events: Vec<StreamEvent>,
    /// The `[DONE]` sentinel was seen; the caller should stop reading.
    pub terminal: bool,
}

/// Drain every complete line from `buf`, leaving any trailing partial line
/// behind for the next chunk.
pub(crate) fn split_lines(buf: &mut String) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(pos) = buf.find('\n') {
        lines.push(buf[..pos].trim().to_string());
        buf.drain(..pos + 1);
    }
    lines
}

#[derive(Debug, Default)]
pub(crate) struct OpenAiSseDecoder {
    /// BTreeMap so flushing is deterministic in `index` order.
    tools: BTreeMap<u32, ToolAccum>,
}

impl OpenAiSseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode_line(&mut self, line: &str) -> Result<Decoded, ApiError> {
        let line = line.trim();

        // Non-data lines (blank separators, `:` keep-alive comments, `event:`)
        // carry nothing we need.
        let data = match line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:"))
        {
            Some(d) => d.trim(),
            None => return Ok(Decoded::default()),
        };

        let mut out = Vec::new();

        if data == "[DONE]" {
            // Some providers close without ever sending a finish_reason; flush
            // whatever tool calls are still in flight before terminating.
            self.flush_tools(&mut out);
            out.push(StreamEvent::Done {
                stop_reason: "stop".to_string(),
            });
            return Ok(Decoded {
                events: out,
                terminal: true,
            });
        }

        if data.is_empty() {
            return Ok(Decoded::default());
        }

        let parsed: Value = serde_json::from_str(data)?;

        if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
            for choice in choices {
                let delta = &choice["delta"];

                if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        out.push(StreamEvent::TextDelta(text.to_string()));
                    }
                }

                if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                    for tc in tool_calls {
                        self.accumulate(tc);
                    }
                }

                // `as_str()` on a JSON null yields None, so a `"finish_reason":
                // null` chunk falls through here without terminating.
                if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                    if !reason.is_empty() {
                        self.flush_tools(&mut out);
                        out.push(StreamEvent::Done {
                            stop_reason: reason.to_string(),
                        });
                    }
                }
            }
        }

        Ok(Decoded {
            events: out,
            terminal: false,
        })
    }

    fn accumulate(&mut self, tc: &Value) {
        let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let entry = self.tools.entry(idx).or_default();

        // id and name arrive once, on the first fragment.
        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
            if !id.is_empty() {
                entry.id = id.to_string();
            }
        }
        if let Some(name) = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
        {
            if !name.is_empty() {
                entry.name = name.to_string();
            }
        }
        if let Some(frag) = tc
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|v| v.as_str())
        {
            entry.args.push_str(frag);
        }
    }

    fn flush_tools(&mut self, out: &mut Vec<StreamEvent>) {
        for (_, acc) in std::mem::take(&mut self.tools) {
            // A fragment that never carried a name is not a callable request.
            if acc.name.is_empty() {
                continue;
            }
            let trimmed = acc.args.trim();
            let input: Value = if trimmed.is_empty() {
                json!({})
            } else {
                serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
            };
            out.push(StreamEvent::ToolCallDelta(ToolCall {
                id: acc.id,
                name: acc.name,
                input,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(chunks: &[&str]) -> Vec<StreamEvent> {
        let mut decoder = OpenAiSseDecoder::new();
        let mut leftover = String::new();
        let mut out = Vec::new();
        for chunk in chunks {
            leftover.push_str(chunk);
            for line in split_lines(&mut leftover) {
                let decoded = decoder.decode_line(&line).expect("decode failed");
                out.extend(decoded.events);
                if decoded.terminal {
                    return out;
                }
            }
        }
        out
    }

    fn as_text(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::TextDelta(t) => t,
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    fn as_tool(e: &StreamEvent) -> &ToolCall {
        match e {
            StreamEvent::ToolCallDelta(c) => c,
            other => panic!("expected ToolCallDelta, got {other:?}"),
        }
    }

    fn as_done(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::Done { stop_reason } => stop_reason,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn text_deltas_are_yielded_in_order() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n",
        ]);

        assert_eq!(events.len(), 2);
        let joined: String = events.iter().map(as_text).collect();
        assert_eq!(joined, "Hello");
    }

    #[test]
    fn empty_content_delta_is_skipped() {
        let events = drive(&["data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n"]);
        assert!(events.is_empty(), "got {events:?}");
    }

    #[test]
    fn streamed_tool_argument_fragments_are_reassembled() {
        // The shape OpenAI-compatible providers actually emit: id+name once,
        // then arguments split across chunks.
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"file_write\",\"arguments\":\"\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n",
        ]);

        assert_eq!(events.len(), 2, "got {events:?}");
        let call = as_tool(&events[0]);
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "file_write");
        assert_eq!(call.input, json!({ "path": "a.txt" }));
        assert_eq!(as_done(&events[1]), "tool_calls");
    }

    #[test]
    fn tool_calls_are_flushed_in_index_order() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"function\":{\"name\":\"beta\",\"arguments\":\"{\\\"b\\\":2}\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"function\":{\"name\":\"alpha\",\"arguments\":\"{\\\"a\\\":1}\"}}]}}]}\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n",
        ]);

        assert_eq!(events.len(), 3);
        assert_eq!(as_tool(&events[0]).name, "alpha");
        assert_eq!(as_tool(&events[0]).input, json!({ "a": 1 }));
        assert_eq!(as_tool(&events[1]).name, "beta");
        assert_eq!(as_tool(&events[1]).input, json!({ "b": 2 }));
    }

    #[test]
    fn tool_calls_are_flushed_on_the_done_sentinel_when_no_finish_reason_arrives() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"ping\",\"arguments\":\"{}\"}}]}}]}\n",
            "data: [DONE]\n",
        ]);

        assert_eq!(events.len(), 2);
        assert_eq!(as_tool(&events[0]).name, "ping");
        assert_eq!(as_done(&events[1]), "stop");
    }

    #[test]
    fn a_tool_call_with_no_arguments_yields_an_empty_object() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"list_stories\"}}]}}]}\n",
            "data: [DONE]\n",
        ]);

        assert_eq!(as_tool(&events[0]).input, json!({}));
    }

    #[test]
    fn malformed_tool_arguments_degrade_to_an_empty_object() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"broken\",\"arguments\":\"{oops\"}}]}}]}\n",
            "data: [DONE]\n",
        ]);

        assert_eq!(as_tool(&events[0]).name, "broken");
        assert_eq!(as_tool(&events[0]).input, json!({}));
    }

    #[test]
    fn a_fragment_that_never_carries_a_name_is_dropped() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{}\"}}]}}]}\n",
            "data: [DONE]\n",
        ]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "stop");
    }

    #[test]
    fn null_finish_reason_does_not_terminate_the_turn() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"},\"finish_reason\":null}]}\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"b\"},\"finish_reason\":null}]}\n",
        ]);

        assert_eq!(events.len(), 2);
        assert_eq!(as_text(&events[0]), "a");
        assert_eq!(as_text(&events[1]), "b");
    }

    #[test]
    fn finish_reason_stop_yields_done() {
        let events = drive(&["data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n"]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "stop");
    }

    #[test]
    fn done_sentinel_reports_terminal() {
        let mut decoder = OpenAiSseDecoder::new();
        let decoded = decoder.decode_line("data: [DONE]").expect("decode");

        assert!(decoded.terminal);
        assert_eq!(as_done(&decoded.events[0]), "stop");
    }

    #[test]
    fn non_data_lines_are_ignored() {
        let mut decoder = OpenAiSseDecoder::new();
        for line in ["", ": keep-alive", "event: message", "id: 42"] {
            let decoded = decoder.decode_line(line).expect("decode");
            assert!(decoded.events.is_empty(), "line {line:?} produced events");
            assert!(!decoded.terminal);
        }
    }

    #[test]
    fn data_prefix_without_a_space_is_accepted() {
        let events = drive(&["data:{\"choices\":[{\"delta\":{\"content\":\"tight\"}}]}\n"]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "tight");
    }

    #[test]
    fn a_line_split_across_two_chunks_is_joined() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"cont",
            "ent\":\"joined\"}}]}\n",
        ]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "joined");
    }

    #[test]
    fn a_partial_trailing_line_is_not_decoded_until_complete() {
        let mut buf = String::from("data: {\"choices\":[{\"del");
        let lines = split_lines(&mut buf);

        assert!(lines.is_empty());
        assert_eq!(buf, "data: {\"choices\":[{\"del");
    }

    #[test]
    fn malformed_data_line_is_a_hard_error() {
        let mut decoder = OpenAiSseDecoder::new();
        let err = decoder
            .decode_line("data: {not json}")
            .expect_err("malformed JSON should abort the stream");

        assert!(matches!(err, ApiError::Serialization(_)), "got {err:?}");
    }
}
