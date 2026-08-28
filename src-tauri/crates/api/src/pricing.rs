// Per-model facts: price rates for run cost estimation, and context windows
// for the input budget.
//
// Both are keyed the same way — by model-id prefix, longest match wins, after
// the same [`normalize`] pass — so they live together rather than growing a
// second copy of that machinery elsewhere. A model added to one table is
// usually a model that needs adding to the other.
//
// `story_runs.estimated_cost_usd` is an *estimate*, and the only way to keep it
// from being a fabrication is to price from a table that either knows a model
// or admits that it does not. `estimate_cost_usd` returns `None` for an unknown
// model so the caller can record real token counts against no cost at all,
// rather than quietly billing them at some other model's rate.
//
// Rates are US dollars per million tokens, from Anthropic's published pricing
// (captured 2026-06-24). Only models with a documented rate belong here —
// guessing a price is worse than declining to quote one.

use crate::types::Usage;

/// Per-million-token rates for one model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    /// Reading a cached prefix. Cheaper than uncached input.
    pub cache_read_per_mtok: f64,
    /// *Writing* a cache entry. More expensive than uncached input — priced
    /// separately, or caching would look like a saving in the wrong direction.
    pub cache_write_per_mtok: f64,
}

impl ModelPrice {
    /// Anthropic prices cache reads at 0.1x and 5-minute cache writes at 1.25x
    /// the uncached input rate, so both derive from it.
    const fn anthropic(input_per_mtok: f64, output_per_mtok: f64) -> Self {
        Self {
            input_per_mtok,
            output_per_mtok,
            cache_read_per_mtok: input_per_mtok * 0.1,
            cache_write_per_mtok: input_per_mtok * 1.25,
        }
    }

    /// Cost in USD of one usage record at these rates.
    pub fn cost_usd(&self, usage: &Usage) -> f64 {
        let per_mtok = |tokens: u64, rate: f64| (tokens as f64 / 1_000_000.0) * rate;
        per_mtok(usage.input_tokens, self.input_per_mtok)
            + per_mtok(usage.output_tokens, self.output_per_mtok)
            + per_mtok(usage.cache_read_input_tokens, self.cache_read_per_mtok)
            + per_mtok(usage.cache_creation_input_tokens, self.cache_write_per_mtok)
    }
}

/// Known models, keyed by an id *prefix*.
///
/// A prefix rather than an exact id because the same model reaches us under
/// several spellings: bare (`claude-opus-5`), dated (`claude-opus-4-5-20251101`),
/// and OpenRouter's vendor-scoped form, which [`normalize`] strips down to the
/// bare id. The longest matching prefix wins, so `claude-opus-4-5` cannot be
/// swallowed by a shorter entry.
const PRICES: &[(&str, ModelPrice)] = &[
    ("claude-fable-5", ModelPrice::anthropic(10.0, 50.0)),
    ("claude-mythos-5", ModelPrice::anthropic(10.0, 50.0)),
    ("claude-opus-5", ModelPrice::anthropic(5.0, 25.0)),
    ("claude-opus-4-8", ModelPrice::anthropic(5.0, 25.0)),
    ("claude-opus-4-7", ModelPrice::anthropic(5.0, 25.0)),
    ("claude-opus-4-6", ModelPrice::anthropic(5.0, 25.0)),
    ("claude-sonnet-5", ModelPrice::anthropic(2.0, 10.0)),
    ("claude-sonnet-4-6", ModelPrice::anthropic(3.0, 15.0)),
    ("claude-haiku-4-5", ModelPrice::anthropic(1.0, 5.0)),
];

/// Context window, in tokens, for the models we know.
///
/// Keyed exactly like [`PRICES`]. Only documented windows belong here —
/// guessing a window is worse than falling back to
/// [`DEFAULT_CONTEXT_WINDOW`], because an invented window that is too large
/// produces the overflow the budget exists to prevent.
///
/// (Anthropic's published windows, captured 2026-06-24.)
const CONTEXT_WINDOWS: &[(&str, u64)] = &[
    ("claude-fable-5", 1_000_000),
    ("claude-mythos-5", 1_000_000),
    ("claude-opus-5", 1_000_000),
    ("claude-opus-4-8", 1_000_000),
    ("claude-opus-4-7", 1_000_000),
    ("claude-opus-4-6", 1_000_000),
    ("claude-sonnet-5", 1_000_000),
    ("claude-sonnet-4-6", 1_000_000),
    ("claude-haiku-4-5", 200_000),
];

/// Window assumed for a model the table does not know — a local Ollama build,
/// a new OpenRouter id, anything unlisted.
///
/// Deliberately small. The cost of guessing low is compacting a conversation
/// that would have fit; the cost of guessing high is the provider rejecting
/// the request and the run dying, which is the failure this exists to prevent.
/// A profile that knows better sets `max_input_tokens` explicitly.
pub const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

/// Longest-matching-prefix lookup over a table keyed by normalised model id.
fn lookup_by_prefix<T: Copy>(table: &[(&str, T)], model: &str) -> Option<T> {
    let id = normalize(model);
    table
        .iter()
        .filter(|(prefix, _)| id.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, value)| *value)
}

/// The model's context window in tokens, or `None` if it is not in the table.
///
/// `None` is the honest answer for an unknown model; callers fall back to
/// [`DEFAULT_CONTEXT_WINDOW`] rather than assuming a generous one.
pub fn context_window(model: &str) -> Option<u64> {
    lookup_by_prefix(CONTEXT_WINDOWS, model)
}

