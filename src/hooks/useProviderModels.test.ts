import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { tauriMock } from "../test/tauriMock";
import { PROVIDER_MODELS } from "../types/agent";

let useProviderModels: typeof import("./useProviderModels").useProviderModels;

beforeEach(async () => {
  useProviderModels = (await import("./useProviderModels")).useProviderModels;
});

afterEach(() => {
  vi.restoreAllMocks();
});

function rawModel(overrides: Record<string, unknown> = {}) {
  return {
    value: "claude-opus-5",
    label: "Claude Opus 5",
    priced: true,
    context_window: 1_000_000,
    ...overrides,
  };
}

describe("useProviderModels", () => {
  it("asks for nothing until a provider is chosen", async () => {
    tauriMock.handleAll({ list_provider_models: () => [] });

    const { result } = renderHook(() => useProviderModels(""));

    expect(result.current.models).toEqual([]);
    expect(tauriMock.callCount("list_provider_models")).toBe(0);
  });

  it("shows the built-in list immediately, before the provider answers", async () => {
    // The dropdown has to be usable during the round trip. For every provider
    // but Ollama the two lists are the same in the ordinary case anyway.
    tauriMock.handleAll({ list_provider_models: () => [rawModel()] });

    const { result } = renderHook(() => useProviderModels("anthropic"));

    expect(result.current.models.length).toBe(PROVIDER_MODELS.anthropic.length);
    expect(result.current.isFallback).toBe(true);
  });

  it("replaces the built-in list with the provider's own", async () => {
    tauriMock.handleAll({
      list_provider_models: () => [
        rawModel(),
        rawModel({ value: "claude-sonnet-5", label: "Claude Sonnet 5" }),
      ],
    });

    const { result } = renderHook(() => useProviderModels("anthropic"));

    await waitFor(() => expect(result.current.isFallback).toBe(false));
    expect(result.current.models.map(m => m.value)).toEqual([
      "claude-opus-5",
      "claude-sonnet-5",
    ]);
  });

  it("maps snake_case fields to camelCase", async () => {
    tauriMock.handleAll({
      list_provider_models: () => [rawModel({ priced: false, context_window: 128_000 })],
    });

    const { result } = renderHook(() => useProviderModels("anthropic"));

    await waitFor(() => expect(result.current.isFallback).toBe(false));
    expect(result.current.models[0]).toEqual({
      value: "claude-opus-5",
      label: "Claude Opus 5",
      priced: false,
      contextWindow: 128_000,
    });
  });

  it("keeps the built-in list when the call fails", async () => {
    // An empty dropdown reads as a broken app. The backend falls back on its
    // own side too; this covers the call not coming back at all.
    vi.spyOn(console, "error").mockImplementation(() => {});
    tauriMock.handleAll({
      list_provider_models: () => {
        throw new Error("no settings file");
      },
    });

    const { result } = renderHook(() => useProviderModels("anthropic"));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.models.length).toBe(PROVIDER_MODELS.anthropic.length);
    expect(result.current.isFallback).toBe(true);
  });

  it("keeps the built-in list when the provider answers with nothing", async () => {
    tauriMock.handleAll({ list_provider_models: () => [] });

    const { result } = renderHook(() => useProviderModels("anthropic"));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.models.length).toBe(PROVIDER_MODELS.anthropic.length);
    expect(result.current.isFallback).toBe(true);
  });

  it("does not claim the built-in models are priced", async () => {
    // The tempting shortcut, and wrong: the price table covers Anthropic only,
    // so every DeepSeek and OpenRouter id in the built-in list is unpriced.
    // Claiming otherwise would suppress the warning for exactly the providers
    // that need it. Unknown is the honest answer until the backend replies.
    tauriMock.handleAll({ list_provider_models: () => [] });

    const { result } = renderHook(() => useProviderModels("deepseek"));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.models.every(m => m.priced === null)).toBe(true);
    expect(result.current.models.every(m => m.contextWindow === null)).toBe(true);
  });

  it("refetches when the provider changes", async () => {
    tauriMock.handleAll({ list_provider_models: () => [rawModel()] });

    const { result, rerender } = renderHook(
      ({ provider }: { provider: "anthropic" | "deepseek" }) => useProviderModels(provider),
      { initialProps: { provider: "anthropic" as "anthropic" | "deepseek" } },
    );
    await waitFor(() => expect(result.current.isFallback).toBe(false));

    rerender({ provider: "deepseek" });

    await waitFor(() => expect(tauriMock.callCount("list_provider_models")).toBe(2));
  });
});
