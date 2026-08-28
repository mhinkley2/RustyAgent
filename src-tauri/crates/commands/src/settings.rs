// Application settings stored in {app_data_dir}/settings.json.
// Replaces OS keychain for API key storage — simple, portable, user-editable.

use db::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

// ---------------------------------------------------------------------------
// AppSettings — the on-disk schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    /// Anthropic API key (sk-ant-…)
    pub anthropic_api_key: Option<String>,
    /// OpenRouter API key (sk-or-…)
    pub openrouter_api_key: Option<String>,
    /// DeepSeek API key
    pub deepseek_api_key: Option<String>,
    /// Ollama base URL. Defaults to http://localhost:11434 when None.
    pub ollama_base_url: Option<String>,
    /// Number of most-recent runs per story whose events are retained.
    /// Older run_events are pruned after each run completes. Default: 10.
    pub event_retention_runs: Option<u32>,
    /// How many steps of a parallel pipeline may execute at once.
    ///
    /// Each in-flight step is a full checkout on disk and its own stream of
    /// provider calls, so this is a spend and disk ceiling as much as a
    /// concurrency one. A per-workspace override in `workspace_settings` takes
    /// precedence; `pipeline::DEFAULT_MAX_PARALLEL_STEPS` applies when neither
    /// is set.
    pub max_parallel_steps: Option<u32>,
}

impl AppSettings {
    /// Canonical path for the settings file.
    pub fn settings_path(app: &AppHandle) -> PathBuf {
        app.path()
            .app_data_dir()
            .expect("Failed to resolve app data directory")
            .join("settings.json")
    }

    /// Load from disk; returns `Default` if the file is missing or invalid.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist to disk, creating parent directories as needed.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create settings directory: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("Cannot write settings.json: {e}"))
    }

    /// Convenience: strip empty strings to None.
    pub fn normalize(mut self) -> Self {
        if self.anthropic_api_key.as_deref().map(str::is_empty).unwrap_or(false) {
            self.anthropic_api_key = None;
        }
        if self.openrouter_api_key.as_deref().map(str::is_empty).unwrap_or(false) {
            self.openrouter_api_key = None;
        }
        if self.deepseek_api_key.as_deref().map(str::is_empty).unwrap_or(false) {
            self.deepseek_api_key = None;
        }
        if self.ollama_base_url.as_deref().map(str::is_empty).unwrap_or(false) {
            self.ollama_base_url = None;
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Tauri command functions
// ---------------------------------------------------------------------------

pub async fn get_settings(app: AppHandle) -> Result<AppSettings, String> {
    let path = AppSettings::settings_path(&app);
    Ok(AppSettings::load_from(&path))
}

pub async fn save_settings(settings: AppSettings, app: AppHandle) -> Result<(), String> {
    let path = AppSettings::settings_path(&app);
    settings.normalize().save_to(&path)
}

/// Load the per-workspace settings override JSON for the given workspace.
/// Returns an empty JSON object `{}` if no override exists.
pub async fn get_workspace_settings(
    workspace_id: String,
    db: &DbPool,
) -> Result<serde_json::Value, String> {
    let row = sqlx::query(
        "SELECT settings_json FROM workspace_settings WHERE workspace_id = ?"
    )
    .bind(&workspace_id)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    match row {
        Some(r) => {
            let json_str: String = r.try_get("settings_json").unwrap_or_else(|_| "{}".into());
            serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))
        }
        None => Ok(serde_json::Value::Object(Default::default())),
    }
}

/// Persist per-workspace settings overrides.
pub async fn save_workspace_settings(
    workspace_id: String,
    overrides: serde_json::Value,
    db: &DbPool,
) -> Result<(), String> {
    let json = serde_json::to_string(&overrides)
        .map_err(|e| format!("Serialize error: {e}"))?;
    sqlx::query(
        "INSERT INTO workspace_settings (workspace_id, settings_json)
         VALUES (?, ?)
         ON CONFLICT(workspace_id) DO UPDATE SET
           settings_json = excluded.settings_json,
           updated_at    = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')"
    )
    .bind(&workspace_id)
    .bind(&json)
    .execute(db)
    .await
    .map_err(|e| format!("DB error: {e}"))?;
    Ok(())
}
