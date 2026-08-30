use async_trait::async_trait;
use serde_json::json;
use tracing::{info, warn};
use crate::{NotificationCategory, Tool, ToolContext, ToolOutput};

pub struct SendNotificationTool;

#[async_trait]
impl Tool for SendNotificationTool {
    fn name(&self) -> &str { "send_notification" }

    fn description(&self) -> &str {
        "Send a desktop notification to the user. Returns an error if it could not be \
         delivered — a successful result means the notification was handed to the OS, so \
         only then may you assume the user has been alerted."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Notification title" },
                "body":  { "type": "string", "description": "Notification body text" }
            },
            "required": ["title", "body"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("RustyAgent");
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(b) => b.to_string(),
            None => return ToolOutput::err("Missing required field: body"),
        };

        // No sink means no desktop to deliver to — the stdio MCP binary, or a
        // test. Say so rather than returning ok: a model told the user was
        // notified will go on to reason as though the user knows.
        //
        // The body is logged here and not on the success path below, which is
        // the opposite of how it reads at first glance: nothing else recorded
        // it, because nothing delivered it, so this line is the only trace of
        // what the agent tried to say.
        let Some(notifier) = ctx.notifier.as_ref() else {
            warn!("send_notification called with no notification sink [{title}]: {body}");
            return ToolOutput::err(
                "Notification NOT delivered: no desktop notification sink is available in \
                 this context. The user has not been alerted. Do not assume they have seen \
                 this; put anything important in your final response instead.",
            );
        };

        match notifier.notify(NotificationCategory::Agent, title, &body).await {
            Ok(()) => {
                // Title only. The body has just been handed to the OS and
                // shown to the user; repeating it here would leave a durable
                // copy, in a log file that outlives the toast, of text the
                // user saw once and the model chose freely.
                info!(body_len = body.len(), "Notification delivered [{title}]");
                // The body is not echoed back. The model supplied it one turn
                // ago and `run_events.tool_input` already holds it verbatim,
                // so repeating it in the result buys no information and costs
                // context on every notification an agent sends.
                ToolOutput::ok(format!("Notification delivered: {title}"))
            }
            Err(reason) => {
                warn!("Notification delivery failed [{title}]: {reason}");
                ToolOutput::err(format!(
                    "Notification NOT delivered: {reason} The user has not been alerted. \
                     Do not retry; put anything important in your final response instead."
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};
    use crate::Notifier;
    use std::sync::{Arc, Mutex};

    /// Records what it was asked to deliver, and can be told to fail.
    struct FakeNotifier {
        outcome: Result<(), String>,
        seen: Mutex<Vec<(NotificationCategory, String, String)>>,
    }

    impl FakeNotifier {
        fn ok() -> Arc<Self> {
            Arc::new(Self { outcome: Ok(()), seen: Mutex::new(Vec::new()) })
        }
        fn failing(reason: &str) -> Arc<Self> {
            Arc::new(Self { outcome: Err(reason.to_string()), seen: Mutex::new(Vec::new()) })
        }
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        async fn notify(
            &self,
            category: NotificationCategory,
            title: &str,
            body: &str,
        ) -> Result<(), String> {
            self.seen
                .lock()
                .unwrap()
                .push((category, title.to_string(), body.to_string()));
            self.outcome.clone()
        }
    }

    #[tokio::test]
    async fn send_notification_delivers_through_the_sink() {
        let db = make_test_pool().await;
        let mut ctx = make_ctx(db);
        let sink = FakeNotifier::ok();
        ctx.notifier = Some(sink.clone());

        let r = SendNotificationTool
            .execute(
                serde_json::json!({"title": "Alert", "body": "Something happened"}),
                &ctx,
            )
            .await;

        assert!(!r.is_error, "Expected success, got: {}", r.content);
        let seen = sink.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, NotificationCategory::Agent);
        assert_eq!(seen[0].1, "Alert");
        assert_eq!(seen[0].2, "Something happened");
    }

    /// The defect this tool used to have: it logged, returned ok, and the
    /// model acted as though the user had been alerted.
    #[tokio::test]
    async fn send_notification_without_a_sink_reports_failure() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = SendNotificationTool
            .execute(
                serde_json::json!({"title": "Alert", "body": "Something happened"}),
                &ctx,
            )
            .await;

        assert!(r.is_error, "A notification that went nowhere must not read as sent");
        assert!(r.content.contains("NOT delivered"), "got: {}", r.content);
    }

    #[tokio::test]
    async fn send_notification_surfaces_the_delivery_failure_reason() {
        let db = make_test_pool().await;
        let mut ctx = make_ctx(db);
        ctx.notifier = Some(FakeNotifier::failing("Permission was refused by the OS."));

        let r = SendNotificationTool
            .execute(serde_json::json!({"title": "Alert", "body": "Body"}), &ctx)
            .await;

        assert!(r.is_error);
        assert!(r.content.contains("Permission was refused"), "got: {}", r.content);
    }

    #[tokio::test]
    async fn send_notification_missing_body_returns_error() {
        let db = make_test_pool().await;
        let mut ctx = make_ctx(db);
        ctx.notifier = Some(FakeNotifier::ok());

        let r = SendNotificationTool
            .execute(serde_json::json!({"title": "Only title"}), &ctx)
            .await;

        assert!(r.is_error);
        assert!(r.content.contains("body"));
    }
}
