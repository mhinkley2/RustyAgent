// Mirrors commands/src/settings.rs AppSettings struct.

export interface AppSettings {
  anthropic_api_key: string | null;
  openrouter_api_key: string | null;
  deepseek_api_key: string | null;
  /** Ollama base URL. Defaults to http://localhost:11434 when null. */
  ollama_base_url: string | null;
}

export const DEFAULT_SETTINGS: AppSettings = {
  anthropic_api_key: null,
  openrouter_api_key: null,
  deepseek_api_key: null,
  ollama_base_url: null,
};
