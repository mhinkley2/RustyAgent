use async_stream::try_stream;
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::{
    error::ApiError,
    provider::{EventStream, LlmProvider},
    types::{ChatMessage, CompletionConfig, MessageRole, StreamEvent, ToolCall, ToolDefinition, Usage},
};

const BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// How long a fetched catalogue is reused before another call re-fetches it.
///
/// `list_models` is a trait method callers treat as cheap, and the settings
/// screen calls it on every open. A model catalogue changes a few times a year,
/// so an hour is generous and still means at most one request per app session
/// in ordinary use.
const CATALOGUE_TTL: Duration = Duration::from_secs(60 * 60);

/// The catalogue used when the API cannot be reached.
///
/// Not a convenience. An empty dropdown makes the app look broken, and the
/// cases that produce one — no key configured yet, an offline laptop, a rate
/// limit — are all ordinary. This is the last hand-maintained copy of this
/// list; the API is the source of truth whenever it can be reached.
const FALLBACK_MODELS: [&str; 5] = [
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-haiku-4-5",
    "claude-opus-4-8",
    "claude-fable-5",
];

/// One model as the provider describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    /// What the provider calls it, for a dropdown. Falls back to the id.
    pub display_name: String,
    /// The model's real context window, when the provider reports one.
    ///
    /// Anthropic's Models API does not return this today — it carries `id`,
    /// `display_name`, `created_at` and `type`. Parsed as an `Option` so the
    /// day it does, the table in `pricing.rs` stops being the authority
    /// without another round of this work.
    pub max_input_tokens: Option<u32>,
}

impl ModelInfo {
    fn from_id(id: &str) -> Self {
        Self {
            id: id.to_string(),
            display_name: id.to_string(),
            max_input_tokens: None,
        }
    }
}

/// A catalogue and when it was fetched.
struct CachedCatalogue {
    models: Vec<ModelInfo>,
    fetched_at: Instant,
}

pub struct AnthropicClient {
    client: Client,
    api_key: String,
    base_url: String,
    /// The last catalogue fetched, and when.
    ///
    /// A `std::sync::Mutex` rather than tokio's: it is held only to read or
    /// replace the value, never across an await, and the guard is dropped
    /// before any network call begins.
    catalogue: Arc<Mutex<Option<CachedCatalogue>>>,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: BASE_URL.to_string(),
            catalogue: Arc::new(Mutex::new(None)),
        }
    }

    /// Point the client at a different origin. For tests.
    #[cfg(test)]
    fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// The model catalogue, fetched at most once per [`CATALOGUE_TTL`].
    ///
    /// Never fails. Every reason the call can go wrong — no key, no network, a
    /// rate limit, a shape this build does not understand — resolves to the
    /// fallback, because a caller populating a dropdown has nothing useful to
    /// do with an error and a user has nothing useful to do with an empty list.
    pub async fn catalogue(&self) -> Vec<ModelInfo> {
        if let Some(cached) = self.cached_catalogue() {
            return cached;
        }

        match self.fetch_catalogue().await {
            Ok(models) if !models.is_empty() => {
                self.store_catalogue(models.clone());
                models
            }
            // An empty catalogue is treated as a failure rather than cached.
            // Believing the provider has no models would empty the dropdown
            // for an hour on one odd response.
            Ok(_) => {
                warn!("Anthropic returned an empty model catalogue; using the built-in list");
                fallback_catalogue()
            }
            Err(error) => {
                // The key is in a header, never in the URL, and `ApiError`
                // carries only the status and body — so this cannot print it.
                warn!("Could not fetch the Anthropic model catalogue: {error}");
                fallback_catalogue()
            }
        }
    }

    fn cached_catalogue(&self) -> Option<Vec<ModelInfo>> {
        let guard = self.catalogue.lock().ok()?;
        let cached = guard.as_ref()?;
        (cached.fetched_at.elapsed() < CATALOGUE_TTL).then(|| cached.models.clone())
    }

    fn store_catalogue(&self, models: Vec<ModelInfo>) {
        if let Ok(mut guard) = self.catalogue.lock() {
            *guard = Some(CachedCatalogue {
                models,
                fetched_at: Instant::now(),
            });
        }
    }

    async fn fetch_catalogue(&self) -> Result<Vec<ModelInfo>, ApiError> {
        let response = self
            .client
            .get(format!("{}/models?limit=100", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            // The body, not the request: nothing here echoes the key back.
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let body: Value = response.json().await?;
        Ok(parse_catalogue(&body))
    }

    fn build_request_body(
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
        config: &CompletionConfig,
    ) -> Value {
        // Extract system prompt from messages or config.
        let system: Option<String> = config.system_prompt.clone().or_else(|| {
            messages
                .iter()
                .find(|m| m.role == MessageRole::System)
                .map(|m| m.content.clone())
        });

        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| {
                match m.role {
                    MessageRole::Assistant => {
                        // If there are tool calls, build a content-block array.
                        if let Some(calls) = &m.tool_calls {
                            let mut blocks: Vec<Value> = Vec::new();
                            if !m.content.is_empty() {
                                blocks.push(json!({ "type": "text", "text": m.content }));
                            }
                            for c in calls {
                                blocks.push(json!({
                                    "type": "tool_use",
                                    "id": c.id,
                                    "name": c.name,
                                    "input": c.input,
                                }));
                            }
                            json!({ "role": "assistant", "content": blocks })
                        } else {
                            json!({ "role": "assistant", "content": m.content })
                        }
                    }
                    MessageRole::Tool => {
                        // Anthropic wraps tool results in a user message with tool_result blocks.
                        json!({
                            "role": "user",
                            "content": [{
                                "type": "tool_result",
                                "tool_use_id": m.tool_call_id,
                                "content": m.content,
                            }]
                        })
                    }
                    _ => json!({ "role": "user", "content": m.content }),
                }
            })
            .collect();

        let mut api_tools: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        // Anthropic caches everything *up to and including* the marked block,
        // and renders tools before the system prompt, so one breakpoint on the
        // final tool covers the whole tool schema. Without this the full tool
        // list is re-billed at the uncached rate on every iteration of a run.
        if let Some(last) = api_tools.last_mut() {
            last["cache_control"] = json!({ "type": "ephemeral" });
        }

        let mut body = json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": true,
            "messages": api_messages,
        });

        if let Some(sys) = system {
            // The system prompt must be a content-block array to carry a
            // cache_control marker; a bare string cannot.
            body["system"] = json!([{
                "type": "text",
                "text": sys,
                "cache_control": { "type": "ephemeral" },
            }]);
        }
        if let Some(temp) = config.temperature {
            body["temperature"] = json!(temp);
        }
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        body
    }
}

