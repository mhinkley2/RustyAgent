// Mirrors commands/src/settings.rs AppSettings struct.

export interface AppSettings {
  anthropic_api_key: string | null;
  openrouter_api_key: string | null;
  deepseek_api_key: string | null;
  /** Ollama base URL. Defaults to http://localhost:11434 when null. */
  ollama_base_url: string | null;
  /**
   * Backend-only knobs with no control in the Settings UI. They are carried
   * through this type so that saving settings preserves rather than erases
   * whatever the user put in settings.json by hand.
   */
  event_retention_runs?: number | null;
  /** Steps of a parallel pipeline allowed to run at once. */
  max_parallel_steps?: number | null;
  /** Which notifications the app is allowed to raise. */
  notifications?: NotificationSettings;
  /**
   * Seconds a gated tool call waits for a decision. `null` waits indefinitely,
   * so a run parks until the user comes back rather than failing the call.
   */
  approval_timeout_secs?: number | null;
}

/**
 * Mirrors `tools::NotificationSettings`.
 *
 * Serialized camelCase by the backend, unlike the snake_case rest of
 * `AppSettings` — the Rust type carries `rename_all = "camelCase"`.
 */
export interface NotificationSettings {
  /** Master switch. Off suppresses every category. */
  enabled: boolean;
  /** A gated tool call is waiting on you. */
  onApproval: boolean;
  /** A run ended in failure. */
  onRunFailed: boolean;
  /** A run finished successfully. */
  onRunCompleted: boolean;
  /** An agent asked to notify you itself. */
  onAgentRequest: boolean;
}

export const DEFAULT_NOTIFICATIONS: NotificationSettings = {
  enabled: true,
  onApproval: true,
  onRunFailed: true,
  onRunCompleted: true,
  onAgentRequest: true,
};

export const DEFAULT_SETTINGS: AppSettings = {
  anthropic_api_key: null,
  openrouter_api_key: null,
  deepseek_api_key: null,
  ollama_base_url: null,
  notifications: DEFAULT_NOTIFICATIONS,
  approval_timeout_secs: null,
};
