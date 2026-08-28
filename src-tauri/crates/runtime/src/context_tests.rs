//! Unit tests for context budgeting and eviction planning.
//!
//! These cover the decision in isolation — what gets dropped, what is refused,
//! what the budget resolves to. The end-to-end behaviour (a run that survives
//! past the point it used to die, the emitted event, the summariser's failure
//! path) is driven through the real loop in `runtime_tests.rs`.

use api::{ChatMessage, CompletionConfig, MessageRole, ToolCall};
use serde_json::json;

use crate::context::{
    plan_compaction, protected_prefix, summary_prompt, ContextPolicy, ContextStrategy,
    SAFETY_HEADROOM_TOKENS,
};

// ---------------------------------------------------------------------------
// Strategy parsing
// ---------------------------------------------------------------------------

#[test]
fn each_documented_strategy_string_parses_to_its_variant() {
    assert_eq!(ContextStrategy::parse("recent"), ContextStrategy::Recent);
    assert_eq!(ContextStrategy::parse("summary"), ContextStrategy::Summary);
    assert_eq!(ContextStrategy::parse("full"), ContextStrategy::Full);
}

#[test]
fn strategy_parsing_tolerates_casing_and_surrounding_whitespace() {
    // The column is written by a TOML file and by hand as well as by the UI.
    assert_eq!(ContextStrategy::parse("  FULL "), ContextStrategy::Full);
    assert_eq!(ContextStrategy::parse("Summary"), ContextStrategy::Summary);
}

#[test]
fn an_unrecognised_strategy_falls_back_to_recent_instead_of_panicking() {
    assert_eq!(ContextStrategy::parse("agressive"), ContextStrategy::DEFAULT);
    assert_eq!(ContextStrategy::parse(""), ContextStrategy::DEFAULT);
    assert_eq!(ContextStrategy::parse("null"), ContextStrategy::DEFAULT);
    assert_eq!(ContextStrategy::DEFAULT, ContextStrategy::Recent);
}

// ---------------------------------------------------------------------------
// Budget derivation
// ---------------------------------------------------------------------------

/// A model the window table knows.
const KNOWN_MODEL: &str = "claude-haiku-4-5"; // 200k window

#[test]
fn an_unset_max_input_tokens_derives_the_budget_from_the_models_window() {
    let policy = ContextPolicy::from_profile("recent", None);
    let config = CompletionConfig::new(KNOWN_MODEL, 8_192);

    assert_eq!(
        policy.budget_tokens(&config),
        200_000 - 8_192 - SAFETY_HEADROOM_TOKENS
    );
}

#[test]
fn the_derived_budget_reserves_room_for_the_response() {
    // Budgeting the whole window would still overflow once the model starts
    // writing — the reservation is the point.
    let policy = ContextPolicy::from_profile("recent", None);

    let small_response = policy.budget_tokens(&CompletionConfig::new(KNOWN_MODEL, 1_024));
    let large_response = policy.budget_tokens(&CompletionConfig::new(KNOWN_MODEL, 64_000));

    assert!(small_response > large_response);
    assert_eq!(small_response - large_response, 64_000 - 1_024);
    assert!(small_response < 200_000, "the full window must not be budgeted");
}

#[test]
fn an_explicit_max_input_tokens_sets_the_budget() {
    let policy = ContextPolicy::from_profile("recent", Some(50_000));

    assert_eq!(
        policy.budget_tokens(&CompletionConfig::new(KNOWN_MODEL, 4_096)),
        50_000
    );
}

#[test]
fn an_explicit_budget_larger_than_the_model_is_clamped_to_what_fits() {
    // A profile pointed at a smaller model must not be allowed to send a
    // request the model cannot take.
    let policy = ContextPolicy::from_profile("recent", Some(900_000));

    assert_eq!(
        policy.budget_tokens(&CompletionConfig::new(KNOWN_MODEL, 4_096)),
        200_000 - 4_096 - SAFETY_HEADROOM_TOKENS
    );
}