// ---------------------------------------------------------------------------
// SSE decoding
//
// Split out from `stream_completion` as pure functions so the wire format can
// be tested without a network round-trip. `stream_completion` is a thin loop
// over these two.
// ---------------------------------------------------------------------------

/// Partial state for one `tool_use` content block.
///
/// Anthropic streams a tool call as `content_block_start` (carrying the id and
/// name) followed by any number of `input_json_delta` fragments that must be
/// concatenated before they parse as JSON, terminated by `content_block_stop`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ToolAccum {
    id: String,
    name: String,
    json: String,
}

/// Drain every complete SSE block (terminated by a blank line) from `buf`,
/// leaving any trailing partial block behind for the next chunk.
pub(crate) fn split_sse_blocks(buf: &mut String) -> Vec<String> {
    let mut blocks = Vec::new();
    while let Some(pos) = buf.find("\n\n") {
        blocks.push(buf[..pos].to_string());
        buf.drain(..pos + 2);
    }
    blocks
}

/// Fold a wire `usage` object into the running total for a stream.
///
/// Anthropic reports usage *cumulatively*: `message_start` carries the input
/// and cache counts, `message_delta` restates the running output count. Every
/// field is therefore an absolute, and must be assigned rather than summed —
/// adding the deltas would make the output count grow quadratically.
///
/// A payload carrying none of the four fields leaves `total` untouched, so
/// "the provider said nothing" stays distinguishable from "the provider said
/// zero".
fn merge_usage(total: &mut Option<Usage>, raw: &Value) {
    let field = |name: &str| raw.get(name).and_then(Value::as_u64);
    let (input, output, cache_read, cache_write) = (
        field("input_tokens"),
        field("output_tokens"),
        field("cache_read_input_tokens"),
        field("cache_creation_input_tokens"),
    );
    if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
        return;
    }

    let usage = total.get_or_insert_with(Usage::default);
    if let Some(v) = input {
        usage.input_tokens = v;
    }
    if let Some(v) = output {
        usage.output_tokens = v;
    }
    if let Some(v) = cache_read {
        usage.cache_read_input_tokens = v;
    }
    if let Some(v) = cache_write {
        usage.cache_creation_input_tokens = v;
    }
}

