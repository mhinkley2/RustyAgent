// Built-in agent tools: story CRUD, memory read/write, notifications, etc.
// See RUSTYAGE-5 for implementation details.

pub mod builtin;
pub mod paging;
pub mod paths;
pub mod read_cap;
pub mod shell;

use std::sync::Arc;

use async_trait::async_trait;
use db::DbPool;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Tool execution context
// ---------------------------------------------------------------------------

/// A boxed async callback used by `SpawnSubtaskTool`.
///
/// Signature: `(story_id, agent_id, pipeline_run_id, depth, workspace_root) -> run_id`
///
/// `workspace_root` is the *caller's* root, which for an isolated run is its
/// private git worktree. A subtask inherits it rather than resolving the active
/// workspace afresh: the parent spawned it to do work the parent will then
/// look at, and a subtask that wrote somewhere else would both be invisible to
/// the parent and land in the user's checkout.
pub type SpawnSubtaskFn = Arc<
    dyn Fn(String, String, String, u32, Option<std::path::PathBuf>) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>>
    + Send
    + Sync,
>;

// ---------------------------------------------------------------------------
// Notification sink
// ---------------------------------------------------------------------------

/// Delivers an OS notification on behalf of a tool.
///
/// Injected rather than called directly because this crate cannot depend on
/// Tauri: the standalone `rustyagent-board-mcp` binary links `tools` without an
/// app to deliver through, and so supplies `None`. A tool that finds `None`
/// must report that it could not notify — see `SendNotificationTool`.
#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    /// Deliver `title`/`body`, or return why it could not be delivered.
    ///
    /// The error string is shown to the model, so it should say what went
    /// wrong in terms the model can act on ("permission refused" rather than a
    /// plugin error code).
    ///
    /// `category` is what the user's per-category preferences are keyed on, so
    /// suppression lives in the implementation rather than at each call site —
    /// one place to get right, and the same answer for an agent-initiated
    /// notification as for an automatic one.
    async fn notify(
        &self,
        category: NotificationCategory,
        title: &str,
        body: &str,
    ) -> Result<(), String>;
}

/// The user's notification preferences.
///
/// Lives here, beside [`NotificationCategory`], so the switch and the thing it
/// switches cannot drift apart: `commands::AppSettings` stores this type
/// verbatim and `runtime::AppNotifier` reads the same one back.
///
/// Every field defaults to on. Notifications are the whole point of an
/// unattended run — a default-off delivery path is the stub this replaced,
/// only with a settings screen in front of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotificationSettings {
    /// Master switch. Off suppresses every category.
    pub enabled: bool,
    /// A gated tool call is waiting on the user.
    pub on_approval: bool,
    /// A run ended in `failed`.
    pub on_run_failed: bool,
    /// A run ended in `done`.
    pub on_run_completed: bool,
    /// An agent called `send_notification` itself.
    pub on_agent_request: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            on_approval: true,
            on_run_failed: true,
            on_run_completed: true,
            on_agent_request: true,
        }
    }
}

impl NotificationSettings {
    /// Whether a delivery in `category` should go out.
    pub fn allows(&self, category: NotificationCategory) -> bool {
        if !self.enabled {
            return false;
        }
        match category {
            NotificationCategory::Agent => self.on_agent_request,
            NotificationCategory::Approval => self.on_approval,
            NotificationCategory::RunFailed => self.on_run_failed,
            NotificationCategory::RunCompleted => self.on_run_completed,
        }
    }
}

/// Interpret a configured approval-timeout value.
///
/// `None` — and zero — mean "wait indefinitely". Zero is not "expire
/// immediately": a zero-second gate would deny every gated call the instant it
/// was raised, which has no use case and is almost certainly a cleared input
/// field.
///
/// Lives here because two crates read the same setting: `commands::AppSettings`
/// exposes it to the UI, and `runtime` reads it back out of `settings.json`
/// where it cannot see `commands`. One rule, both readers.
pub fn approval_timeout_from_secs(secs: Option<u64>) -> Option<std::time::Duration> {
    secs.filter(|s| *s > 0).map(std::time::Duration::from_secs)
}

/// Which switch in the user's notification preferences governs a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    /// An agent called `send_notification` itself.
    Agent,
    /// A tool call is gated and the run is parked until the user decides.
    Approval,
    /// A run reached `failed`.
    RunFailed,
    /// A run reached `completed`.
    RunCompleted,
}

