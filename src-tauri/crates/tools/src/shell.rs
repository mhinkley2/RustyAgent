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

/// Split a command string into tokens, honouring single and double quotes.
///
/// Quotes group whitespace into a single argument and are removed from the
/// result, so `sh -c "exit 3"` yields `["sh", "-c", "exit 3"]` rather than
/// splitting the script across two arguments.
///
/// Backslash is **not** an escape character. This is deliberate: `working_dir`
/// and `command` routinely carry Windows paths like `C:\tools\build.exe`, and
/// treating `\` as an escape would silently eat the separators. Quoting is the
/// only grouping mechanism.
///
/// An unterminated quote contributes the text accumulated so far rather than
/// raising a parse error — the resulting exec failure names the real program
/// and is more useful than a parse error with no context.
fn split_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;

    for ch in command.chars() {
        match quote {
            // Closing quote.
            Some(q) if ch == q => quote = None,
            // Inside a quote every character is literal, whitespace included.
            Some(_) => current.push(ch),
            // Opening quote. Marks a token even if the quoted text is empty,
            // so `""` survives as a deliberate empty argument.
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                in_token = true;
            }
            None if ch.is_whitespace() => {
                if in_token {
                    out.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None => {
                current.push(ch);
                in_token = true;
            }
        }
    }

    if in_token {
        out.push(current);
    }
    out
}

impl ShellCommandTool {
    /// Split the command string into (program, args) without invoking a shell.
    /// Quote-aware; see [`split_command`].
    fn argv(&self) -> Option<(String, Vec<String>)> {
        let mut parts = split_command(&self.command);
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

/// Cap tool output at `MAX_OUTPUT_BYTES` to prevent context flooding.
///
/// The cut must land on a UTF-8 character boundary: slicing a `String` at a
/// fixed byte offset panics whenever that offset falls mid-codepoint, which any
/// command emitting non-ASCII output can trigger.
fn truncate_output(combined: String) -> String {
    if combined.len() <= MAX_OUTPUT_BYTES {
        return combined;
    }

    let mut cut = MAX_OUTPUT_BYTES;
    while cut > 0 && !combined.is_char_boundary(cut) {
        cut -= 1;
    }

    format!(
        "{}\n\n[Output truncated at {} KB. {} bytes omitted.]",
        &combined[..cut],
        MAX_OUTPUT_BYTES / 1024,
        combined.len() - cut
    )
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

                let truncated = truncate_output(combined);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};

    fn tool(command: &str) -> ShellCommandTool {
        ShellCommandTool {
            id: "tool-1".into(),
            tool_name: "run_it".into(),
            tool_description: "runs it".into(),
            command: command.into(),
            working_dir: ".".into(),
            timeout_secs: 30,
        }
    }

    // -- output truncation --------------------------------------------------

    #[test]
    fn output_under_the_cap_is_returned_verbatim() {
        let out = truncate_output("short output".to_string());
        assert_eq!(out, "short output");
    }

    #[test]
    fn output_exactly_at_the_cap_is_not_truncated() {
        let payload = "a".repeat(MAX_OUTPUT_BYTES);
        assert_eq!(truncate_output(payload.clone()), payload);
    }

    #[test]
    fn output_over_the_cap_is_truncated_with_a_notice() {
        let payload = "a".repeat(MAX_OUTPUT_BYTES + 500);
        let out = truncate_output(payload);

        assert!(out.contains("[Output truncated at 32 KB."));
        assert!(out.contains("500 bytes omitted."));
        assert!(out.starts_with(&"a".repeat(100)));
    }

    #[test]
    fn output_over_32kb_is_truncated_without_panicking() {
        // Regression: the cut was a fixed byte offset into a `String`, which
        // panics when it lands mid-codepoint. "é" is two bytes, so a payload of
        // them guarantees the boundary at MAX_OUTPUT_BYTES splits a character.
        let payload = "é".repeat(MAX_OUTPUT_BYTES);
        assert!(!payload.is_char_boundary(MAX_OUTPUT_BYTES + 1));

        let out = truncate_output(payload);

        assert!(out.contains("[Output truncated at 32 KB."));
        // The kept prefix must still be valid UTF-8 made only of whole chars.
        let kept = out.split("\n\n[Output truncated").next().unwrap();
        assert!(kept.chars().all(|c| c == 'é'), "prefix was corrupted");
    }

    #[test]
    fn a_multibyte_char_straddling_the_boundary_is_dropped_whole() {
        // One ASCII byte short of the cap, then a 4-byte emoji straddling it.
        let mut payload = "a".repeat(MAX_OUTPUT_BYTES - 1);
        payload.push('🦀');
        payload.push_str("trailing");

        let out = truncate_output(payload);
        let kept = out.split("\n\n[Output truncated").next().unwrap();

        assert_eq!(kept.len(), MAX_OUTPUT_BYTES - 1);
        assert!(!kept.contains('🦀'), "partial emoji must not be kept");
    }

    // -- argv splitting ------------------------------------------------------

    #[test]
    fn argv_splits_program_from_arguments() {
        let (program, args) = tool("cargo test --workspace").argv().expect("argv");
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["test", "--workspace"]);
    }