#[test]
fn an_explicit_budget_stands_when_the_model_window_is_unknown() {
    // Nothing to clamp against, and the operator knows their local model
    // better than the fallback does.
    let policy = ContextPolicy::from_profile("recent", Some(300_000));

    assert_eq!(
        policy.budget_tokens(&CompletionConfig::new("some-local-llama", 4_096)),
        300_000
    );
}

#[test]
fn a_null_or_nonsense_max_input_tokens_is_treated_as_unset() {
    // A budget of zero or less is not a budget any run could satisfy.
    let zero = ContextPolicy::from_profile("recent", Some(0));
    let negative = ContextPolicy::from_profile("recent", Some(-1));
    let unset = ContextPolicy::from_profile("recent", None);
    let config = CompletionConfig::new(KNOWN_MODEL, 4_096);

    assert_eq!(zero.budget_tokens(&config), unset.budget_tokens(&config));
    assert_eq!(negative.budget_tokens(&config), unset.budget_tokens(&config));
}

#[test]
fn a_budget_is_never_zero_even_for_an_absurd_output_reservation() {
    // max_output_tokens larger than the window would saturate to zero and make
    // every request unsatisfiable; the floor keeps the failure legible.
    let policy = ContextPolicy::from_profile("recent", None);

    assert!(policy.budget_tokens(&CompletionConfig::new(KNOWN_MODEL, u32::MAX)) > 0);
}

// ---------------------------------------------------------------------------
// Protection
// ---------------------------------------------------------------------------

fn filler(role: &str, size: usize) -> ChatMessage {
    let body = "x".repeat(size);
    match role {
        "user" => ChatMessage::user(body),
        _ => ChatMessage::assistant(body),
    }
}

#[test]
fn the_system_prompt_and_the_first_user_message_are_protected() {
    let messages = vec![
        ChatMessage::system("you are an agent"),
        ChatMessage::user("do the thing"),
        ChatMessage::assistant("ok"),
        ChatMessage::user("also this"),
    ];

    assert_eq!(protected_prefix(&messages), 2);
}

#[test]
fn a_conversation_with_no_system_message_still_protects_the_task() {
    let messages = vec![
        ChatMessage::user("do the thing"),
        ChatMessage::assistant("ok"),
    ];

    assert_eq!(protected_prefix(&messages), 1);
}

#[test]
fn protection_handles_an_empty_conversation() {
    assert_eq!(protected_prefix(&[]), 0);
}

#[test]
fn compaction_never_evicts_the_protected_prefix_even_at_an_impossible_budget() {
    let messages = vec![
        ChatMessage::system("you are an agent"),
        filler("user", 4_000),
        filler("assistant", 4_000),
        filler("user", 4_000),
    ];

    let plan = plan_compaction(&messages, 0, 1, 1.0);

    assert_eq!(plan.evict_end, messages.len(), "everything else goes");
    assert_eq!(plan.protected, 2);
    assert!(!plan.fits, "the prefix alone still exceeds the budget");
}

// ---------------------------------------------------------------------------
// Eviction
// ---------------------------------------------------------------------------

#[test]
fn a_conversation_inside_the_budget_is_left_alone() {
    let messages = vec![
        ChatMessage::user("do the thing"),
        ChatMessage::assistant("ok"),
    ];

    let plan = plan_compaction(&messages, 0, 100_000, 1.0);

    assert!(plan.is_noop());
    assert!(plan.fits);
    assert_eq!(plan.before_tokens, plan.after_tokens);
}

#[test]
fn eviction_takes_the_oldest_turns_first_and_stops_once_it_fits() {
    let messages = vec![
        ChatMessage::user("task"),
        filler("assistant", 3_000), // ~1000 tokens each
        filler("user", 3_000),
        filler("assistant", 3_000),
    ];

    let plan = plan_compaction(&messages, 0, 2_200, 1.0);

    assert!(plan.fits);
    assert_eq!(plan.protected, 1);
    assert_eq!(plan.evict_end, 2, "only the oldest droppable turn goes");
    assert!(plan.after_tokens <= 2_200);
    assert!(plan.after_tokens < plan.before_tokens);
}

