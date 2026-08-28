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
    types::{StreamEvent, ToolCall, Usage},
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
    /// Usage from the trailing `include_usage` chunk; None until one arrives.
    usage: Option<Usage>,
    /// A `finish_reason` we have seen but not yet turned into a `Done`.
    ///
    /// With `stream_options.include_usage`, the usage chunk arrives *after*
    /// the chunk carrying `finish_reason`. Emitting `Done` immediately would
    /// end the turn one chunk before the token counts show up, so the terminal
    /// event is held until the usage chunk, the `[DONE]` sentinel, or the end
    /// of the stream — whichever comes first.
    pending_done: Option<String>,
    /// Whether the terminal event has already gone out. The turn ends exactly
    /// once, so a `[DONE]` sentinel trailing an already-released `Done` must
    /// not produce a second one.
    done_emitted: bool,
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
            let reason = self.pending_done.take().unwrap_or_else(|| "stop".to_string());
            self.emit_done(&mut out, reason);
            return Ok(Decoded {
                events: out,
                terminal: true,
            });
        }

        if data.is_empty() {
            return Ok(Decoded::default());
        }

        let parsed: Value = serde_json::from_str(data)?;

        // Read usage before the choices, so a provider that packs both into one
        // chunk still gets its counts onto that chunk's `Done`.
        if let Some(raw) = parsed.get("usage") {
            self.absorb_usage(raw);
        }

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
                        self.pending_done = Some(reason.to_string());
                    }
                }
            }
        }

        // Release the held terminal event as soon as usage is in hand.
        if self.usage.is_some() {
            if let Some(reason) = self.pending_done.take() {
                self.emit_done(&mut out, reason);
            }
        }

        Ok(Decoded {
            events: out,
            terminal: false,
        })
    }

    /// Push the turn's single terminal event, if it has not gone out already.
    fn emit_done(&mut self, out: &mut Vec<StreamEvent>, stop_reason: String) {
        if self.done_emitted {
            return;
        }
        self.done_emitted = true;
        out.push(StreamEvent::Done {
            stop_reason,
            usage: self.usage,
        });
    }

    /// Flush anything still held when the byte stream ends.
    ///
    /// A provider that closes the connection without a `[DONE]` sentinel would
    /// otherwise strand the deferred `Done` — and any tool call that never got
    /// a `finish_reason` — inside the decoder.
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        self.flush_tools(&mut out);
        if let Some(reason) = self.pending_done.take() {
            self.emit_done(&mut out, reason);
        }
        out
    }

    /// Normalise one OpenAI-shaped `usage` object.
    ///
    /// OpenAI counts cached tokens *inside* `prompt_tokens`, whereas [`Usage`]
    /// follows Anthropic and keeps them apart, so the cached portion is
    /// subtracted out rather than counted twice. These providers report no
    /// cache-write count at all, which stays zero.
    fn absorb_usage(&mut self, raw: &Value) {
        let field = |name: &str| raw.get(name).and_then(Value::as_u64);
        let (prompt, completion) = (field("prompt_tokens"), field("completion_tokens"));
        if prompt.is_none() && completion.is_none() {
            return;
        }

        let cached = raw
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let prompt = prompt.unwrap_or(0);

        self.usage = Some(Usage {
            input_tokens: prompt.saturating_sub(cached),
            output_tokens: completion.unwrap_or(0),
            cache_read_input_tokens: cached.min(prompt),
            cache_creation_input_tokens: 0,
        });
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

    /// Feed chunks through the same loop the clients use, including the
    /// end-of-stream flush.
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
        out.extend(decoder.finish());
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
            StreamEvent::Done { stop_reason, .. } => stop_reason,
            other => panic!("expected Done, got {other:?}"),
        }
    }

    fn done_usage(e: &StreamEvent) -> Option<Usage> {
        match e {
            StreamEvent::Done { usage, .. } => *usage,
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

    // -- usage ---------------------------------------------------------------

    #[test]
    fn the_trailing_usage_chunk_is_attached_to_the_done_event() {
        // With stream_options.include_usage the counts arrive one chunk *after*
        // finish_reason, so Done has to wait for them.
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":300,\"completion_tokens\":25}}\n",
            "data: [DONE]\n",
        ]);

        assert_eq!(events.len(), 2, "got {events:?}");
        assert_eq!(as_done(&events[1]), "stop", "the real reason, not the sentinel default");
        let usage = done_usage(&events[1]).expect("usage reported");
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.output_tokens, 25);
    }

    #[test]
    fn cached_prompt_tokens_are_split_out_of_the_prompt_total() {
        // OpenAI counts cached tokens inside prompt_tokens; Usage keeps them
        // apart, so counting both would double-bill the cached prefix.
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":10,\"prompt_tokens_details\":{\"cached_tokens\":800}}}\n",
        ]);

        let usage = done_usage(&events[0]).expect("usage reported");
        assert_eq!(usage.input_tokens, 200, "uncached remainder");
        assert_eq!(usage.cache_read_input_tokens, 800);
        assert_eq!(usage.total_input_tokens(), 1000, "and they still sum back");
    }

    #[test]
    fn usage_arriving_with_the_finish_reason_does_not_delay_the_done_event() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n",
        ]);

        assert_eq!(events.len(), 1);
        assert!(done_usage(&events[0]).is_some());
    }

    #[test]
    fn a_stream_that_reports_no_usage_still_terminates_with_none() {
        let events = drive(&["data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n"]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "stop");
        assert_eq!(done_usage(&events[0]), None);
    }

    #[test]
    fn a_connection_that_closes_after_finish_reason_still_yields_done() {
        // No [DONE] sentinel and no usage chunk: the end-of-stream flush is the
        // only thing that can release the held terminal event.
        let mut decoder = OpenAiSseDecoder::new();
        let decoded = decoder
            .decode_line("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}")
            .expect("decode");
        assert!(decoded.events.is_empty(), "Done is held pending usage");

        let flushed = decoder.finish();
        assert_eq!(flushed.len(), 1);
        assert_eq!(as_done(&flushed[0]), "length");
        assert_eq!(done_usage(&flushed[0]), None);
    }

    #[test]
    fn an_empty_usage_object_is_ignored_rather_than_reported_as_zero() {
        let events = drive(&[
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":null}\n",
        ]);

        assert_eq!(done_usage(&events[0]), None);
    }

    #[test]
    fn finish_is_idempotent_once_the_terminal_event_has_been_emitted() {
        let mut decoder = OpenAiSseDecoder::new();
        decoder.decode_line("data: [DONE]").expect("decode");

        assert!(decoder.finish().is_empty(), "Done must not be emitted twice");
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
