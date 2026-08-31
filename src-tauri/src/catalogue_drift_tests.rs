//! The frontend's model catalogue must agree with what the app can account for.
//!
//! This is the third of the drift incidents named in [`crate::version_drift_tests`]
//! and the only one that had shipped a user-visible defect: the profile editor
//! offered `claude-3-5-sonnet-20241022`, `claude-3-opus-20240229`,
//! `claude-3-5-haiku-20241022` and `claude-haiku-3-5` — three retired and one
//! that was never a valid id — and every one of them would have failed on first
//! use.
//!
//! Fetching the catalogue from the provider fixes *which models exist*. It
//! cannot fix pricing, because pricing is in nobody's models API, so `PRICES`
//! and `CONTEXT_WINDOWS` stay hand-maintained. What is left to check is that
//! the built-in fallback — still shipped, still hand-written, and still what a
//! user sees before a key is configured or when the API is unreachable — names
//! only models the app can cost and budget.
//!
//! Reading TypeScript from a Rust test is not elegant. It is what is available:
//! the two catalogues are in different languages and different build systems,
//! and the alternative on offer is another comment asking the next person to
//! remember. A comment is what failed here.

use std::path::PathBuf;

/// The repository root, one level above `src-tauri/`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

/// Every model id the frontend offers for one provider.
///
/// Parses `PROVIDER_MODELS` by finding the provider's key and reading
/// `value: "..."` up to the closing bracket. Deliberately narrow: a shape it
/// does not understand yields nothing, and the emptiness check below turns that
/// into a failure rather than a silent pass.
fn frontend_models(source: &str, provider: &str) -> Vec<String> {
    let catalogue = source
        .split_once("export const PROVIDER_MODELS")
        .map(|(_, rest)| rest)
        .unwrap_or("");

    let Some(start) = catalogue.find(&format!("{provider}: [")) else {
        return Vec::new();
    };
    let block = &catalogue[start..];
    let end = block.find(']').unwrap_or(block.len());
    let block = &block[..end];

    block
        .match_indices("value:")
        .filter_map(|(at, _)| {
            let rest = &block[at..];
            let open = rest.find('"')?;
            let after = &rest[open + 1..];
            let close = after.find('"')?;
            Some(after[..close].to_string())
        })
        .collect()
}

fn agent_ts() -> String {
    let path = repo_root().join("src").join("types").join("agent.ts");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

#[test]
fn the_parser_finds_the_catalogue_it_is_checking() {
    // Without this the checks below pass vacuously the moment `agent.ts` is
    // reformatted — which is the failure mode of every test that parses another
    // language's source.
    let models = frontend_models(&agent_ts(), "anthropic");

    assert!(
        models.len() >= 3,
        "parsed {models:?} from PROVIDER_MODELS.anthropic — the format has moved and this \
         file's parser needs updating, not deleting",
    );
    assert!(
        models.iter().all(|id| id.starts_with("claude-")),
        "parsed something that is not a model id: {models:?}",
    );
}

#[test]
fn every_model_the_editor_offers_can_be_priced() {
    // An unpriced model still runs and records no cost, so this fails quietly
    // in production: a run that cost real money reports zero, and nothing
    // anywhere says why.
    let unpriced: Vec<String> = frontend_models(&agent_ts(), "anthropic")
        .into_iter()
        .filter(|id| api::pricing::lookup(id).is_none())
        .collect();

    assert!(
        unpriced.is_empty(),
        "the profile editor offers models with no entry in PRICES: {unpriced:?}. Add them to \
         crates/api/src/pricing.rs, or stop offering them.",
    );
}

#[test]
fn every_model_the_editor_offers_has_a_real_context_window() {
    // A model missing from CONTEXT_WINDOWS budgets at the conservative default
    // instead of its real window, so its conversations compact far earlier than
    // they need to — slower, more expensive, and invisible.
    let unbudgeted: Vec<String> = frontend_models(&agent_ts(), "anthropic")
        .into_iter()
        .filter(|id| api::pricing::context_window(id).is_none())
        .collect();

    assert!(
        unbudgeted.is_empty(),
        "the profile editor offers models with no entry in CONTEXT_WINDOWS: {unbudgeted:?}. \
         They would budget at the default rather than their real window.",
    );
}

#[test]
fn the_offline_fallback_offers_the_same_models_as_the_editor() {
    // Two hand-written lists remain — the frontend's, and the Rust fallback
    // used before a key is configured or when the API is unreachable. They are
    // the same list for the same user, so a disagreement means the dropdown
    // changes under someone the moment a key is entered.
    let frontend = frontend_models(&agent_ts(), "anthropic");

    let mut missing: Vec<&String> = Vec::new();
    for id in &frontend {
        // The fallback is not public, so this asks the question the user's
        // machine would: what does the provider hand back with no API to reach?
        if !api::anthropic_fallback_models().contains(&id.as_str()) {
            missing.push(id);
        }
    }

    assert!(
        missing.is_empty(),
        "the editor offers models the offline fallback does not: {missing:?}",
    );
    assert_eq!(
        frontend.len(),
        api::anthropic_fallback_models().len(),
        "the two hand-written catalogues have drifted apart",
    );
}

/// Providers whose every offered model is in the price table.
///
/// Anthropic alone, today. Not an oversight to fix in passing — inventing rates
/// for the others would be worse than recording none, and a wrong price is a
/// wrong invoice.
const PRICED_PROVIDERS: [&str; 1] = ["anthropic"];

/// Providers whose runs currently report no cost at all.
const UNPRICED_PROVIDERS: [&str; 2] = ["deepseek", "openrouter"];

#[test]
fn the_providers_the_app_cannot_price_are_the_ones_recorded_here() {
    // This is a gap, written down. Every DeepSeek and OpenRouter run records
    // zero cost today — not an estimate, zero — and nothing in the app says so
    // except the editor's warning, which relies on `pricing` genuinely not
    // knowing them.
    //
    // It is asserted in both directions on purpose. Adding rates for one of
    // these providers should fail this test and send the author here to move
    // its name, rather than leaving two stale lists behind.
    let source = agent_ts();

    for provider in PRICED_PROVIDERS {
        let unpriced: Vec<String> = frontend_models(&source, provider)
            .into_iter()
            .filter(|id| api::pricing::lookup(id).is_none())
            .collect();
        assert!(
            unpriced.is_empty(),
            "{provider} is listed as priced but offers {unpriced:?}, which PRICES does not know",
        );
    }

    for provider in UNPRICED_PROVIDERS {
        let models = frontend_models(&source, provider);
        assert!(
            !models.is_empty(),
            "no models parsed for {provider} — the parser or the catalogue has moved",
        );
        let priced: Vec<String> = models
            .into_iter()
            .filter(|id| api::pricing::lookup(id).is_some())
            .collect();
        assert!(
            priced.is_empty(),
            "{provider} now has rates for {priced:?}. Move it to PRICED_PROVIDERS — its runs              will start reporting real costs, and the editor will stop warning about them.",
        );
    }
}
