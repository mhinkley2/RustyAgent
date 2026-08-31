//! The model catalogue the profile editor offers.
//!
//! Four hand-maintained lists had to agree and only two were cross-checked; the
//! drift shipped retired ids into the editor once already. The provider is the
//! source of truth for *which models exist* now — but not for what they cost,
//! because pricing is not in anyone's models API. So this joins the two: the
//! provider says what exists, `api::pricing` says what the app can account for,
//! and a model the app cannot price is marked rather than quietly billed at
//! zero.

use serde::{Deserialize, Serialize};

/// One selectable model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelOption {
    /// The id sent to the provider.
    pub value: String,
    /// What to show in a dropdown.
    pub label: String,
    /// Whether `api::pricing` can cost a run on this model.
    ///
    /// An unpriced model still runs — it records no cost, and budgets its
    /// context at the conservative default rather than its real window. That
    /// is a quiet degradation of two things a user reads later, so the editor
    /// says so at the point of choosing.
    pub priced: bool,
    /// The context window the app will budget with, in tokens.
    ///
    /// The value actually used, not the model's real window: an unlisted model
    /// reports the conservative default, which is what the run will honour.
    pub context_window: u64,
}

impl ModelOption {
    fn new(value: String, label: String) -> Self {
        let priced = api::pricing::lookup(&value).is_some();
        let context_window = api::pricing::context_window_or_default(&value);
        Self {
            value,
            label,
            priced,
            context_window,
        }
    }
}

/// The models a provider offers, as the editor should show them.
///
/// Never an error. A provider that cannot be reached, or has no key configured
/// yet, still yields its built-in list — see `AnthropicClient::catalogue`. An
/// empty dropdown reads as a broken app, and "no key yet" is the state every
/// user starts in.
pub async fn list_provider_models(
    provider: Box<dyn api::LlmProvider>,
) -> Vec<ModelOption> {
    let ids = provider.list_models().await.unwrap_or_default();
    ids.into_iter()
        .map(|id| {
            let label = pretty_label(&id);
            ModelOption::new(id, label)
        })
        .collect()
}

/// A readable name for a model id.
///
/// Providers vary: Anthropic's Models API carries a `display_name`, Ollama
/// returns bare tags, OpenRouter returns vendor-scoped ids. The trait flattens
/// all of that to a `Vec<String>`, so this reconstructs something presentable
/// rather than showing a raw id — and it is deliberately dumb, because a wrong
/// guess here costs a slightly ugly dropdown entry and nothing else.
fn pretty_label(id: &str) -> String {
    // OpenRouter's `vendor/model` — the vendor is worth keeping, so only the
    // model half is prettified.
    let (prefix, bare) = match id.split_once('/') {
        Some((vendor, rest)) => (format!("{vendor}/"), rest),
        None => (String::new(), id),
    };

    // A dated Anthropic id ends in an 8-digit stamp that means nothing to a
    // reader choosing from a list.
    let bare = bare
        .rsplit_once('-')
        .filter(|(_, tail)| tail.len() == 8 && tail.chars().all(|c| c.is_ascii_digit()))
        .map_or(bare, |(head, _)| head);

    let words: Vec<String> = bare
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                // Leave anything with a digit alone: "4o" must not become "4O",
                // and "3.3" and "70b" are versions, not words.
                Some(first) if part.chars().any(|c| c.is_ascii_digit()) => {
                    let mut out = String::new();
                    out.push(first);
                    out.push_str(chars.as_str());
                    out
                }
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();

    format!("{prefix}{}", words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anthropic_id_reads_as_a_name() {
        assert_eq!(pretty_label("claude-opus-5"), "Claude Opus 5");
        assert_eq!(pretty_label("claude-haiku-4-5"), "Claude Haiku 4 5");
    }

    #[test]
    fn a_dated_id_drops_the_date() {
        // The stamp means nothing to someone choosing from a list, and it is
        // the difference between a readable dropdown and a wall of digits.
        assert_eq!(pretty_label("claude-opus-4-5-20251101"), "Claude Opus 4 5");
    }

    #[test]
    fn something_that_only_looks_like_a_date_is_kept() {
        // Eight digits, or it is part of the name.
        assert_eq!(pretty_label("model-2025"), "Model 2025");
    }

    #[test]
    fn a_vendor_scoped_id_keeps_its_vendor() {
        assert_eq!(pretty_label("openai/gpt-4o"), "openai/Gpt 4o");
        assert_eq!(pretty_label("meta-llama/llama-3.3-70b-instruct"),
                   "meta-llama/Llama 3.3 70b Instruct");
    }

    #[test]
    fn a_known_model_is_priced_and_carries_its_real_window() {
        let option = ModelOption::new("claude-opus-5".into(), "Claude Opus 5".into());

        assert!(option.priced);
        assert_eq!(option.context_window, 1_000_000);
    }

    #[test]
    fn an_unknown_model_is_marked_unpriced_and_budgets_conservatively() {
        // What the editor warns about: it still runs, but records no cost and
        // budgets at the default rather than whatever its real window is.
        let option = ModelOption::new("some-local-build".into(), "Local".into());

        assert!(!option.priced);
        assert_eq!(option.context_window, api::pricing::DEFAULT_CONTEXT_WINDOW);
    }

    #[test]
    fn a_dated_id_is_still_priced_by_its_prefix() {
        // `pricing` matches on a prefix precisely so a dated id resolves. If
        // that ever stopped holding, every fetched Anthropic id would show as
        // unpriced the moment the API started returning dated forms.
        let option = ModelOption::new("claude-opus-5-20251101".into(), "x".into());

        assert!(option.priced);
    }
}
