//! Context-window budgeting and compaction for the agent loop.
//!
//! `ConversationRuntime::messages` only ever grew, so a long run eventually
//! sent the provider more input than the model could take, the call failed,
//! and the run died with its work lost. This module decides, before each
//! provider call, whether the conversation still fits — and if it does not,
//! which part of it to drop.
//!
//! Three things it is careful about:
//!
//! * **The system prompt and the task are never dropped.** An agent that
//!   forgets its instructions or what it was asked to do is worse than one
//!   that fails.
//! * **Tool calls and their results move as a unit.** Anthropic rejects a
//!   `tool_result` block with no preceding `tool_use`, so evicting an
//!   assistant message while keeping the tool results that answer it turns a
//!   survivable run into a hard 400 — the opposite of the intent.
//! * **The budget is checked against an estimate that errs high**
//!   ([`api::tokens`]), reconciled afterwards against the provider's real
//!   figures.

use api::{ChatMessage, CompletionConfig, MessageRole};
use tracing::warn;

// ---------------------------------------------------------------------------
// Strategy
// ---------------------------------------------------------------------------

/// How a profile wants an over-budget conversation handled.
///
/// Parsed from `agent_profiles.context_strategy`, which is a free-text column
/// written by the profile editor, by TOML workspace files, and by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStrategy {
    /// Drop the oldest turns until the conversation fits.
    Recent,
    /// As `Recent`, but replace what was dropped with a generated summary.
    Summary,
    /// Never compact. Fail loudly when the budget is exceeded rather than
    /// silently forgetting something the operator expected to be kept.
    Full,
}

impl ContextStrategy {
    /// What an unrecognised — or absent — `context_strategy` resolves to.
    ///
    /// `recent` because it is the column's schema default, the only strategy
    /// that costs nothing extra, and the one that keeps a run alive.
    pub const DEFAULT: Self = Self::Recent;

    /// Parse the stored string. Unknown values fall back to [`Self::DEFAULT`]
    /// with a warning — a typo in a settings field must not panic a run.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "recent" => Self::Recent,
            "summary" => Self::Summary,
            "full" => Self::Full,
            "" => Self::DEFAULT,
            other => {
                warn!(
                    "Unrecognised context_strategy '{other}'; falling back to '{}'",
                    Self::DEFAULT.as_str()
                );
                Self::DEFAULT
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Summary => "summary",
            Self::Full => "full",
        }
    }
}

impl Default for ContextStrategy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Slack left between the budget and the model's real window, on top of the
/// response reservation.
///
/// The estimate is approximate and the provider adds framing of its own, so a
/// budget set exactly at `window - max_output_tokens` would still overflow on
/// a bad guess.
pub const SAFETY_HEADROOM_TOKENS: u64 = 2_048;

/// A profile's context settings, resolved into something the loop can enforce.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextPolicy {
    pub strategy: ContextStrategy,
    /// `agent_profiles.max_input_tokens`. `None` — the column is nullable and
    /// usually null — means "derive one from the model".
    pub max_input_tokens: Option<u64>,
}

impl ContextPolicy {
    /// Build from the two raw profile columns.
    ///
    /// `max_input_tokens` is a signed column; a zero or negative value is
    /// treated as unset rather than as a budget of nothing, which no run could
    /// ever satisfy.
    pub fn from_profile(strategy: &str, max_input_tokens: Option<i64>) -> Self {
        Self {
            strategy: ContextStrategy::parse(strategy),
            max_input_tokens: max_input_tokens.filter(|v| *v > 0).map(|v| v as u64),
        }
    }

    /// The input-token budget for one call under `config`.
    ///
    /// Unset falls back to the model's window minus room for the response.
    /// An explicit setting is honoured, but never above what the model can
    /// actually take — a profile configured for a 200k budget against a
    /// smaller model is a misconfiguration, and letting it through would send
    /// the oversized request this exists to prevent. For a model the table
    /// does not know there is nothing to clamp against, so the operator's
    /// number stands.
    pub fn budget_tokens(&self, config: &CompletionConfig) -> u64 {
        let derived = |window: u64| {
            window
                .saturating_sub(config.max_tokens as u64)
                .saturating_sub(SAFETY_HEADROOM_TOKENS)
                .max(1)
        };

        match (self.max_input_tokens, api::context_window(&config.model)) {
            (Some(explicit), Some(window)) => explicit.min(derived(window)),
            (Some(explicit), None) => explicit,
            (None, _) => derived(api::context_window_or_default(&config.model)),
        }
    }
}

// ---------------------------------------------------------------------------
// Eviction planning
// ---------------------------------------------------------------------------

/// What compaction would do to a message list, computed without touching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    /// Messages `[0, protected)` are never evicted.
    pub protected: usize,
    /// Messages `[protected, evict_end)` would be dropped.
    pub evict_end: usize,
    /// Estimated request size before compaction, overhead included.
    pub before_tokens: u64,
    /// Estimated request size after it.
    pub after_tokens: u64,
    /// Whether `after_tokens` actually lands within the budget. `false` means
    /// even the protected prefix plus the overhead is too big, and there is
    /// nothing further compaction can do.
    pub fits: bool,
}

impl CompactionPlan {
    pub fn evicted_count(&self) -> usize {
        self.evict_end.saturating_sub(self.protected)
    }

    pub fn is_noop(&self) -> bool {
        self.evicted_count() == 0
    }
}

/// Does this message open a tool-call unit that its results must stay with?
fn opens_tool_unit(message: &ChatMessage) -> bool {
    message.role == MessageRole::Assistant
        && message
            .tool_calls
            .as_ref()
            .map(|calls| !calls.is_empty())
            .unwrap_or(false)
}

