use async_trait::async_trait;
use serde_json::json;

use crate::{Tool, ToolContext, ToolOutput};

/// How deep spawning may go.
///
/// One, because one is what the engine can do. A spawned child is built with
/// `spawn_subtask: None` (`pipeline/src/lib.rs`, "No further spawn_subtask
/// recursion") to break a circular closure that would otherwise make the future
/// `!Send` — so a child has no spawning tool at all, and the depth counter can
/// never reach two.
///
/// This said 5. The guard below could therefore never fire, and the limit an
/// agent read in the refusal message described a capability that did not exist.
/// A ceiling nothing can reach is not a safety margin; it is a false claim
/// about the system, and the one place an agent would look to find out.
///
/// Raising it is an async architecture change — a spawn broker or a trait
/// object in place of the captured closure — not a change to this number.
const MAX_PIPELINE_DEPTH: u32 = 1;

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
         Returns the new run_id — poll it with get_run to see whether the subtask has \
         finished and how it ended. Use memory_read/memory_write with \
         scope=shared_scratchpad to exchange data with it. A subtask cannot spawn \
         subtasks of its own."
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

        // Depth guard. Reachable now that the limit is the real one: a subtask
        // that finds this tool in its registry is told why it cannot use it,
        // rather than being handed the misleading "not available in this
        // context" that a missing callback produces.
        if ctx.pipeline_depth >= MAX_PIPELINE_DEPTH {
            return ToolOutput::err(format!(
                "Spawn depth limit ({MAX_PIPELINE_DEPTH}) reached — a subtask cannot spawn \
                 subtasks of its own. Do this work here, or return it to the run that \
                 spawned you."
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
                "message": "Subtask spawned. Poll get_run with this run_id to see when it \
                            finishes; the story's own status depends on the subtask agent \
                            choosing to set it, and on a workspace setting, so the run is \
                            the reliable one."
            })).unwrap_or_default()),
            Err(e) => ToolOutput::err(format!("Failed to spawn subtask: {e}")),
        }
    }
}
