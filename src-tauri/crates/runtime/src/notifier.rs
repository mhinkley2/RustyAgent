//! Delivering an OS notification from inside a run.
//!
//! The `tools` crate defines [`tools::Notifier`] but cannot implement it: it is
//! linked by the standalone `rustyagent-board-mcp` binary, which has no Tauri
//! app to deliver through. This is the desktop implementation, and it lives in
//! `runtime` because every crate that builds a `ConversationRuntime` —
//! `commands`, `pipeline`, `scheduler` — already depends on this one and
//! already holds an `AppHandle`.

use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tools::{NotificationCategory, NotificationSettings, Notifier};
use tracing::warn;

/// The slice of `commands::AppSettings` a run needs in order to survive being
/// left alone.
///
/// Only the fields this crate reads. `serde` ignores the rest of the file, so
/// the two definitions do not have to be kept in step — beyond these key
/// names, which a test in `commands::settings` asserts, because nothing else
/// would fail if they were renamed there: the user's preferences would simply
/// stop being read.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct UnattendedSettings {
    pub notifications: NotificationSettings,
    pub approval_timeout_secs: Option<u64>,
}

impl UnattendedSettings {
    /// How long a gated tool call should wait; `None` waits indefinitely.
    pub fn approval_timeout(&self) -> Option<std::time::Duration> {
        tools::approval_timeout_from_secs(self.approval_timeout_secs)
    }
}

/// Read the unattended-run preferences out of `settings.json`.
///
/// Read per use rather than cached, so turning notifications off takes effect
/// on the run already in flight — which is the run the user is most likely to
/// be turning them off because of. Reads are rare (a parked approval, a
/// finished run, the start of a run), so the file access is not a cost worth
/// engineering around.
pub fn unattended_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> UnattendedSettings {
    let Some(dir) = db::paths::with_override(app.path().app_data_dir().ok()) else {
        return UnattendedSettings::default();
    };
    std::fs::read_to_string(dir.join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<UnattendedSettings>(&s).ok())
        .unwrap_or_default()
}

/// Delivers through the Tauri notification plugin, honouring the user's
/// per-category preferences.
pub struct AppNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> AppNotifier<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }

    /// Build one ready to hand to `ConversationRuntime::notifier`.
    pub fn arc(app: tauri::AppHandle<R>) -> Arc<dyn Notifier> {
        Arc::new(Self::new(app))
    }

    /// The user's notification preferences, as of this delivery.
    fn settings(&self) -> NotificationSettings {
        unattended_settings(&self.app).notifications
    }
}

#[async_trait]
impl<R: tauri::Runtime> Notifier for AppNotifier<R> {
    async fn notify(
        &self,
        category: NotificationCategory,
        title: &str,
        body: &str,
    ) -> Result<(), String> {
        if !self.settings().allows(category) {
            return Err(
                "Notifications for this category are turned off in Settings.".to_string()
            );
        }

        // Granted unconditionally on desktop, where notification permission is
        // not a runtime concept; the check is here for mobile and for the case
        // where a platform starts refusing. Refusal must not break the run, so
        // it comes back as an error string rather than a panic or a retry.
        match self.app.notification().permission_state() {
            Ok(PermissionState::Granted) => {}
            Ok(_) => match self.app.notification().request_permission() {
                Ok(PermissionState::Granted) => {}
                Ok(state) => {
                    return Err(format!(
                        "The operating system has not granted notification permission \
                         (state: {state:?})."
                    ))
                }
                Err(e) => return Err(format!("Could not request notification permission: {e}")),
            },
            Err(e) => return Err(format!("Could not read notification permission: {e}")),
        }

        // `show()` is synchronous and talks to the platform's notification
        // service, so it does not belong on the async runtime's worker.
        let app = self.app.clone();
        let title = title.to_string();
        let body = body.to_string();
        let sent = tokio::task::spawn_blocking(move || {
            app.notification().builder().title(title).body(body).show()
        })
        .await;

        match sent {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("The operating system rejected the notification: {e}")),
            Err(e) => {
                warn!("Notification task panicked: {e}");
                Err("The notification could not be sent.".to_string())
            }
        }
    }
}