/// Exclusive end of the eviction unit beginning at `start`.
///
/// Normally one message. An assistant turn that requested tools swallows the
/// `tool` messages that answer it, so the pair is dropped or kept together.
fn unit_end(messages: &[ChatMessage], start: usize) -> usize {
    let mut end = start + 1;
    if opens_tool_unit(&messages[start]) {
        while end < messages.len() && messages[end].role == MessageRole::Tool {
            end += 1;
        }
    }
    end
}

/// How many leading messages are off limits.
///
/// Any system messages at the head, plus the first user message — the one
/// that states the task. Everything after that is history.
pub fn protected_prefix(messages: &[ChatMessage]) -> usize {
    let mut protected = 0;
    while protected < messages.len() && messages[protected].role == MessageRole::System {
        protected += 1;
    }
    if let Some(offset) = messages[protected..]
        .iter()
        .position(|m| m.role == MessageRole::User)
    {
        protected += offset + 1;
    }
    protected
}

/// Decide what to evict so that `overhead + messages` fits inside `budget`.
///
/// `overhead` is the part of the request that is not a message and cannot be
/// compacted — system prompt and tool schemas.
///
/// `scale` corrects the raw estimate against what the provider actually
/// counted on the last call (1.0 before there is any evidence). It multiplies
/// the estimate rather than dividing the budget so that every figure the plan
/// reports — and therefore every figure in the run event — is in the same
/// units as the operator's configured `max_input_tokens`.
pub fn plan_compaction(
    messages: &[ChatMessage],
    overhead: u64,
    budget: u64,
    scale: f64,
) -> CompactionPlan {
    let calibrate = |tokens: u64| {
        if scale <= 0.0 || !scale.is_finite() {
            return tokens;
        }
        // Saturating: `as u64` clamps rather than wrapping, and a token count
        // large enough to overflow is over budget under any reading.
        ((tokens as f64) * scale).ceil() as u64
    };

    let per_message: Vec<u64> = messages
        .iter()
        .map(|m| calibrate(api::tokens::estimate_message(m)))
        .collect();
    let before_tokens = calibrate(overhead) + per_message.iter().sum::<u64>();

    let protected = protected_prefix(messages);
    let mut evict_end = protected;
    let mut remaining = before_tokens;

    while remaining > budget && evict_end < messages.len() {
        let end = unit_end(messages, evict_end);
        let unit: u64 = per_message[evict_end..end].iter().sum();
        remaining = remaining.saturating_sub(unit);
        evict_end = end;
    }

    CompactionPlan {
        protected,
        evict_end,
        before_tokens,
        after_tokens: remaining,
        fits: remaining <= budget,
    }
}

// ---------------------------------------------------------------------------
// Summarisation
// ---------------------------------------------------------------------------

/// Marker the reinserted summary carries, so a reader of the transcript — or
/// of `run_events` — can tell a synthetic message from a real turn.
pub const SUMMARY_PREFIX: &str = "[compacted context] ";

/// Longest transcript handed to the summariser, in bytes.
///
/// The dropped prefix can be larger than the window it was dropped for, so it
/// cannot be sent whole. The *tail* is kept: the newest of the dropped turns
/// is the part the agent is most likely to still need.
const MAX_TRANSCRIPT_BYTES: usize = 40_000;

/// Longest any single message runs before it is elided in the transcript.
const MAX_MESSAGE_BYTES: usize = 2_000;

/// System prompt for the summarisation call.
pub const SUMMARY_SYSTEM_PROMPT: &str = "\
You compress an AI agent's conversation history so the agent can keep working \
after the older turns are dropped from its context window. Reply with the \
summary only — no preamble, no commentary.";

/// Truncate on a character boundary, never mid-codepoint.
///
/// Slicing a `String` at a fixed byte offset panics on multibyte text, and
/// agent output is routinely non-ASCII.
fn floor_boundary(text: &str, max_bytes: usize) -> usize {
    if text.len() <= max_bytes {
        return text.len();
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

fn ceil_boundary(text: &str, from: usize) -> usize {
    let mut cut = from.min(text.len());
    while cut < text.len() && !text.is_char_boundary(cut) {
        cut += 1;
    }
    cut
}

fn render_message(message: &ChatMessage) -> String {
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool_result",
    };

    let mut body = message.content[..floor_boundary(&message.content, MAX_MESSAGE_BYTES)].to_string();
    if body.len() < message.content.len() {
        body.push('…');
    }

    if let Some(calls) = &message.tool_calls {
        for call in calls {
            body.push_str(&format!("\n[called tool {}]", call.name));
        }
    }

    format!("{role}: {body}")
}

/// Build the prompt that asks the model to summarise the evicted prefix.
pub fn summary_prompt(evicted: &[ChatMessage]) -> String {
    let transcript = evicted
        .iter()
        .map(render_message)
        .collect::<Vec<_>>()
        .join("\n\n");

    let start = ceil_boundary(
        &transcript,
        transcript.len().saturating_sub(MAX_TRANSCRIPT_BYTES),
    );
    let transcript = &transcript[start..];

    format!(
        "The following earlier turns of an agent run are about to be dropped from \
its context window. Write a compact summary that lets the agent continue \
without them.\n\n\
Keep: the task and any constraints on it, decisions taken and why, files and \
commands touched with their outcomes, what has been verified, and what is \
still outstanding. Drop: pleasantries and full tool output bodies.\n\n\
If a decision or fact matters beyond this run, say so explicitly — the agent \
can re-read it later with the `memory_read` tool, but only if it knows to \
look.\n\n\
--- transcript ---\n{transcript}\n--- end transcript ---"
    )
}
