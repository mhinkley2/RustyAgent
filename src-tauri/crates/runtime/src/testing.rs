//! Test harness for the agent loop.
//!
//! `ConversationRuntime` emits through the [`EventSink`] trait rather than a
//! concrete `tauri::AppHandle`, so a test can substitute [`RecordingSink`] and
//! assert the exact event sequence a run produced — no Tauri app, no window.

use std::sync::{Arc, Mutex};

use crate::runtime::{EventSink, RunEvent};

/// An [`EventSink`] that records everything emitted through it.
#[derive(Clone, Default)]
pub struct RecordingSink {
    emitted: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap in an `Arc` for handing to `ConversationRuntime`. The clone still
    /// observes everything, since the log is shared.
    pub fn handle(&self) -> Arc<dyn EventSink> {
        Arc::new(self.clone())
    }

    /// Every `(event_name, payload)` pair, in emission order.
    pub fn all(&self) -> Vec<(String, serde_json::Value)> {
        self.emitted.lock().expect("sink poisoned").clone()
    }

    /// Payloads emitted under one event name.
    pub fn payloads(&self, name: &str) -> Vec<serde_json::Value> {
        self.all()
            .into_iter()
            .filter(|(n, _)| n == name)
            .map(|(_, p)| p)
            .collect()
    }

    /// `run-event` payloads decoded back into [`RunEvent`].
    pub fn run_events(&self) -> Vec<RunEvent> {
        self.payloads("run-event")
            .into_iter()
            .map(|p| {
                serde_json::from_value(p.clone())
                    .unwrap_or_else(|e| panic!("undecodable run-event {p}: {e}"))
            })
            .collect()
    }

    /// The `type` tag of each run event, e.g. `["token", "token", "complete"]`.
    ///
    /// Lets a test assert the shape of a run without pinning every field.
    pub fn kinds(&self) -> Vec<String> {
        self.payloads("run-event")
            .into_iter()
            .filter_map(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
            .collect()
    }

    /// Concatenated content of every `token` event — the assistant's full reply.
    pub fn text(&self) -> String {
        self.run_events()
            .into_iter()
            .filter_map(|e| match e {
                RunEvent::Token { content, .. } => Some(content),
                _ => None,
            })
            .collect()
    }

    /// How many times an event name was emitted.
    pub fn count(&self, name: &str) -> usize {
        self.payloads(name).len()
    }
}

impl EventSink for RecordingSink {
    fn emit_event(&self, name: &str, payload: serde_json::Value) {
        self.emitted
            .lock()
            .expect("sink poisoned")
            .push((name.to_string(), payload));
    }
}