    #[test]
    fn argv_collapses_repeated_whitespace() {
        let (program, args) = tool("  npm   run    build  ").argv().expect("argv");
        assert_eq!(program, "npm");
        assert_eq!(args, vec!["run", "build"]);
    }

    #[test]
    fn argv_on_a_bare_program_yields_no_arguments() {
        let (program, args) = tool("ls").argv().expect("argv");
        assert_eq!(program, "ls");
        assert!(args.is_empty());
    }

    #[test]
    fn argv_on_an_empty_or_blank_command_is_none() {
        assert!(tool("").argv().is_none());
        assert!(tool("   \t  ").argv().is_none());
    }

    #[test]
    fn argv_keeps_a_double_quoted_argument_whole() {
        let (program, args) = tool("sh -c \"exit 3\"").argv().expect("argv");
        assert_eq!(program, "sh");
        assert_eq!(args, vec!["-c", "exit 3"]);
    }

    #[test]
    fn argv_keeps_a_single_quoted_argument_whole() {
        let (program, args) = tool("git commit -m 'a message'").argv().expect("argv");
        assert_eq!(program, "git");
        assert_eq!(args, vec!["commit", "-m", "a message"]);
    }

    #[test]
    fn argv_treats_the_other_quote_as_a_literal_inside_a_quote() {
        let (_, args) = tool("echo \"it's fine\"").argv().expect("argv");
        assert_eq!(args, vec!["it's fine"]);
    }

    /// Backslash is a path separator, not an escape — a Windows path must
    /// survive splitting byte for byte.
    #[test]
    fn argv_preserves_backslashes_in_windows_paths() {
        let (program, args) = tool("C:\\tools\\build.exe --out C:\\a b\\out.txt")
            .argv()
            .expect("argv");
        assert_eq!(program, "C:\\tools\\build.exe");
        assert_eq!(args, vec!["--out", "C:\\a", "b\\out.txt"]);

        let (_, quoted) = tool("build.exe \"C:\\a b\\out.txt\"").argv().expect("argv");
        assert_eq!(quoted, vec!["C:\\a b\\out.txt"]);
    }

    #[test]
    fn argv_keeps_an_explicitly_empty_argument() {
        let (program, args) = tool("prog \"\" next").argv().expect("argv");
        assert_eq!(program, "prog");
        assert_eq!(args, vec!["", "next"]);
    }

    #[test]
    fn argv_on_an_unterminated_quote_keeps_what_it_has() {
        let (program, args) = tool("sh -c \"exit 3").argv().expect("argv");
        assert_eq!(program, "sh");
        assert_eq!(args, vec!["-c", "exit 3"]);
    }

    #[tokio::test]
    async fn an_empty_command_returns_an_error_instead_of_executing() {
        let ctx = make_ctx(make_test_pool().await);
        let out = tool("").execute(json!({}), &ctx).await;

        assert!(out.is_error);
        assert!(
            out.content.contains("empty command"),
            "got {:?}",
            out.content
        );
    }

    // -- working directory ---------------------------------------------------

    #[test]
    fn a_dot_working_dir_resolves_to_the_workspace_root() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = ".".into();

