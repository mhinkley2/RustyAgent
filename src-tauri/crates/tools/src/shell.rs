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

use crate::paths::{is_within, resolve_existing_prefix, resolve_for_containment};
use crate::{Tool, ToolContext, ToolOutput, ToolPermissionInfo};

const MAX_OUTPUT_BYTES: usize = 32 * 1024; // 32 KB

/// Environment variables that carry an LLM provider credential.
///
/// A custom tool inherits the parent environment, which is what makes
/// `cargo test` behave the way it does in a terminal. That inheritance also
/// hands every command the agent can invoke any provider key the operator
/// happened to export, so these names are removed before the process starts.
///
/// This is not full containment and should not be sold as such: RustyAgent
/// stores its own provider keys in the settings file, and a shell command with
/// read access to the disk can still read them. It closes the exported-key
/// case, which costs nothing to close.
const PROVIDER_CREDENTIAL_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "DEEPSEEK_API_KEY",
];

/// Strip provider credentials from a command's environment.
fn scrub_provider_credentials(cmd: &mut Command) {
    for name in PROVIDER_CREDENTIAL_ENV_VARS {
        cmd.env_remove(name);
    }
}

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

    /// Resolve the working directory against the workspace root, rejecting any
    /// result that lands outside it.
    ///
    /// This used to honour an absolute `working_dir` verbatim, which made the
    /// workspace root advisory: a custom tool could name any directory on the
    /// machine and its command would run there. A `..` chain in a relative
    /// `working_dir` did the same thing more quietly. Both are now refused.
    ///
    /// With no workspace root there is nothing to be outside of, so the path is
    /// taken as given. That is the "no workspace open" case, not a bypass.
    fn resolve_working_dir(
        &self,
        workspace_root: Option<&PathBuf>,
    ) -> Result<PathBuf, String> {
        let is_default = self.working_dir.is_empty()
            || Path::new(&self.working_dir) == Path::new(".");

        let Some(root) = workspace_root else {
            return Ok(if is_default {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                PathBuf::from(&self.working_dir)
            });
        };

        let canonical_root = resolve_existing_prefix(root);
        if is_default {
            return Ok(canonical_root);
        }

        let resolved = resolve_for_containment(&self.working_dir, Some(root));
        if !is_within(&resolved, &canonical_root) {
            return Err(format!(
                "Working directory '{}' resolves outside the workspace root '{}'. A custom tool may only run inside the workspace.",
                self.working_dir,
                root.display()
            ));
        }
        Ok(resolved)
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

    /// A shell command is the least constrained thing an agent can reach for:
    /// it can read and write anywhere the RustyAgent process can, and it takes
    /// no path input that a path allow-list could be checked against.
    ///
    /// Declaring it as both a read and a write is what puts it behind
    /// `require_approval_on_write`, and what makes `allow_file_read_paths` /
    /// `allow_file_write_paths` refuse it outright instead of waving it
    /// through unchecked. See `PermissionPolicy::check_tool`.
    fn permission_info(&self) -> ToolPermissionInfo {
        ToolPermissionInfo {
            reads_files: true,
            writes_files: true,
            path_inputs: &[],
            shell_program: self.argv().map(|(program, _)| program),
        }
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

        let cwd = match self.resolve_working_dir(ctx.workspace_root.as_ref()) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            // Never allow the process to read from stdin.
            .stdin(std::process::Stdio::null());
        // The rest of the environment is inherited, minus provider credentials.
        scrub_provider_credentials(&mut cmd);

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

    /// A path that is absolute on both platforms, given a leaf name.
    fn absolute_outside(leaf: &str) -> String {
        if cfg!(windows) {
            format!("C:\\{leaf}")
        } else {
            format!("/{leaf}")
        }
    }

    /// Compare two paths as the filesystem sees them.
    ///
    /// Not `assert_eq!` on the raw values: `/workspace/proj` really does mean
    /// `C:\workspace\proj` on Windows, so a resolved path and the literal it
    /// came from are equal on Unix and unequal there. These tests use a real
    /// directory and compare canonical forms, which is true on both.
    fn assert_same_dir(left: &Path, right: &Path) {
        let l = std::fs::canonicalize(left).unwrap_or_else(|e| panic!("{left:?}: {e}"));
        let r = std::fs::canonicalize(right).unwrap_or_else(|e| panic!("{right:?}: {e}"));
        assert_eq!(l, r);
    }

    #[test]
    fn a_dot_working_dir_resolves_to_the_workspace_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let mut t = tool("ls");
        t.working_dir = ".".into();

        assert_same_dir(&t.resolve_working_dir(Some(&root)).expect("inside"), &root);
    }

    #[test]
    fn an_empty_working_dir_resolves_to_the_workspace_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let mut t = tool("ls");
        t.working_dir = String::new();

        assert_same_dir(&t.resolve_working_dir(Some(&root)).expect("inside"), &root);
    }

    #[test]
    fn a_relative_working_dir_is_joined_onto_the_workspace_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let nested = root.join("crates").join("api");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let mut t = tool("ls");
        t.working_dir = "crates/api".into();

        assert_same_dir(&t.resolve_working_dir(Some(&root)).expect("inside"), &nested);
    }

    /// Changed deliberately. This previously asserted that an absolute
    /// `working_dir` *overrode* the workspace root, which meant a custom tool
    /// could run its command anywhere on the machine.
    #[test]
    fn an_absolute_working_dir_outside_the_workspace_root_is_rejected() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = absolute_outside("elsewhere");

        let err = t
            .resolve_working_dir(Some(&root))
            .expect_err("an escape must be refused");
        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    #[test]
    fn an_absolute_working_dir_inside_the_workspace_root_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let nested = root.join("crates");
        std::fs::create_dir_all(&nested).expect("mkdir");

        let mut t = tool("ls");
        t.working_dir = nested.to_string_lossy().into_owned();

        assert_same_dir(&t.resolve_working_dir(Some(&root)).expect("inside"), &nested);
    }

    #[test]
    fn a_relative_working_dir_that_climbs_out_of_the_workspace_is_rejected() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = "crates/../../../etc".into();

        let err = t
            .resolve_working_dir(Some(&root))
            .expect_err("a traversal must be refused");
        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    /// A sibling directory whose name merely starts with the root's is not
    /// inside it — the containment check is on components, not characters.
    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_rejected() {
        let root = PathBuf::from("/workspace/proj");
        let mut t = tool("ls");
        t.working_dir = "/workspace/proj-other".into();

        let err = t
            .resolve_working_dir(Some(&root))
            .expect_err("a name-prefix sibling must be refused");
        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    /// With no workspace open there is no root to be outside of, so the
    /// directory is taken as written. This is the pre-existing behaviour and is
    /// not a bypass of the check above.
    #[test]
    fn without_a_workspace_root_the_working_dir_is_taken_as_given() {
        let mut t = tool("ls");
        t.working_dir = absolute_outside("elsewhere");

        assert_eq!(
            t.resolve_working_dir(None).expect("no root to escape"),
            PathBuf::from(&t.working_dir)
        );
    }

    #[tokio::test]
    async fn a_working_dir_outside_the_workspace_fails_the_call_instead_of_running() {
        let mut ctx = make_ctx(make_test_pool().await);
        ctx.workspace_root = Some(PathBuf::from("/workspace/proj"));
        let mut t = tool(if cfg!(windows) { "cmd /c rem" } else { "true" });
        t.working_dir = absolute_outside("elsewhere");

        let out = t.execute(json!({}), &ctx).await;

        assert!(out.is_error);
        assert!(
            out.content.contains("outside the workspace root"),
            "got {:?}",
            out.content
        );
    }

    // -- environment ---------------------------------------------------------

    #[tokio::test]
    async fn a_provider_credential_is_not_visible_to_the_spawned_command() {
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo %ANTHROPIC_API_KEY%"])
        } else {
            ("sh", vec!["-c", "echo $ANTHROPIC_API_KEY"])
        };

        let mut cmd = Command::new(program);
        cmd.args(&args).env("ANTHROPIC_API_KEY", "sk-do-not-leak");
        scrub_provider_credentials(&mut cmd);

        let out = cmd.output().await.expect("run echo");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();

        assert!(
            !stdout.contains("sk-do-not-leak"),
            "the key reached the command: {stdout:?}"
        );
    }

    #[tokio::test]
    async fn an_unrelated_environment_variable_still_reaches_the_command() {
        // The scrub is a named list, not a blanket wipe — a command that needs
        // its normal environment must keep it.
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo %RUSTYAGENT_SCRUB_PROBE%"])
        } else {
            ("sh", vec!["-c", "echo $RUSTYAGENT_SCRUB_PROBE"])
        };

        let mut cmd = Command::new(program);
        cmd.args(&args).env("RUSTYAGENT_SCRUB_PROBE", "kept");
        scrub_provider_credentials(&mut cmd);

        let out = cmd.output().await.expect("run echo");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();

        assert!(stdout.contains("kept"), "got {stdout:?}");
    }

    // -- permission declaration ----------------------------------------------

    #[test]
    fn a_custom_shell_tool_declares_itself_as_reading_and_writing() {
        let info = tool("cargo test --workspace").permission_info();

        assert!(info.writes_files, "a shell command can write anywhere");
        assert!(info.reads_files, "a shell command can read anything");
        assert!(
            info.path_inputs.is_empty(),
            "there is no path input a path allow-list could check"
        );
    }

    #[test]
    fn the_declared_shell_program_is_the_program_not_the_argument_text() {
        let info = tool("git commit -m 'npm run build'").permission_info();
        assert_eq!(info.shell_program.as_deref(), Some("git"));
    }

    #[test]
    fn an_empty_command_declares_no_shell_program() {
        assert_eq!(tool("").permission_info().shell_program, None);
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