#[test]
fn the_non_evictable_overhead_counts_against_the_budget() {
    // System prompt and tool schemas are input too. Ignoring them budgets a
    // conversation that then overflows on the parts nobody measured.
    let messages = vec![ChatMessage::user("task"), filler("assistant", 3_000)];

    let without = plan_compaction(&messages, 0, 1_500, 1.0);
    let with = plan_compaction(&messages, 5_000, 1_500, 1.0);

    assert!(without.before_tokens < with.before_tokens);
    assert_eq!(with.before_tokens - without.before_tokens, 5_000);
    assert!(!with.fits, "overhead alone blows the budget");
}

fn tool_turn(id: &str, payload_size: usize) -> (ChatMessage, ChatMessage) {
    let call = ChatMessage::assistant_with_tool_calls(
        "",
        vec![ToolCall {
            id: id.to_string(),
            name: "file_read".into(),
            input: json!({ "path": "a.txt" }),
        }],
    );
    let result = ChatMessage::tool_result(id, "y".repeat(payload_size));
    (call, result)
}

#[test]
fn an_assistant_tool_call_and_its_result_are_evicted_together() {
    // A `tool_result` with no preceding `tool_use` is a hard 400 from
    // Anthropic — the naive drop turns a survivable run into a dead one.
    let (call, result) = tool_turn("c1", 6_000);
    let messages = vec![ChatMessage::user("task"), call, result];

    // A budget that the assistant message alone would satisfy, so a
    // per-message eviction would stop after dropping just the call.
    let plan = plan_compaction(&messages, 0, 1_000, 1.0);

    assert_eq!(plan.evict_end, 3, "the pair moves as one unit");
    let survivors = &messages[plan.evict_end..];
    assert!(survivors.iter().all(|m| m.role != MessageRole::Tool));
}

#[test]
fn every_result_of_a_multi_tool_turn_leaves_with_its_call() {
    let call = ChatMessage::assistant_with_tool_calls(
        "",
        vec![
            ToolCall { id: "c1".into(), name: "alpha".into(), input: json!({}) },
            ToolCall { id: "c2".into(), name: "beta".into(), input: json!({}) },
        ],
    );
    let messages = vec![
        ChatMessage::user("task"),
        call,
        ChatMessage::tool_result("c1", "A".repeat(3_000)),
        ChatMessage::tool_result("c2", "B".repeat(3_000)),
        ChatMessage::assistant("done"),
    ];

    let plan = plan_compaction(&messages, 0, 1_200, 1.0);

    assert_eq!(plan.evict_end, 4, "both results go with the one call");
    assert!(plan.fits);
}

#[test]
fn no_surviving_tool_result_is_ever_orphaned_from_its_call() {
    // Sweep every budget from "everything fits" to "nothing fits" and assert
    // the invariant holds at each one, rather than at a single hand-picked
    // number that happens to work.
    let (call_a, result_a) = tool_turn("c1", 900);
    let (call_b, result_b) = tool_turn("c2", 900);
    let messages = vec![
        ChatMessage::user("task"),
        call_a,
        result_a,
        ChatMessage::assistant("thinking"),
        call_b,
        result_b,
        ChatMessage::assistant("done"),
    ];

    for budget in 0..=plan_compaction(&messages, 0, 0, 1.0).before_tokens {
        let plan = plan_compaction(&messages, 0, budget, 1.0);
        let survivors: Vec<&ChatMessage> = messages[..plan.protected]
            .iter()
            .chain(messages[plan.evict_end..].iter())
            .collect();

        let mut open: Vec<&str> = Vec::new();
        for message in &survivors {
            if let Some(calls) = &message.tool_calls {
                open = calls.iter().map(|c| c.id.as_str()).collect();
            } else if message.role == MessageRole::Tool {
                let id = message.tool_call_id.as_deref().unwrap_or_default();
                assert!(
                    open.contains(&id),
                    "budget {budget}: tool_result '{id}' survived without its tool_use"
                );
            }
        }
    }
}

