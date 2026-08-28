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
}

export const DEFAULT_SETTINGS: AppSettings = {
  anthropic_api_key: null,
  openrouter_api_key: null,
  deepseek_api_key: null,
  ollama_base_url: null,
};