#[derive(Clone)]
pub struct ToolContext {
    pub db: DbPool,
    pub agent_profile_id: String,
    pub run_id: String,
    /// Set for pipeline runs so that `shared_scratchpad` memory is scoped
    /// to this pipeline execution.
    pub pipeline_run_id: Option<String>,
    /// Current recursion depth — guards against infinite spawning chains.
    pub pipeline_depth: u32,
    /// Optional callback injected by the pipeline engine so `spawn_subtask`
    /// can fire new agent runs without depending on the `pipeline` crate.
    pub spawn_subtask: Option<SpawnSubtaskFn>,
    /// Workspace root directory. File tools are confined to this path.
    /// When None, file access is unrestricted (not recommended for production).
    pub workspace_root: Option<std::path::PathBuf>,
    /// How `send_notification` reaches the desktop. `None` where no desktop
    /// exists to reach — the stdio MCP binary, and most tests.
    pub notifier: Option<Arc<dyn Notifier>>,
}

// ---------------------------------------------------------------------------
// Tool result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: false }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self { content: content.into(), is_error: true }
    }
}

// ---------------------------------------------------------------------------
// Tool trait
// ---------------------------------------------------------------------------

/// What a tool tells the permission policy about itself.
///
/// The policy lives in the `runtime` crate and only ever sees a tool *name* and
/// a JSON blob of inputs. That is not enough to decide anything: it cannot know
/// that `file_list` reads the filesystem, that a particular custom tool shells
/// out to `git`, or which input key of an arbitrary tool holds a path. So each
/// tool declares it here and the policy reads the declaration.
///
/// The default is "inert" — a tool that touches neither the filesystem nor a
/// subprocess. That is the right default for the story, memory, notification
/// and subtask tools, and a tool that forgets to override it is treated as
/// harmless only because it also cannot be *reached* by any of the gates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolPermissionInfo {
    /// The tool reads file contents or directory listings.
    pub reads_files: bool,
    /// The tool mutates the filesystem.
    pub writes_files: bool,
    /// Input keys whose values are filesystem paths.
    ///
    /// Every one of them is checked, so a tool is not required to name its
    /// path parameter `path` to be covered.
    pub path_inputs: &'static [&'static str],
    /// The program a shell-style tool will execute, already separated from its
    /// arguments so that argument text cannot smuggle a match past an
    /// allow-list.
    pub shell_program: Option<String>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolOutput;

    /// How the permission policy should treat this tool. See
    /// [`ToolPermissionInfo`].
    fn permission_info(&self) -> ToolPermissionInfo {
        ToolPermissionInfo::default()
    }
}

// ---------------------------------------------------------------------------
// Tool registry
// ---------------------------------------------------------------------------

pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool. Accepts a `Box<dyn Tool>` for ergonomic call-sites that
    /// construct tools with `Box::new(...)`, and converts it to an `Arc` internally.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(Arc::from(tool));
    }

    /// Borrow a reference to a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    /// Clone the `Arc` for a tool by name. Use this when you need to hold the
    /// tool across an `.await` without keeping the registry lock held.
    pub fn get_arc(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    /// The permission declaration of a registered tool, or `None` when no tool
    /// of that name is registered.
    ///
    /// `None` is what makes the policy fail closed: a call the registry cannot
    /// identify cannot be classified, and an unclassifiable call must not slip
    /// past a restriction the operator configured.
    pub fn permission_info(&self, name: &str) -> Option<ToolPermissionInfo> {
        self.get(name).map(|t| t.permission_info())
    }

    pub fn all_definitions(&self) -> Vec<api::ToolDefinition> {
        self.tools
            .iter()
            .map(|t| api::ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shared test helpers — available to all modules within this crate.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::ToolContext;
    use db::DbPool;

    /// Open a single-connection in-memory SQLite pool and run all migrations.
    /// Foreign-key enforcement is intentionally left OFF so tests can insert
    /// rows without needing full relational-seed data.
    pub async fn make_test_pool() -> DbPool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("Failed to open in-memory SQLite");

        // FK off — tests seed only the rows they need.
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&pool)
            .await
            .expect("Failed to disable foreign keys");

        sqlx::migrate!("../db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run migrations");

        pool
    }

    /// Build a minimal `ToolContext` backed by `db`.
    pub fn make_ctx(db: DbPool) -> ToolContext {
        ToolContext {
            db,
            agent_profile_id: "test-agent-001".to_string(),
            run_id: "test-run-001".to_string(),
            pipeline_run_id: None,
            pipeline_depth: 0,
            spawn_subtask: None,
            workspace_root: None,
            notifier: None,
        }
    }
}
