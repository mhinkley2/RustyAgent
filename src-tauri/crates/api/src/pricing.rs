// Per-model price table for run cost estimation.
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
    let id = normalize(model);
    PRICES
        .iter()
        .filter(|(prefix, _)| id.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, price)| *price)
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
}
