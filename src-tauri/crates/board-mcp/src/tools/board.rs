//! Board mutations that don't already exist as agent tools.
//!
//! The six story CRUD tools are registered directly from `tools::builtin::story`
//! via `register_agent_tool` — see `tools/mod.rs`.

use serde_json::{json, Value};
use tools::ToolOutput;

use crate::{mcp_tool, registry::json_ok};

mcp_tool! {
    pub ReorderStoriesTool,
    name        = "reorder_stories",
    description = "Set the sort order of several stories at once, in a single transaction. \
                   Use after moving cards to persist the new column order.",
    mutation    = true,
    schema      = {
        "type": "object",
        "properties": {
            "updates": {
                "type": "array",
                "description": "Story ids paired with their new sort order",
                "items": {
                    "type": "object",
                    "properties": {
                        "id":         { "type": "string" },
                        "sort_order": { "type": "integer" }
                    },
                    "required": ["id", "sort_order"]
                }
            }
        },
        "required": ["updates"]
    },
    |input, ctx| {
        let Some(raw) = input.get("updates").and_then(Value::as_array) else {
            return ToolOutput::err("Missing required field: updates (array)");
        };

        let mut updates = Vec::with_capacity(raw.len());
        for entry in raw {
            let (Some(id), Some(sort_order)) = (
                entry.get("id").and_then(Value::as_str),
                entry.get("sort_order").and_then(Value::as_i64),
            ) else {
                return ToolOutput::err(
                    "Each update needs a string 'id' and an integer 'sort_order'",
                );
            };
            updates.push(commands::StoryOrderUpdate {
                id: id.to_string(),
                sort_order,
            });
        }

        let count = updates.len();
        match commands::stories::batch_update_story_order(updates, &ctx.db).await {
            Ok(()) => json_ok(json!({ "updated": count })),
            Err(error) => ToolOutput::err(error),
        }
    }
}
