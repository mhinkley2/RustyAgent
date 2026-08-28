// Built-in agent tools: story CRUD, memory read/write, notifications, etc.
// See RUSTYAGE-5 for implementation details.

pub mod builtin;
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
        }
    }
}
