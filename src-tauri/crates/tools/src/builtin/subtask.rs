use async_trait::async_trait;
use serde_json::json;

use crate::{Tool, ToolContext, ToolOutput};

const MAX_PIPELINE_DEPTH: u32 = 5;

/// Built-in tool that lets an orchestrator agent spawn a subtask (a new agent
/// run on a given story) as part of a pipeline chain.
///
/// The actual firing logic is injected via `ToolContext::spawn_subtask` so that
/// the `tools` crate does not need to depend on the `pipeline` or `runtime`
/// crates directly.
pub struct SpawnSubtaskTool;

#[async_trait]
impl Tool for SpawnSubtaskTool {
    fn name(&self) -> &str { "spawn_subtask" }

    fn description(&self) -> &str {
        "Spawn a subtask: run an agent on a story as a child task of the current pipeline. \
         Returns the new run_id. The parent pipeline must poll get_story or use \
         memory_read/memory_write with scope=shared_scratchpad to exchange results."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "story_id": {
                    "type": "string",
                    "description": "UUID of the story to run"
                },
                "agent_id": {
                    "type": "string",
                    "description": "UUID of the agent profile to execute the story"
                }
            },
            "required": ["story_id", "agent_id"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let story_id = match input.get("story_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => return ToolOutput::err("Missing required field: story_id"),
        };
        let agent_id = match input.get("agent_id").and_then(|v| v.as_str()) {
            Some(a) => a.to_string(),
            None => return ToolOutput::err("Missing required field: agent_id"),
        };

        // Depth guard
        if ctx.pipeline_depth >= MAX_PIPELINE_DEPTH {
            return ToolOutput::err(format!(
                "Pipeline depth limit ({MAX_PIPELINE_DEPTH}) reached — cannot spawn further subtasks"
            ));
        }

        let pipeline_run_id = match &ctx.pipeline_run_id {
            Some(id) => id.clone(),
            None => {
                // Allow spawn_subtask outside a pipeline by treating the current run as root.
                ctx.run_id.clone()
            }
        };

        let spawn_fn = match &ctx.spawn_subtask {
            Some(f) => f.clone(),
            None => return ToolOutput::err("spawn_subtask is not available in this context"),
        };

        match spawn_fn(
            story_id.clone(),
            agent_id.clone(),
            pipeline_run_id,
            ctx.pipeline_depth + 1,
            ctx.workspace_root.clone(),
        )
        .await
        {
            Ok(run_id) => ToolOutput::ok(serde_json::to_string(&json!({
                "run_id": run_id,
                "story_id": story_id,
                "agent_id": agent_id,
                "message": "Subtask spawned — use get_story to poll progress"
            })).unwrap_or_default()),
            Err(e) => ToolOutput::err(format!("Failed to spawn subtask: {e}")),
        }
    }
}