        assert_eq!(t.resolve_working_dir(Some(&root)), root);
    }

    #[test]
    fn an_empty_working_dir_resolves_to_the_workspace_root() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = String::new();

        assert_eq!(t.resolve_working_dir(Some(&root)), root);
    }

    #[test]
    fn a_relative_working_dir_is_joined_onto_the_workspace_root() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = "crates/api".into();

        assert_eq!(
            t.resolve_working_dir(Some(&root)),
            PathBuf::from("/workspace/proj").join("crates/api")
        );
    }

    #[test]
    fn an_absolute_working_dir_overrides_the_workspace_root() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = if cfg!(windows) {
            "C:\\elsewhere".into()
        } else {
            "/elsewhere".into()
        };

        assert_eq!(
            t.resolve_working_dir(Some(&root)),
            PathBuf::from(&t.working_dir)
        );
    }

    // -- process outcomes ----------------------------------------------------

    /// A command that exits non-zero on either platform.
    fn failing_command() -> &'static str {
        if cfg!(windows) {
            "cmd /c exit 3"
        } else {
            "sh -c \"exit 3\""
        }
    }

    #[tokio::test]
    async fn a_successful_command_with_no_output_reports_success() {
        let ctx = make_ctx(make_test_pool().await);
        let cmd = if cfg!(windows) {
            "cmd /c rem"
        } else {
            "true"
        };

        let out = tool(cmd).execute(json!({}), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert!(out.content.contains("completed successfully"));
    }

    #[tokio::test]
    async fn a_nonzero_exit_is_an_error_carrying_the_exit_code() {
        let ctx = make_ctx(make_test_pool().await);
        let out = tool(failing_command()).execute(json!({}), &ctx).await;

        assert!(out.is_error);
        assert!(
            out.content.contains("exit code 3"),
            "got {:?}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_missing_program_is_an_error_not_a_panic() {
        let ctx = make_ctx(make_test_pool().await);
        let out = tool("definitely-not-a-real-program-xyz")
            .execute(json!({}), &ctx)
            .await;

        assert!(out.is_error);
        assert!(
            out.content.contains("Failed to start"),
            "got {:?}",
            out.content
        );
    }

    #[tokio::test]
    async fn a_command_exceeding_its_timeout_is_reported_as_a_timeout() {
        let ctx = make_ctx(make_test_pool().await);
        let mut t = tool(if cfg!(windows) {
            // ping is the portable "sleep" on Windows without PowerShell.
            "cmd /c ping -n 6 127.0.0.1"
        } else {
            "sleep 5"
        });
        t.timeout_secs = 1;

        let out = t.execute(json!({}), &ctx).await;

        assert!(out.is_error);
        assert!(
            out.content.contains("timed out after 1 seconds"),
            "got {:?}",
            out.content
        );
    }

    // -- loader --------------------------------------------------------------

    async fn seed_tool(db: &db::DbPool, id: &str, name: &str, timeout_secs: i64) {
        sqlx::query(
            "INSERT INTO custom_tools (id, name, description, command, working_dir, timeout_secs)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind("seeded")
        .bind("echo hi")
        .bind(".")
        .bind(timeout_secs)
        .execute(db)
        .await
        .expect("seed custom_tools");
    }

    /// `agent_custom_tool_bindings` is a composite-PK junction table — no id column.
    async fn bind_tool(db: &db::DbPool, profile_id: &str, tool_id: &str) {
        sqlx::query(
            "INSERT INTO agent_custom_tool_bindings (agent_profile_id, custom_tool_id)
             VALUES (?, ?)",
        )
        .bind(profile_id)
        .bind(tool_id)
        .execute(db)
        .await
        .expect("seed binding");
    }

    #[tokio::test]
    async fn load_for_agent_returns_only_bound_tools_ordered_by_name() {
        let db = make_test_pool().await;
        seed_tool(&db, "t-zeta", "zeta", 10).await;
        seed_tool(&db, "t-alpha", "alpha", 10).await;
        seed_tool(&db, "t-other", "other", 10).await;

        bind_tool(&db, "agent-1", "t-zeta").await;
        bind_tool(&db, "agent-1", "t-alpha").await;
        bind_tool(&db, "agent-2", "t-other").await;

        let loaded = load_for_agent("agent-1", &db).await.expect("load");

        let names: Vec<_> = loaded.iter().map(|t| t.tool_name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[tokio::test]
    async fn load_for_agent_with_no_bindings_returns_empty() {
        let db = make_test_pool().await;
        seed_tool(&db, "t-1", "alpha", 10).await;

        let loaded = load_for_agent("agent-nobody", &db).await.expect("load");

        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn load_for_agent_carries_through_the_stored_timeout() {
        let db = make_test_pool().await;
        seed_tool(&db, "t-1", "alpha", 90).await;
        bind_tool(&db, "agent-1", "t-1").await;

        let loaded = load_for_agent("agent-1", &db).await.expect("load");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].timeout_secs, 90);
    }

    #[tokio::test]
    async fn an_omitted_timeout_falls_back_to_the_schema_default_of_30() {
        // timeout_secs is NOT NULL DEFAULT 30, so the column can never be NULL —
        // the default comes from the schema, not from load_for_agent's unwrap_or.
        let db = make_test_pool().await;
        sqlx::query(
            "INSERT INTO custom_tools (id, name, description, command) VALUES (?, ?, ?, ?)",
        )
        .bind("t-1")
        .bind("alpha")
        .bind("seeded")
        .bind("echo hi")
        .execute(&db)
        .await
        .expect("seed custom_tools");
        bind_tool(&db, "agent-1", "t-1").await;

        let loaded = load_for_agent("agent-1", &db).await.expect("load");

        assert_eq!(loaded[0].timeout_secs, 30);
        assert_eq!(loaded[0].working_dir, ".");
    }
}
