import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { PROVIDER_MODELS } from "../types/agent";
import type { Provider } from "../types/agent";

/** One selectable model, as the backend describes it. */
export interface ModelOption {
  value: string;
  label: string;
  /**
   * Whether the app can cost a run on this model.
   *
   * An unpriced model still runs. It records no cost and budgets its context at
   * the conservative default rather than its real window — two things a user
   * reads later, degrading quietly. The editor says so at the point of choosing.
   */
  priced: boolean;
  /** The window the app will budget with, in tokens — not necessarily the model's. */
  contextWindow: number;
}

interface RawModelOption {
  value: string;
  label: string;
  priced: boolean;
  context_window: number;
}

/**
 * The static catalogue, as `ModelOption`s.
 *
 * Everything in `PROVIDER_MODELS` is priced by construction — a drift test
 * fails the build otherwise — so the fallback claims so rather than warning
 * about models it has no reason to doubt. `contextWindow` is 0 to mean "not
 * known here"; the fallback is used precisely when the backend has not
 * answered, and inventing a number would be worse than admitting that.
 */
function staticOptions(provider: Provider): ModelOption[] {
  return PROVIDER_MODELS[provider].map(m => ({
    value: m.value,
    label: m.label,
    priced: true,
    contextWindow: 0,
  }));
}

export interface UseProviderModelsReturn {
  models: ModelOption[];
  loading: boolean;
  /** True while showing the built-in list because the backend has not answered. */
  isFallback: boolean;
}

/**
 * The models a provider offers.
 *
 * Asks the backend, which asks the provider — so a retired id stops being
 * offered when the provider stops listing it, rather than when someone
 * remembers to edit three files.
 *
 * Falls back to the built-in catalogue rather than showing nothing. The backend
 * already falls back on its own side; this covers the case where the call
 * itself does not come back at all.
 */
export function useProviderModels(provider: Provider | ""): UseProviderModelsReturn {
  const [models, setModels] = useState<ModelOption[]>([]);
  const [loading, setLoading] = useState(false);
  const [isFallback, setIsFallback] = useState(false);

  useEffect(() => {
    if (!provider) {
      setModels([]);
      setIsFallback(false);
      setLoading(false);
      return;
    }

    // A provider switched while a fetch is in flight must not have the previous
    // provider's models land on top of it.
    let cancelled = false;
    const fallback = staticOptions(provider);

    setLoading(true);
    // Shown immediately rather than after the round trip: the dropdown is
    // usable while the real answer arrives, and for every provider but Ollama
    // the two lists are the same in the ordinary case.
    setModels(fallback);
    setIsFallback(true);

    invoke<RawModelOption[]>("list_provider_models", { provider })
      .then(raw => {
        if (cancelled) return;
        if (raw.length === 0) return; // keep the fallback rather than emptying
        setModels(
          raw.map(m => ({
            value: m.value,
            label: m.label,
            priced: m.priced,
            contextWindow: m.context_window,
          })),
        );
        setIsFallback(false);
      })
      .catch(e => {
        if (cancelled) return;
        // Not a toast. The dropdown is populated and usable; a modal error
        // about a list the user can already see would be noise.
        console.error("Could not fetch the model catalogue:", e);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [provider]);

  return { models, loading, isFallback };
}