/// The model's context window, or [`DEFAULT_CONTEXT_WINDOW`] when unknown.
pub fn context_window_or_default(model: &str) -> u64 {
    context_window(model).unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Reduce a provider-specific model id to the bare Anthropic-style id.
///
/// OpenRouter scopes ids by vendor (`anthropic/claude-opus-5`) and may append a
/// variant suffix (`:beta`, `:free`); Ollama tags by version (`llama3:8b`).
/// Lower-casing keeps the table case-insensitive.
fn normalize(model: &str) -> String {
    let bare = model.rsplit('/').next().unwrap_or(model);
    let bare = bare.split(':').next().unwrap_or(bare);
    bare.trim().to_ascii_lowercase()
}

/// Look up a model's rates, or `None` if the table does not know it.
pub fn lookup(model: &str) -> Option<ModelPrice> {
    lookup_by_prefix(PRICES, model)
}

/// Estimated USD cost of `usage` on `model`, or `None` when the model is not
/// in the price table.
///
/// `None` is the honest answer for an unknown model and callers are expected to
/// persist it as "no cost recorded" — not as zero dollars spent.
pub fn estimate_cost_usd(model: &str, usage: &Usage) -> Option<f64> {
    lookup(model).map(|price| price.cost_usd(usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_model_prices_input_and_output_at_its_published_rates() {
        // 1M input + 1M output on Opus 5 is $5 + $25.
        let cost = estimate_cost_usd("claude-opus-5", &Usage::new(1_000_000, 1_000_000))
            .expect("opus 5 is in the table");

        assert!((cost - 30.0).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn an_unknown_model_declines_to_quote_a_price() {
        assert_eq!(lookup("some-local-llama"), None);
        assert_eq!(
            estimate_cost_usd("some-local-llama", &Usage::new(1_000, 1_000)),
            None,
            "an unpriced model must not be given another model's rate"
        );
    }

    #[test]
    fn cache_reads_are_cheaper_and_cache_writes_dearer_than_uncached_input() {
        let price = lookup("claude-opus-5").expect("opus 5");
        let tokens = 1_000_000;

        let uncached = price.cost_usd(&Usage { input_tokens: tokens, ..Usage::default() });
        let read = price.cost_usd(&Usage { cache_read_input_tokens: tokens, ..Usage::default() });
        let write =
            price.cost_usd(&Usage { cache_creation_input_tokens: tokens, ..Usage::default() });

        assert!(read < uncached, "cache read {read} should beat uncached {uncached}");
        assert!(write > uncached, "cache write {write} should exceed uncached {uncached}");
    }

    #[test]
    fn openrouter_vendor_prefixes_and_variant_suffixes_resolve_to_the_same_model() {
        let bare = lookup("claude-sonnet-5").expect("bare id");

        assert_eq!(lookup("anthropic/claude-sonnet-5"), Some(bare));
        assert_eq!(lookup("anthropic/claude-sonnet-5:beta"), Some(bare));
        assert_eq!(lookup("CLAUDE-SONNET-5"), Some(bare));
    }

    #[test]
    fn a_dated_model_id_matches_its_undated_prefix() {
        assert_eq!(
            lookup("claude-opus-4-6-20260101"),
            lookup("claude-opus-4-6"),
            "a dated snapshot is the same model at the same price"
        );
    }

    #[test]
    fn the_longest_matching_prefix_wins() {
        // "claude-sonnet-5" and "claude-sonnet-4-6" are priced differently and
        // neither may capture the other.
        let sonnet_5 = lookup("claude-sonnet-5").expect("sonnet 5");
        let sonnet_4_6 = lookup("claude-sonnet-4-6").expect("sonnet 4.6");

        assert_ne!(sonnet_5.input_per_mtok, sonnet_4_6.input_per_mtok);
        assert_eq!(lookup("claude-sonnet-4-6-20260101"), Some(sonnet_4_6));
    }

    #[test]
    fn zero_usage_on_a_known_model_costs_nothing() {
        assert_eq!(
            estimate_cost_usd("claude-opus-5", &Usage::default()),
            Some(0.0)
        );
    }

    #[test]
    fn an_empty_model_id_is_unknown_rather_than_a_partial_match() {
        assert_eq!(lookup(""), None);
    }

    #[test]
    fn a_known_model_reports_its_published_context_window() {
        assert_eq!(context_window("claude-opus-5"), Some(1_000_000));
        assert_eq!(context_window("claude-haiku-4-5"), Some(200_000));
    }

    #[test]
    fn context_windows_share_the_price_tables_id_normalisation() {
        let bare = context_window("claude-sonnet-5").expect("sonnet 5");

        assert_eq!(context_window("anthropic/claude-sonnet-5:beta"), Some(bare));
        assert_eq!(context_window("CLAUDE-SONNET-5-20260101"), Some(bare));
    }

    #[test]
    fn an_unknown_model_falls_back_to_the_conservative_default_window() {
        assert_eq!(context_window("some-local-llama"), None);
        assert_eq!(
            context_window_or_default("some-local-llama"),
            DEFAULT_CONTEXT_WINDOW
        );
        // The fallback must undershoot the models we do know, not overshoot:
        // guessing a window too large is what sends the oversized request.
        let known = context_window("claude-haiku-4-5").expect("haiku 4.5");
        assert!(DEFAULT_CONTEXT_WINDOW < known);
    }

    #[test]
    fn every_priced_model_also_has_a_context_window() {
        // The two tables are keyed identically and are meant to be maintained
        // together; a model priced but unwindowed silently budgets at the
        // conservative default instead of its real capacity.
        for (prefix, _) in PRICES {
            assert!(
                context_window(prefix).is_some(),
                "'{prefix}' is priced but has no context window"
            );
        }
    }
}
