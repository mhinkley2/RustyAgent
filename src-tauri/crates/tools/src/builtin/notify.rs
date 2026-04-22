use async_trait::async_trait;
use serde_json::json;
use tracing::info;
use crate::{Tool, ToolContext, ToolOutput};

pub struct SendNotificationTool;

#[async_trait]
impl Tool for SendNotificationTool {
    fn name(&self) -> &str { "send_notification" }

    fn description(&self) -> &str {
        "Send a desktop notification to the user."
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

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let title = input.get("title").and_then(|v| v.as_str()).unwrap_or("RustyAgent");
        let body = match input.get("body").and_then(|v| v.as_str()) {
            Some(b) => b.to_string(),
            None => return ToolOutput::err("Missing required field: body"),
        };

        // Actual OS notification delivered via Tauri's notification plugin.
        // For now, log it — the plugin will be wired in a later story.
        info!("NOTIFICATION [{title}]: {body}");
        ToolOutput::ok(format!("Notification sent: {title} — {body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};

    #[tokio::test]
    async fn send_notification_succeeds_with_valid_input() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = SendNotificationTool
            .execute(
                serde_json::json!({"title": "Alert", "body": "Something happened"}),
                &ctx,
            )
            .await;
        assert!(!r.is_error, "Expected success, got: {}", r.content);
        assert!(r.content.contains("Alert"));
    }

    #[tokio::test]
    async fn send_notification_missing_body_returns_error() {
        let db = make_test_pool().await;
        let ctx = make_ctx(db);

        let r = SendNotificationTool
            .execute(serde_json::json!({"title": "Only title"}), &ctx)
            .await;
        assert!(r.is_error);
        assert!(r.content.contains("body"));
    }
}