#[test]
fn an_unanswered_tool_call_at_the_tail_is_still_a_valid_unit() {
    // The loop can be interrupted between the assistant's request and the
    // results; the unit walker must not run off the end.
    let (call, _) = tool_turn("c1", 0);
    let messages = vec![ChatMessage::user("task"), filler("assistant", 9_000), call];

    // Budget zero, so eviction is forced all the way to the end.
    let plan = plan_compaction(&messages, 0, 0, 1.0);

    assert_eq!(plan.evict_end, messages.len());
}

// ---------------------------------------------------------------------------
// Summary prompt
// ---------------------------------------------------------------------------

#[test]
fn the_summary_prompt_carries_the_evicted_turns_and_points_at_memory() {
    let evicted = vec![
        ChatMessage::user("build the parser"),
        ChatMessage::assistant("chose a recursive descent design"),
    ];

    let prompt = summary_prompt(&evicted);

    assert!(prompt.contains("build the parser"));
    assert!(prompt.contains("recursive descent"));
    assert!(
        prompt.contains("memory_read"),
        "an agent that compacts away a decision has to know it can recover it"
    );
}

#[test]
fn a_huge_multibyte_transcript_is_truncated_without_panicking() {
    // The dropped prefix can be larger than the window it was dropped for, so
    // the summariser's own prompt has to be bounded — and the cut must land on
    // a character boundary.
    let evicted: Vec<ChatMessage> = (0..40)
        .map(|_| ChatMessage::assistant("é".repeat(5_000)))
        .collect();

    let prompt = summary_prompt(&evicted);

    assert!(prompt.len() < 60_000, "got {} bytes", prompt.len());
    assert!(prompt.contains("--- end transcript ---"));
}

#[test]
fn the_summary_prompt_survives_an_empty_eviction() {
    let prompt = summary_prompt(&[]);

    assert!(prompt.contains("--- transcript ---"));
}

// ---------------------------------------------------------------------------
// Calibration scale
// ---------------------------------------------------------------------------

#[test]
fn the_calibration_scale_moves_the_estimate_rather_than_the_budget() {
    // Scaling the estimate keeps every figure the plan reports — and so every
    // figure in the run event — in the same units as the operator's configured
    // max_input_tokens.
    let messages = vec![ChatMessage::user("task"), filler("assistant", 3_000)];

    let raw = plan_compaction(&messages, 0, 100_000, 1.0);
    let doubled = plan_compaction(&messages, 0, 100_000, 2.0);

    assert!(
        doubled.before_tokens >= raw.before_tokens * 2 - 2,
        "{} should be about twice {}",
        doubled.before_tokens,
        raw.before_tokens
    );
}

#[test]
fn a_provider_that_counts_more_than_we_estimated_forces_an_earlier_eviction() {
    let messages = vec![ChatMessage::user("task"), filler("assistant", 3_000)];

    assert!(
        plan_compaction(&messages, 0, 1_500, 1.0).is_noop(),
        "uncalibrated, this fits"
    );
    assert!(
        !plan_compaction(&messages, 0, 1_500, 2.0).is_noop(),
        "at twice the measured size it does not"
    );
}

#[test]
fn a_degenerate_scale_falls_back_to_the_raw_estimate() {
    // A provider reporting something absurd must not be able to zero the
    // estimate and wave an oversized request through.
    let messages = vec![ChatMessage::user("task"), filler("assistant", 3_000)];
    let raw = plan_compaction(&messages, 0, 100_000, 1.0).before_tokens;

    for scale in [0.0, -4.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            plan_compaction(&messages, 0, 100_000, scale).before_tokens,
            raw,
            "scale {scale} should have been ignored"
        );
    }
}
