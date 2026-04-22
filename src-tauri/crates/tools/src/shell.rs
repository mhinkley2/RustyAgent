// Shell command tool — executes a pre-defined shell command when called by an agent.
//
// Security model:
// - The command string is split into program + args at definition time.
// - The agent can only call the tool by name — it supplies NO free-form input.
// - shell=false: we use Command::new(program).args([...]) — no shell injection possible.
// - Output is capped at MAX_OUTPUT_BYTES (32 KB) to prevent context flooding.
// - Execution includes a configurable timeout (default 30 s).

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::Row;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

use crate::{Tool, ToolContext, ToolOutput};

const MAX_OUTPUT_BYTES: usize = 32 * 1024; // 32 KB

/// A user-defined tool that runs a fixed shell command.
///
/// The agent cannot change the command — it can only invoke it by name.
#[derive(Debug, Clone)]
pub struct ShellCommandTool {
    pub id: String,
    pub tool_name: String,
    pub tool_description: String,
    /// Raw command string, e.g. "cargo test --workspace". Split into argv at construction time.
    pub command: String,
    /// Directory relative to workspace root where the command will be executed (default: ".").
    pub working_dir: String,
    pub timeout_secs: u64,
}

impl ShellCommandTool {
    /// Split the command string into (program, args) without invoking a shell.
    /// Uses simple whitespace splitting — sufficient for pre-defined commands.
    fn argv(&self) -> Option<(String, Vec<String>)> {
        let mut parts: Vec<String> = self
            .command
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        if parts.is_empty() {
            return None;
        }
        let program = parts.remove(0);
        Some((program, parts))
    }

    /// Resolve the working directory against the workspace root.
    fn resolve_working_dir(&self, workspace_root: Option<&PathBuf>) -> PathBuf {
        let rel = Path::new(&self.working_dir);
        if rel == Path::new(".") || self.working_dir.is_empty() {
            workspace_root
                .cloned()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        } else if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            workspace_root
                .map(|r| r.join(rel))
                .unwrap_or_else(|| rel.to_path_buf())
        }
    }
}

#[async_trait]
impl Tool for ShellCommandTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    /// No agent-supplied parameters — the command is fully defined by the user.
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> ToolOutput {
        let (program, args) = match self.argv() {
            Some(v) => v,
            None => return ToolOutput::err("Custom tool has an empty command — cannot execute."),
        };

        let cwd = self.resolve_working_dir(ctx.workspace_root.as_ref());

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Never allow the process to read from stdin.
            .stdin(std::process::Stdio::null())
            // Ensure env is inherited (no extra injection from us).
            ;

        // Spawn and apply timeout.
        let result = timeout(
            Duration::from_secs(self.timeout_secs),
            async {
                let child = cmd.spawn().map_err(|e| {
                    format!("Failed to start '{}': {e}", program)
                })?;
                child.wait_with_output().await.map_err(|e| {
                    format!("Process wait error: {e}")
                })
            },
        )
        .await;

        match result {
            Err(_elapsed) => ToolOutput::err(format!(
                "Command timed out after {} seconds: {}",
                self.timeout_secs, self.command
            )),
            Ok(Err(spawn_err)) => ToolOutput::err(spawn_err),
            Ok(Ok(output)) => {
                let mut combined = String::new();

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !stdout.is_empty() {
                    combined.push_str("stdout:\n");
                    combined.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str("stderr:\n");
                    combined.push_str(&stderr);
                }

                // Cap output to prevent context flooding.
                let truncated = if combined.len() > MAX_OUTPUT_BYTES {
                    format!(
                        "{}\n\n[Output truncated at {} KB. {} bytes omitted.]",
                        &combined[..MAX_OUTPUT_BYTES],
                        MAX_OUTPUT_BYTES / 1024,
                        combined.len() - MAX_OUTPUT_BYTES
                    )
                } else {
                    combined
                };

                let exit_code = output.status.code().unwrap_or(-1);
                if output.status.success() {
                    let content = if truncated.is_empty() {
                        format!("Command completed successfully (exit code 0): {}", self.command)
                    } else {
                        truncated
                    };
                    ToolOutput::ok(content)
                } else {
                    let content = if truncated.is_empty() {
                        format!(
                            "Command failed (exit code {}): {}",
                            exit_code, self.command
                        )
                    } else {
                        format!(
                            "Command failed (exit code {}):\n{}",
                            exit_code, truncated
                        )
                    };
                    ToolOutput::err(content)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Loader — query DB and return ready-to-register ShellCommandTools
// ---------------------------------------------------------------------------

/// Load all custom shell command tools bound to a given agent profile from the DB.
/// Returns a `Vec<ShellCommandTool>` ready to be registered into a `ToolRegistry`.
pub async fn load_for_agent(
    agent_profile_id: &str,
    db: &db::DbPool,
) -> anyhow::Result<Vec<ShellCommandTool>> {
    let rows = sqlx::query(
        "SELECT ct.id, ct.name, ct.description, ct.command, ct.working_dir, ct.timeout_secs
         FROM custom_tools ct
         INNER JOIN agent_custom_tool_bindings actb ON actb.custom_tool_id = ct.id
         WHERE actb.agent_profile_id = ?
         ORDER BY ct.name ASC",
    )
    .bind(agent_profile_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| ShellCommandTool {
            id:               row.try_get("id").unwrap_or_default(),
            tool_name:        row.try_get("name").unwrap_or_default(),
            tool_description: row.try_get("description").unwrap_or_default(),
            command:          row.try_get("command").unwrap_or_default(),
            working_dir:      row.try_get("working_dir").unwrap_or_else(|_| ".".to_string()),
            timeout_secs:     row.try_get::<i64, _>("timeout_secs").unwrap_or(30) as u64,
        })
        .collect())
}
