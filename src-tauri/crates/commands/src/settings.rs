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

    /// Which notifications the user wants delivered.
    ///
    /// The type is `tools::NotificationSettings` rather than a local copy
    /// because `runtime::AppNotifier` reads this same field back out of
    /// `settings.json` to decide whether to deliver.
    #[serde(default)]
    pub notifications: tools::NotificationSettings,

    /// How long a gated tool call waits for a decision, in seconds.
    ///
    /// `None` — the default — waits indefinitely, so an unattended run parks
    /// until the user comes back instead of failing the call five minutes
    /// after they leave. A value is for anyone who would rather a run end than
    /// sit parked; expiry is recorded as `expired`, never as a rejection.
    pub approval_timeout_secs: Option<u64>,
}

impl AppSettings {
    /// Canonical path for the settings file.
    ///
    /// Goes through `db::paths` so `RUSTYAGENT_DATA_DIR` moves settings along
    /// with the database it configures; a half-moved data directory is worse
    /// than none.
    pub fn settings_path(app: &AppHandle) -> PathBuf {
        db::paths::with_override(app.path().app_data_dir().ok())
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

    /// How long the approval gate should wait, as a `Duration`.
    ///
    /// `None` means wait indefinitely. Zero is treated as "no wait configured"
    /// rather than "expire immediately": a zero-second gate would deny every
    /// gated call the instant it was raised, which is a footgun with no use
    /// case, and almost certainly a cleared input field.
    pub fn approval_timeout(&self) -> Option<std::time::Duration> {
        tools::approval_timeout_from_secs(self.approval_timeout_secs)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `runtime::AppNotifier` reads `settings.json` back with its own struct
    /// carrying only a `notifications` field, because `runtime` cannot depend
    /// on this crate. That makes the *key name* a contract between the two,
    /// and nothing else would fail if it were renamed here — the notifier
    /// would simply fall back to defaults and quietly ignore the user's
    /// preferences.
    #[test]
    fn notification_settings_serialize_under_the_key_the_notifier_reads() {
        let json = serde_json::to_value(AppSettings::default()).expect("serialize");
        let notifications = json
            .get("notifications")
            .expect("AppSettings must expose notification preferences as `notifications`");
        assert_eq!(notifications["enabled"], serde_json::json!(true));
        assert_eq!(notifications["onApproval"], serde_json::json!(true));
    }

    /// A settings file written before notifications existed must not read as
    /// "the user turned everything off".
    #[test]
    fn settings_without_notifications_default_to_on() {
        let settings: AppSettings =
            serde_json::from_str(r#"{"anthropic_api_key":"sk-ant-x"}"#).expect("parse");
        assert!(settings.notifications.enabled);
        assert!(settings.notifications.allows(tools::NotificationCategory::Approval));
    }

    #[test]
    fn approval_timeout_defaults_to_waiting_indefinitely() {
        assert_eq!(AppSettings::default().approval_timeout(), None);
    }

    #[test]
    fn zero_approval_timeout_is_read_as_unset_not_as_instant_expiry() {
        let settings = AppSettings { approval_timeout_secs: Some(0), ..Default::default() };
        assert_eq!(settings.approval_timeout(), None);
    }

    #[test]
    fn a_configured_approval_timeout_is_honoured() {
        let settings = AppSettings { approval_timeout_secs: Some(900), ..Default::default() };
        assert_eq!(settings.approval_timeout(), Some(std::time::Duration::from_secs(900)));
    }
}