/// Turn one SSE block into zero or more `StreamEvent`s, updating the in-flight
/// tool accumulators keyed by content-block index and the stream's running
/// token usage.
pub(crate) fn decode_block(
    block: &str,
    tools: &mut HashMap<u32, ToolAccum>,
    usage: &mut Option<Usage>,
) -> Result<Vec<StreamEvent>, ApiError> {
    let mut event_type = String::new();
    let mut data_line = String::new();

    for line in block.lines() {
        if let Some(evt) = line.strip_prefix("event: ") {
            event_type = evt.trim().to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            data_line = d.trim().to_string();
        }
    }

    if data_line.is_empty() || data_line == "[DONE]" {
        return Ok(Vec::new());
    }

    let parsed: Value = serde_json::from_str(&data_line)?;

    // The `data` payload normally repeats the type; fall back to the `event:`
    // line when it does not.
    let type_field = parsed
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or(&event_type)
        .to_string();

    let mut out = Vec::new();

    match type_field.as_str() {
        "content_block_start" => {
            if let Some(cb) = parsed.get("content_block") {
                if cb.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                    let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
                    tools.insert(
                        idx,
                        ToolAccum {
                            id: cb["id"].as_str().unwrap_or("").to_string(),
                            name: cb["name"].as_str().unwrap_or("").to_string(),
                            json: String::new(),
                        },
                    );
                }
            }
        }
        "content_block_delta" => {
            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
            if let Some(delta) = parsed.get("delta") {
                match delta.get("type").and_then(|v| v.as_str()) {
                    Some("text_delta") => {
                        let text = delta["text"].as_str().unwrap_or("");
                        if !text.is_empty() {
                            out.push(StreamEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(acc) = tools.get_mut(&idx) {
                            acc.json
                                .push_str(delta["partial_json"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            let idx = parsed["index"].as_u64().unwrap_or(0) as u32;
            if let Some(acc) = tools.remove(&idx) {
                out.push(finish_tool_call(acc));
            }
        }
        "message_start" => {
            // Input and cache counts are only ever reported here.
            if let Some(u) = parsed.get("message").and_then(|m| m.get("usage")) {
                merge_usage(usage, u);
            }
        }
        "message_delta" => {
            // The final output count rides alongside the stop reason.
            if let Some(u) = parsed.get("usage") {
                merge_usage(usage, u);
            }
            if let Some(delta) = parsed.get("delta") {
                let stop_reason = delta
                    .get("stop_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("end_turn")
                    .to_string();
                out.push(StreamEvent::Done { stop_reason, usage: *usage });
            }
        }
        "error" => {
            let msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();
            out.push(StreamEvent::Error(msg));
        }
        _ => {
            debug!("Anthropic SSE unhandled event type: {type_field}");
        }
    }

    Ok(out)
}

/// Parse an accumulated tool block into a `ToolCallDelta`.
///
/// A malformed input is reported as a recoverable `Error` rather than aborting
/// the stream — the rest of the turn is still worth delivering.
fn finish_tool_call(acc: ToolAccum) -> StreamEvent {
    // A tool invoked with no arguments emits no `input_json_delta` at all.
    let trimmed = acc.json.trim();
    if trimmed.is_empty() {
        return StreamEvent::ToolCallDelta(ToolCall {
            id: acc.id,
            name: acc.name,
            input: json!({}),
        });
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(input) => StreamEvent::ToolCallDelta(ToolCall {
            id: acc.id,
            name: acc.name,
            input,
        }),
        Err(e) => StreamEvent::Error(format!(
            "Failed to parse tool input for `{}` (id {}): {e}",
            acc.name, acc.id
        )),
    }
}

#[async_trait]
impl LlmProvider for AnthropicClient {
    async fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
        config: CompletionConfig,
    ) -> Result<EventStream, ApiError> {
        let body = Self::build_request_body(&messages, &tools, &config);

        let response = self
            .client
            .post(format!("{BASE_URL}/messages"))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status().as_u16();
        if status == 429 {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(60);
            return Err(ApiError::RateLimited { retry_after_secs: retry });
        }
        if !response.status().is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(ApiError::Http { status, body: body_text });
        }

        let mut byte_stream = response.bytes_stream();

        let stream = try_stream! {
            // In-flight tool_use blocks, keyed by content-block index.
            let mut tools: HashMap<u32, ToolAccum> = HashMap::new();
            // Running token usage; None until the provider reports any.
            let mut usage: Option<Usage> = None;
            let mut leftover = String::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk: Bytes = chunk?;
                leftover.push_str(&String::from_utf8_lossy(&chunk));

                for block in split_sse_blocks(&mut leftover) {
                    for event in decode_block(&block, &mut tools, &mut usage)? {
                        yield event;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        // Fetched, cached for an hour, and never an error: see
        // `AnthropicClient::catalogue`. Callers treat this method as cheap, and
        // within the TTL it is.
        Ok(self
            .catalogue()
            .await
            .into_iter()
            .map(|model| model.id)
            .collect())
    }
}

/// The ids the provider falls back to when the API cannot be reached.
///
/// Public so the cross-catalogue drift check can compare it with the
/// frontend's list. These are the two hand-written copies that remain, and
/// they are shown to the same user in the same dropdown, so a disagreement
/// means the list changes under them the moment a key is entered.
pub fn anthropic_fallback_models() -> &'static [&'static str] {
    &FALLBACK_MODELS
}

/// The built-in catalogue, as [`ModelInfo`].
fn fallback_catalogue() -> Vec<ModelInfo> {
    FALLBACK_MODELS.iter().map(|id| ModelInfo::from_id(id)).collect()
}

/// Read a `GET /v1/models` body.
///
/// Tolerant on purpose. An entry without an `id` is the only thing that makes a
/// model unusable, so that one is dropped; everything else the provider adds or
/// renames later degrades to a default rather than failing the whole fetch and
/// emptying a dropdown.
fn parse_catalogue(body: &Value) -> Vec<ModelInfo> {
    let Some(entries) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?;
            Some(ModelInfo {
                id: id.to_string(),
                display_name: entry
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .to_string(),
                max_input_tokens: entry
                    .get("max_input_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- fixtures ----------------------------------------------------------

    /// Build one SSE block in Anthropic's wire format.
    fn sse(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    /// Feed chunks through the same split/decode loop `stream_completion` uses.
    fn drive(chunks: &[&str]) -> Vec<StreamEvent> {
        let mut usage = None;
        drive_with_usage(chunks, &mut usage)
    }

    /// As `drive`, but exposes the stream's accumulated usage to the caller.
    fn drive_with_usage(chunks: &[&str], usage: &mut Option<Usage>) -> Vec<StreamEvent> {
        let mut tools: HashMap<u32, ToolAccum> = HashMap::new();
        let mut leftover = String::new();
        let mut out = Vec::new();
        for chunk in chunks {
            leftover.push_str(chunk);
            for block in split_sse_blocks(&mut leftover) {
                out.extend(decode_block(&block, &mut tools, usage).expect("decode failed"));
            }
        }
        out
    }

    // -- accessors ---------------------------------------------------------

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

    fn as_error(e: &StreamEvent) -> &str {
        match e {
            StreamEvent::Error(m) => m,
            other => panic!("expected Error, got {other:?}"),
        }
    }

    // -- text ---------------------------------------------------------------

    #[test]
    fn text_deltas_are_yielded_in_order() {
        let events = drive(&[
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo "}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#,
            ),
        ]);

        assert_eq!(events.len(), 3);
        let joined: String = events.iter().map(as_text).collect();
        assert_eq!(joined, "Hello world");
    }

    #[test]
    fn empty_text_delta_is_skipped() {
        let events = drive(&[&sse(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}"#,
        )]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- tool calls (regression: these were silently dropped) ----------------

    #[test]
    fn tool_use_block_yields_tool_call_delta() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"file_write","input":{}}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.txt\","}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"content\":\"hi\"}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
        ]);

        assert_eq!(events.len(), 1, "expected exactly one event, got {events:?}");
        let call = as_tool(&events[0]);
        assert_eq!(call.id, "toolu_1");
        assert_eq!(call.name, "file_write");
        assert_eq!(call.input, json!({ "path": "a.txt", "content": "hi" }));
    }

    #[test]
    fn tool_use_with_no_arguments_yields_empty_object_input() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"list_stories"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
        ]);

        assert_eq!(events.len(), 1);
        let call = as_tool(&events[0]);
        assert_eq!(call.name, "list_stories");
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn two_concurrent_tool_blocks_are_keyed_by_index() {
        // Anthropic interleaves deltas for parallel tool blocks; the
        // accumulators must not bleed into one another.
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_a","name":"alpha"}}"#,
            ),
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_b","name":"beta"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"a\":"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"b\":"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"2}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":2}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
        ]);

        assert_eq!(events.len(), 2);
        // Emitted in stop order: index 2 first.
        let beta = as_tool(&events[0]);
        assert_eq!(beta.id, "toolu_b");
        assert_eq!(beta.input, json!({ "b": 2 }));

        let alpha = as_tool(&events[1]);
        assert_eq!(alpha.id, "toolu_a");
        assert_eq!(alpha.input, json!({ "a": 1 }));
    }

    #[test]
    fn text_and_tool_use_in_the_same_message_both_survive() {
        let events = drive(&[
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Let me check."}}"#,
            ),
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_story"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"id\":\"s1\"}"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":1}"#,
            ),
            &sse(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            ),
        ]);

        assert_eq!(events.len(), 3);
        assert_eq!(as_text(&events[0]), "Let me check.");
        assert_eq!(as_tool(&events[1]).name, "get_story");
        assert_eq!(as_done(&events[2]), "tool_use");
    }

    #[test]
    fn malformed_partial_json_yields_error_not_panic() {
        let events = drive(&[
            &sse(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_x","name":"broken"}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"unterminated\":"}}"#,
            ),
            &sse(
                "content_block_stop",
                r#"{"type":"content_block_stop","index":0}"#,
            ),
        ]);

        assert_eq!(events.len(), 1);
        let msg = as_error(&events[0]);
        assert!(msg.contains("broken"), "error should name the tool: {msg}");
        assert!(msg.contains("toolu_x"), "error should name the id: {msg}");
    }

    #[test]
    fn content_block_stop_for_a_text_block_yields_nothing() {
        let events = drive(&[&sse(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        )]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- terminal / error events --------------------------------------------

    #[test]
    fn message_delta_yields_done_with_stop_reason() {
        let events = drive(&[&sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "tool_use");
    }

    #[test]
    fn message_delta_without_stop_reason_defaults_to_end_turn() {
        let events = drive(&[&sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_sequence":null}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_done(&events[0]), "end_turn");
    }

    #[test]
    fn error_event_yields_stream_event_error() {
        let events = drive(&[&sse(
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_error(&events[0]), "Overloaded");
    }

    #[test]
    fn ping_and_unknown_event_types_are_ignored() {
        let events = drive(&[
            &sse("ping", r#"{"type":"ping"}"#),
            &sse(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","model":"claude-opus-4-5"}}"#,
            ),
            &sse("message_stop", r#"{"type":"message_stop"}"#),
            &sse("future_event", r#"{"type":"some_future_event"}"#),
        ]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    // -- framing -------------------------------------------------------------

    #[test]
    fn sse_block_split_across_two_chunks() {
        // The block boundary falls mid-token; the leftover buffer must join them.
        let full = sse(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"joined"}}"#,
        );
        let (head, tail) = full.split_at(30);

        let events = drive(&[head, tail]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "joined");
    }

    #[test]
    fn a_partial_trailing_block_is_not_decoded_until_complete() {
        let mut buf = String::from("event: content_block_delta\ndata: {\"type\":\"cont");
        let blocks = split_sse_blocks(&mut buf);

        assert!(blocks.is_empty());
        assert_eq!(buf, "event: content_block_delta\ndata: {\"type\":\"cont");
    }

    #[test]
    fn multiple_blocks_in_one_chunk_are_all_decoded() {
        let chunk = format!(
            "{}{}",
            sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            ),
            sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
            ),
        );

        let events = drive(&[&chunk]);

        assert_eq!(events.len(), 2);
        assert_eq!(as_text(&events[0]), "a");
        assert_eq!(as_text(&events[1]), "b");
    }

    #[test]
    fn event_type_falls_back_to_the_event_line_when_data_omits_it() {
        let events = drive(&[&sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"text_delta","text":"fallback"}}"#,
        )]);

        assert_eq!(events.len(), 1);
        assert_eq!(as_text(&events[0]), "fallback");
    }

    #[test]
    fn done_sentinel_and_empty_data_lines_are_skipped() {
        let events = drive(&["data: [DONE]\n\n", "event: ping\n\n"]);

        assert!(events.is_empty(), "expected no events, got {events:?}");
    }

    #[test]
    fn malformed_data_line_is_a_hard_error() {
        let mut tools = HashMap::new();
        let err = decode_block("event: ping\ndata: {not json}", &mut tools, &mut None)
            .expect_err("malformed JSON should abort the stream");

        assert!(matches!(err, ApiError::Serialization(_)), "got {err:?}");
    }

    // -- request body --------------------------------------------------------

    fn cfg() -> CompletionConfig {
        CompletionConfig::new("claude-opus-4-5", 1024)
    }

    #[test]
    fn build_request_body_maps_tool_role_to_user_tool_result_block() {
        let body = AnthropicClient::build_request_body(
            &[ChatMessage::tool_result("toolu_1", "file written")],
            &[],
            &cfg(),
        );

        assert_eq!(
            body["messages"][0],
            json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_1",
                    "content": "file written",
                }]
            })
        );
    }

    #[test]
    fn build_request_body_emits_tool_use_blocks_for_assistant_tool_calls() {
        let msg = ChatMessage::assistant_with_tool_calls(
            "checking",
            vec![ToolCall {
                id: "toolu_1".into(),
                name: "get_story".into(),
                input: json!({ "id": "s1" }),
            }],
        );

        let body = AnthropicClient::build_request_body(&[msg], &[], &cfg());

        assert_eq!(
            body["messages"][0],
            json!({
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "checking" },
                    { "type": "tool_use", "id": "toolu_1", "name": "get_story", "input": { "id": "s1" } },
                ]
            })
        );
    }

    #[test]
    fn build_request_body_prefers_config_system_prompt_over_system_message() {
        let mut config = cfg();
        config.system_prompt = Some("from config".into());

        let body = AnthropicClient::build_request_body(
            &[
                ChatMessage::system("from message"),
                ChatMessage::user("hello"),
            ],
            &[],
            &config,
        );

        assert_eq!(body["system"][0]["text"], json!("from config"));
        // The system message must not leak into the messages array.
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], json!("user"));
    }

    #[test]
    fn build_request_body_falls_back_to_the_system_message() {
        let body = AnthropicClient::build_request_body(
            &[
                ChatMessage::system("from message"),
                ChatMessage::user("hello"),
            ],
            &[],
            &cfg(),
        );

        assert_eq!(body["system"][0]["text"], json!("from message"));
    }

    #[test]
    fn build_request_body_omits_tools_key_when_empty() {
        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &[], &cfg());

        assert!(body.get("tools").is_none(), "body was {body}");
        assert!(body.get("system").is_none(), "body was {body}");
        assert!(body.get("temperature").is_none(), "body was {body}");
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["max_tokens"], json!(1024));
    }

    #[test]
    fn build_request_body_includes_tool_definitions_as_input_schema() {
        let tools = vec![ToolDefinition {
            name: "get_story".into(),
            description: "Fetch a story".into(),
            input_schema: json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
        }];

        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &tools, &cfg());

        assert_eq!(body["tools"][0]["name"], json!("get_story"));
        assert_eq!(body["tools"][0]["description"], json!("Fetch a story"));
        assert_eq!(
            body["tools"][0]["input_schema"],
            json!({ "type": "object", "properties": { "id": { "type": "string" } } })
        );
    }

    // -- prompt caching ------------------------------------------------------

    #[test]
    fn the_system_prompt_is_sent_as_a_cacheable_block() {
        let mut config = cfg();
        config.system_prompt = Some("a long standing instruction".into());

        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &[], &config);

        assert_eq!(
            body["system"],
            json!([{
                "type": "text",
                "text": "a long standing instruction",
                "cache_control": { "type": "ephemeral" },
            }])
        );
    }

    #[test]
    fn only_the_final_tool_definition_carries_the_cache_breakpoint() {
        // Anthropic caches the prefix up to the marked block, so one marker on
        // the last tool covers them all - and a request may not spend more than
        // four breakpoints in total.
        let tools: Vec<ToolDefinition> = ["alpha", "beta", "gamma"]
            .iter()
            .map(|name| ToolDefinition {
                name: (*name).into(),
                description: "t".into(),
                input_schema: json!({ "type": "object" }),
            })
            .collect();

        let body = AnthropicClient::build_request_body(&[ChatMessage::user("hi")], &tools, &cfg());

        let sent = body["tools"].as_array().expect("tools array");
        assert_eq!(sent.len(), 3);
        assert!(sent[0].get("cache_control").is_none(), "body was {body}");
        assert!(sent[1].get("cache_control").is_none(), "body was {body}");
        assert_eq!(sent[2]["cache_control"], json!({ "type": "ephemeral" }));
    }

    // -- usage ---------------------------------------------------------------

    #[test]
    fn usage_is_assembled_from_message_start_and_message_delta() {
        // Input and cache counts arrive on message_start, output on
        // message_delta; the Done event has to carry both halves.
        let events = drive(&[
            &sse(
                "message_start",
                r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":120,"output_tokens":1,"cache_read_input_tokens":900,"cache_creation_input_tokens":40}}}"#,
            ),
            &sse(
                "content_block_delta",
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
            ),
            &sse(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":57}}"#,
            ),
        ]);

        let usage = done_usage(events.last().expect("a Done event")).expect("usage reported");
        assert_eq!(usage.input_tokens, 120);
        assert_eq!(usage.output_tokens, 57, "the message_delta count is final");
        assert_eq!(usage.cache_read_input_tokens, 900);
        assert_eq!(usage.cache_creation_input_tokens, 40);
    }

    #[test]
    fn cumulative_output_counts_are_taken_not_summed() {
        // Anthropic restates a running total on every message_delta. Summing
        // them would make the reported output grow quadratically.
        let events = drive(&[
            &sse(
                "message_start",
                r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
            ),
            &sse(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":null},"usage":{"output_tokens":5}}"#,
            ),
            &sse(
                "message_delta",
                r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
            ),
        ]);

        let usage = done_usage(events.last().expect("a Done event")).expect("usage reported");
        assert_eq!(usage.output_tokens, 9, "5 + 9 would be double counting");
    }

    #[test]
    fn a_stream_that_never_reports_usage_yields_none_rather_than_zeros() {
        let events = drive(&[&sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        )]);

        assert_eq!(
            done_usage(&events[0]),
            None,
            "unmeasured usage must not masquerade as zero tokens"
        );
    }

    #[test]
    fn input_counts_survive_a_message_delta_that_only_restates_output() {
        let mut usage = None;
        drive_with_usage(
            &[
                &sse(
                    "message_start",
                    r#"{"type":"message_start","message":{"usage":{"input_tokens":77}}}"#,
                ),
                &sse(
                    "message_delta",
                    r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
                ),
            ],
            &mut usage,
        );

        let usage = usage.expect("usage reported");
        assert_eq!(usage.input_tokens, 77, "message_delta must not clear it");
        assert_eq!(usage.output_tokens, 3);
    }
}

