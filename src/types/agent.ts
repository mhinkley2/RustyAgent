// Agent profile types — mirrors the Rust AgentProfile struct and DB schema.

export type Provider = "anthropic" | "openrouter" | "deepseek" | "ollama";
export type RunMode = "manual" | "continuous" | "scheduled";
export type ContextStrategy = "recent" | "summary" | "full";

export interface AgentProfile {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string;
  provider: Provider;
  model: string;
  context_strategy: ContextStrategy;
  persistent_memory: boolean;
  max_input_tokens: number | null;
  max_output_tokens: number | null;
  run_mode: RunMode;
  cron_expression: string | null;
  continuous_poll_interval_secs: number;
  max_iterations: number;
  scope: "global" | "workspace";
  toml_path: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateProfileInput {
  name: string;
  description?: string | null;
  system_prompt?: string;
  provider: Provider;
  model: string;
  context_strategy?: ContextStrategy;
  persistent_memory?: boolean;
  max_input_tokens?: number | null;
  max_output_tokens?: number | null;
  run_mode?: RunMode;
  cron_expression?: string | null;
  continuous_poll_interval_secs?: number;
  max_iterations?: number;
  scope?: "global" | "workspace";
}

export type UpdateProfileInput = Partial<Omit<CreateProfileInput, "name">> & { name?: string };

// ---------------------------------------------------------------------------
// Agent runtime status (scheduler)
// ---------------------------------------------------------------------------

export interface AgentRuntimeStatus {
  profileId: string;
  /** Scheduler mode config: "manual" | "continuous" | "scheduled" */
  schedulerMode: "manual" | "continuous" | "scheduled";
  /**
   * Live runtime state:
   * "idle" | "checking_for_work" | "running_story" | "waiting_for_approval"
   * | "waiting_for_human_input" | "failed" | "completed_recently"
   */
  state:
    | "idle"
    | "checking_for_work"
    | "running_story"
    | "waiting_for_approval"
    | "waiting_for_human_input"
    | "failed"
    | "completed_recently";
  /** Plain-language label from backend for consistent UI copy. */
  stateLabel: string;
  /** ISO-8601 next scheduled fire time (scheduled mode only) */
  nextRunAt: string | null;
  /** run_id of the currently-executing run */
  activeRunId: string | null;
  /** Current story metadata when applicable. */
  activeStoryId: string | null;
  activeStoryTitle: string | null;
  /** Short failure summary when state=failed. */
  failureSummary: string | null;
}

// ---------------------------------------------------------------------------
// Provider → model catalogue
// ---------------------------------------------------------------------------

export const PROVIDER_MODELS: Record<Provider, { value: string; label: string }[]> = {
  anthropic: [
    { value: "claude-opus-4-5",              label: "Claude Opus 4.5" },
    { value: "claude-3-5-sonnet-20241022",   label: "Claude 3.5 Sonnet" },
    { value: "claude-3-5-haiku-20241022",    label: "Claude 3.5 Haiku" },
    { value: "claude-3-opus-20240229",       label: "Claude 3 Opus" },
  ],
  deepseek: [
    { value: "deepseek-chat",     label: "DeepSeek Chat" },
    { value: "deepseek-reasoner", label: "DeepSeek Reasoner" },
  ],
  openrouter: [
    { value: "openai/gpt-4o",               label: "GPT-4o" },
    { value: "openai/gpt-4o-mini",          label: "GPT-4o Mini" },
    { value: "openai/o3-mini",              label: "o3 Mini" },
    { value: "google/gemini-2.5-pro",       label: "Gemini 2.5 Pro" },
    { value: "google/gemini-2.5-flash",     label: "Gemini 2.5 Flash" },
    { value: "meta-llama/llama-3.3-70b-instruct", label: "Llama 3.3 70B" },
  ],
  // Ollama: user types in the model name — handled as free-text
  ollama: [],
};

export const PROVIDER_LABELS: Record<Provider, string> = {
  anthropic:  "Anthropic",
  openrouter: "OpenRouter",
  deepseek:   "DeepSeek",
  ollama:     "Ollama (local)",
};

export const CONTEXT_STRATEGY_OPTIONS: { value: ContextStrategy; label: string; description: string }[] = [
  { value: "recent",  label: "Recent",  description: "Keep the most recent messages" },
  { value: "summary", label: "Summary", description: "Periodically summarise older context" },
  { value: "full",    label: "Full",    description: "Send all messages every time (expensive)" },
];