// ---------------------------------------------------------------------------
// The model catalogue
// ---------------------------------------------------------------------------
//
// Four hand-maintained lists had to agree and only two were cross-checked. The
// drift shipped once already: three retired ids and one that was never valid
// were selectable in the profile editor, each of which would have failed on
// first use. These pin the replacement — fetch, cache, and above all fall back,
// because the failure modes here are all ordinary.

#[cfg(test)]
mod catalogue {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A one-shot HTTP server that answers every request the same way.
    ///
    /// Hand-rolled rather than a mocking crate: this needs one route and a
    /// request count, and `tokio` is already here.
    struct StubServer {
        base_url: String,
        requests: Arc<AtomicUsize>,
    }

    async fn stub(status: u16, body: &'static str) -> StubServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a port");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: \
                     {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Read the request line so the client is not writing into a
                // socket nobody drained.
                let mut scratch = [0u8; 1024];
                let _ = socket.read(&mut scratch).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        StubServer {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
        }
    }

    fn client_for(server: &StubServer) -> AnthropicClient {
        AnthropicClient::new("sk-ant-secret").with_base_url(&server.base_url)
    }

    const TWO_MODELS: &str = r#"{"data":[
        {"id":"claude-opus-5","display_name":"Claude Opus 5","type":"model"},
        {"id":"claude-sonnet-5","display_name":"Claude Sonnet 5","type":"model"}
    ]}"#;

    // -- parsing ------------------------------------------------------------

    #[test]
    fn a_catalogue_body_yields_its_models() {
        let body: Value = serde_json::from_str(TWO_MODELS).expect("valid json");

        let models = parse_catalogue(&body);

        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-opus-5");
        assert_eq!(models[0].display_name, "Claude Opus 5");
        assert_eq!(models[0].max_input_tokens, None);
    }

    #[test]
    fn a_model_without_a_display_name_falls_back_to_its_id() {
        let body = json!({ "data": [{ "id": "claude-opus-5" }] });

        let models = parse_catalogue(&body);

        assert_eq!(models[0].display_name, "claude-opus-5");
    }

    #[test]
    fn a_context_window_is_read_when_the_provider_reports_one() {
        // Anthropic does not return this today. Parsed anyway so the day it
        // does, the hand-kept table stops being the authority for free.
        let body = json!({ "data": [{ "id": "m", "max_input_tokens": 200_000 }] });

        assert_eq!(parse_catalogue(&body)[0].max_input_tokens, Some(200_000));
    }

    #[test]
    fn an_entry_with_no_id_is_dropped_rather_than_failing_the_fetch() {
        // An id is the only field that makes a model usable. Everything else
        // the provider might add or rename should degrade, not empty a dropdown.
        let body = json!({ "data": [{ "display_name": "Mystery" }, { "id": "real" }] });

        let models = parse_catalogue(&body);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "real");
    }

    #[test]
    fn a_body_of_an_unexpected_shape_yields_nothing_rather_than_panicking() {
        assert!(parse_catalogue(&json!({})).is_empty());
        assert!(parse_catalogue(&json!({ "data": "not an array" })).is_empty());
        assert!(parse_catalogue(&json!([])).is_empty());
    }

    // -- fetching, caching, falling back ------------------------------------

    #[tokio::test]
    async fn the_provider_catalogue_is_what_list_models_returns() {
        let server = stub(200, TWO_MODELS).await;

        let models = client_for(&server).list_models().await.expect("never errors");

        assert_eq!(models, ["claude-opus-5", "claude-sonnet-5"]);
    }

    #[tokio::test]
    async fn repeated_calls_within_the_ttl_make_one_request() {
        // `list_models` is a trait method callers treat as cheap, and the
        // settings screen calls it on every open.
        let server = stub(200, TWO_MODELS).await;
        let client = client_for(&server);

        for _ in 0..5 {
            let models = client.list_models().await.expect("never errors");
            assert_eq!(models.len(), 2);
        }

        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failing_api_yields_the_built_in_list_rather_than_an_error() {
        let server = stub(500, r#"{"error":"upstream is unhappy"}"#).await;

        let models = client_for(&server).list_models().await.expect("never errors");

        assert_eq!(models, FALLBACK_MODELS);
    }

    #[tokio::test]
    async fn a_rejected_key_yields_the_built_in_list() {
        // The state a user is in before configuring a provider — and the one
        // most likely to be read as "the app is broken" if it emptied the list.
        let server = stub(401, r#"{"type":"error","error":{"type":"authentication_error"}}"#).await;

        let models = client_for(&server).list_models().await.expect("never errors");

        assert_eq!(models, FALLBACK_MODELS);
    }

    #[tokio::test]
    async fn an_unreachable_api_yields_the_built_in_list() {
        // A port that was just released, so the connection is refused at once.
        // An unroutable address would exercise the same path but would spend
        // the connect timeout doing it, and this runs on every CI build.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a port");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);

        let client =
            AnthropicClient::new("sk-ant-secret").with_base_url(format!("http://127.0.0.1:{port}"));

        let models = client.list_models().await.expect("never errors");

        assert_eq!(models, FALLBACK_MODELS);
    }

    #[tokio::test]
    async fn an_empty_catalogue_is_not_cached_as_the_answer() {
        // Believing the provider has no models would empty the dropdown for the
        // whole TTL on the strength of one odd response.
        let server = stub(200, r#"{"data":[]}"#).await;
        let client = client_for(&server);

        assert_eq!(client.list_models().await.expect("never errors"), FALLBACK_MODELS);
        let _ = client.list_models().await;

        assert_eq!(
            server.requests.load(Ordering::SeqCst),
            2,
            "an empty answer was cached and the provider never asked again",
        );
    }

    #[tokio::test]
    async fn a_failed_fetch_is_retried_rather_than_cached() {
        let server = stub(500, "{}").await;
        let client = client_for(&server);

        let _ = client.list_models().await;
        let _ = client.list_models().await;

        assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn the_api_key_never_reaches_an_error_message() {
        // Errors get logged. The key travels in a header and the URL carries
        // only the path, so a failure has nowhere to pick it up — this is what
        // holds that true.
        let server = stub(403, r#"{"error":"forbidden"}"#).await;
        let client = client_for(&server);

        let error = client
            .fetch_catalogue()
            .await
            .expect_err("a 403 should fail the fetch");

        let rendered = format!("{error}");
        assert!(!rendered.contains("sk-ant-secret"), "the key leaked: {rendered}");
        assert!(rendered.contains("403"), "and the status should still be there: {rendered}");
    }
}

