//! The permission policy: the single decision point between a model asking for
//! a tool call and the runtime making it.
//!
//! Every control this type carries is read by [`PermissionPolicy::check_tool`].
//! That is a deliberate invariant, not an accident of the current shape: a
//! control that is stored, rendered in the permission editor, and never
//! consulted is worse than no control at all, because an operator configures it
//! and then believes it. If a field here ever stops being read, delete it —
//! from this struct, the DB schema, the commands layer, the TOML profile format
//! and the UI, together.
//!
//! `allow_network_hosts` was removed for exactly that reason: RustyAgent has no
//! network-capable agent tool to gate, so it was a lock on a door that does not
//! exist.

use std::path::Path;

use tools::paths::{is_within, resolve_for_containment};
use tools::ToolPermissionInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequiresApproval,
}

/// Built-in tools that mutate the filesystem, by name.
///
/// The authoritative classification is [`tools::Tool::permission_info`] — a
/// tool declares what it does and the policy reads the declaration. This list
/// says the same thing by name so that a call can still be classified when the
/// registry is not consulted (`check_tool` called with no
/// [`ToolPermissionInfo`], which is how the direct unit tests and any future
/// caller without a registry reach it).
///
/// Both entries are real agent tools registered by
/// `tools::builtin::register_builtins`: `file_write` here, and `file_edit` from
/// the patch-based edit story. The six frontend/Tauri filesystem *command*
/// names that used to sit in this list — `write_file_text`, `create_empty_file`,
/// `create_dir_fs`, `delete_path`, `rename_path`, `duplicate_file` — were never
/// agent tools and no model could ever call them, so listing them gated
/// nothing.
pub(crate) const WRITE_TOOLS: &[&str] = &["file_write", "file_edit"];

/// Built-in tools that read the filesystem, by name. Same rationale as
/// [`WRITE_TOOLS`].
pub(crate) const READ_TOOLS: &[&str] = &["file_read", "file_list"];

/// The input key a built-in file tool carries its path in. Used only when the
/// caller supplied no [`ToolPermissionInfo`]; a registered tool declares its
/// own `path_inputs` and is not required to name the parameter `path`.
const DEFAULT_PATH_INPUTS: &[&str] = &["path"];

/// Everything the policy needs to know about one tool call.
///
/// A tool name and a JSON blob are not enough to decide anything — see
/// [`ToolPermissionInfo`]. The runtime fills `info` from the tool registry and
/// `workspace_root` from the run, so relative paths resolve the same way the
/// tool itself will resolve them.
pub struct ToolRequest<'a> {
    pub name: &'a str,
    pub inputs: &'a serde_json::Value,
    /// The registered tool's own declaration, or `None` when the registry does
    /// not know this name. `None` makes the path and shell gates fail closed.
    pub info: Option<ToolPermissionInfo>,
    /// The directory relative paths are resolved against.
    pub workspace_root: Option<&'a Path>,
}

impl<'a> ToolRequest<'a> {
    pub fn new(name: &'a str, inputs: &'a serde_json::Value) -> Self {
        Self { name, inputs, info: None, workspace_root: None }
    }

    pub fn with_info(mut self, info: Option<ToolPermissionInfo>) -> Self {
        self.info = info;
        self
    }

    pub fn with_workspace_root(mut self, root: Option<&'a Path>) -> Self {
        self.workspace_root = root;
        self
    }

    fn reads_files(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.reads_files) || READ_TOOLS.contains(&self.name)
    }

    fn writes_files(&self) -> bool {
        self.info.as_ref().is_some_and(|i| i.writes_files) || WRITE_TOOLS.contains(&self.name)
    }

    fn path_inputs(&self) -> &[&str] {
        match &self.info {
            Some(info) => info.path_inputs,
            None => DEFAULT_PATH_INPUTS,
        }
    }

    fn shell_program(&self) -> Option<&str> {
        self.info.as_ref()?.shell_program.as_deref()
    }

    /// Path values this call actually supplied, from the keys the tool declared.
    fn supplied_paths(&self) -> Vec<&str> {
        self.path_inputs()
            .iter()
            .filter_map(|key| self.inputs.get(*key))
            .filter_map(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub allowed_tools: Vec<String>,
    pub allow_file_read_paths: Vec<String>,
    pub allow_file_write_paths: Vec<String>,
    pub allow_shell_commands: Vec<String>,
    pub require_approval_on_write: bool,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            allowed_tools: vec![],
            allow_file_read_paths: vec![],
            allow_file_write_paths: vec![],
            allow_shell_commands: vec![],
            require_approval_on_write: false,
        }
    }

    pub fn restricted(tools: Vec<String>) -> Self {
        Self {
            allowed_tools: tools,
            ..Self::allow_all()
        }
    }

    pub fn from_db_json(value: Option<serde_json::Value>) -> Self {
        match value {
            None | Some(serde_json::Value::Null) => Self::allow_all(),
            Some(serde_json::Value::Array(arr)) => {
                let names = arr
                    .into_iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                Self::restricted(names)
            }
            _ => Self::allow_all(),
        }
    }

    pub fn from_db_permissions(
        allowed_tools_json: &str,
        allow_file_read_paths_json: &str,
        allow_file_write_paths_json: &str,
        allow_shell_commands_json: &str,
        require_approval_on_write: bool,
    ) -> Self {
        let parse_list = |s: &str| -> Vec<String> {
            serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
        };
        Self {
            allowed_tools: parse_list(allowed_tools_json),
            allow_file_read_paths: parse_list(allow_file_read_paths_json),
            allow_file_write_paths: parse_list(allow_file_write_paths_json),
            allow_shell_commands: parse_list(allow_shell_commands_json),
            require_approval_on_write,
        }
    }

    pub fn check(&self, tool_name: &str) -> bool {
        if self.allowed_tools.is_empty() {
            return true;
        }
        self.allowed_tools.iter().any(|t| t == tool_name)
    }

    /// Does this policy restrict anything beyond the tool-name allow-list?
    ///
    /// `require_approval_on_write` is deliberately not counted: it is a prompt,
    /// not a containment boundary, and failing closed on it would deny every
    /// call the registry cannot identify for a setting that only means "ask
    /// me".
    fn has_path_or_shell_restrictions(&self) -> bool {
        !allow_list(&self.allow_file_read_paths).is_empty()
            || !allow_list(&self.allow_file_write_paths).is_empty()
            || !allow_list(&self.allow_shell_commands).is_empty()
    }

    pub fn check_tool(&self, req: &ToolRequest<'_>) -> PolicyDecision {
        if !self.check(req.name) {
            return PolicyDecision::Deny(format!(
                "Tool '{}' is not permitted for this agent profile",
                req.name
            ));
        }

        // No restriction is configured beyond the tool allow-list, so nothing
        // below can change the answer. Taking this exit first is what keeps an
        // unconfigured profile behaving exactly as it did before enforcement
        // existed.
        if !self.has_path_or_shell_restrictions() && !self.require_approval_on_write {
            return PolicyDecision::Allow;
        }

        // A call the registry cannot identify cannot be classified, and an
        // unclassifiable call must not slip past a restriction the operator
        // configured. In practice this is unreachable — the runtime looks the
        // tool up in the same registry to execute it — but "unreachable" is not
        // a security property.
        if req.info.is_none()
            && !READ_TOOLS.contains(&req.name)
            && !WRITE_TOOLS.contains(&req.name)
            && self.has_path_or_shell_restrictions()
        {
            return PolicyDecision::Deny(format!(
                "Tool '{}' could not be classified against this profile's path and \
                 shell restrictions, so it was refused",
                req.name
            ));
        }

        if let Some(deny) = self.check_shell(req) {
            return deny;
        }
        if let Some(deny) = self.check_paths(req, Access::Read) {
            return deny;
        }
        if let Some(deny) = self.check_paths(req, Access::Write) {
            return deny;
        }

        if self.require_approval_on_write && req.writes_files() {
            return PolicyDecision::RequiresApproval;
        }

        PolicyDecision::Allow
    }

    /// Gate a shell-style tool on the program it will execute.
    ///
    /// The match is against the resolved program, never the raw command string,
    /// so `echo git` does not satisfy an allow-list of `git`.
    fn check_shell(&self, req: &ToolRequest<'_>) -> Option<PolicyDecision> {
        let allowed = allow_list(&self.allow_shell_commands);
        if allowed.is_empty() {
            return None;
        }
        let program = req.shell_program()?;

        if allowed.iter().any(|entry| program_matches(program, entry)) {
            None
        } else {
            Some(PolicyDecision::Deny(format!(
                "Tool '{}' runs '{}', which is not on this profile's allowed shell commands",
                req.name, program
            )))
        }
    }

    /// Gate a filesystem-touching tool on where it touches.
    fn check_paths(&self, req: &ToolRequest<'_>, access: Access) -> Option<PolicyDecision> {
        let (list, applies) = match access {
            Access::Read => (&self.allow_file_read_paths, req.reads_files()),
            Access::Write => (&self.allow_file_write_paths, req.writes_files()),
        };
        let allowed = allow_list(list);
        if allowed.is_empty() || !applies {
            return None;
        }

        let prefixes: Vec<_> = allowed
            .iter()
            .map(|p| resolve_for_containment(p, req.workspace_root))
            .collect();

        let supplied = req.supplied_paths();
        if supplied.is_empty() {
            // The tool touches the filesystem but offers nothing to check —
            // a shell command is the case that matters. Allowing it would be a
            // silent hole straight through the restriction, so it is refused,
            // loudly, with the reason.
            return Some(PolicyDecision::Deny(format!(
                "Tool '{}' can {} arbitrary locations and exposes no path that could be \
                 checked against this profile's allowed {} paths, so it was refused. \
                 Clear that list, or unbind the tool.",
                req.name,
                access.verb(),
                access.noun(),
            )));
        }

        for raw in supplied {
            let candidate = resolve_for_containment(raw, req.workspace_root);
            if !prefixes.iter().any(|prefix| is_within(&candidate, prefix)) {
                return Some(PolicyDecision::Deny(format!(
                    "{} '{}' is outside this profile's allowed {} paths",
                    access.gerund(),
                    raw,
                    access.noun(),
                )));
            }
        }
        None
    }
}

#[derive(Clone, Copy)]
enum Access {
    Read,
    Write,
}

impl Access {
    fn verb(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write to",
        }
    }
    fn noun(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
    fn gerund(self) -> &'static str {
        match self {
            Access::Read => "Reading",
            Access::Write => "Writing to",
        }
    }
}

/// Drop blank entries from an allow-list.
///
/// A blank entry is a configuration slip, and both readings of it are bad: as a
/// prefix it would match everything and silently disable the restriction it
/// appears in. Dropping it leaves the operator's real entries in force, and an
/// all-blank list reads as "unset", which is the documented meaning of empty.
fn allow_list(entries: &[String]) -> Vec<&str> {
    entries
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .collect()
}

/// Does `program` satisfy the allow-list entry `entry`?
///
/// An entry with no path separator matches on the program's file name, so
/// `git` permits `/usr/bin/git`. An entry that names a directory is taken at
/// its word and must match the whole path — an operator who wrote out an
/// absolute path meant that binary, not any binary sharing its name.
///
/// On Windows the comparison ignores case and the executable suffix, because
/// the OS does.
fn program_matches(program: &str, entry: &str) -> bool {
    if entry.contains('/') || entry.contains('\\') {
        return normalise_program(program) == normalise_program(entry);
    }
    let name = Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_string());
    normalise_program(&name) == normalise_program(entry)
}

fn normalise_program(value: &str) -> String {
    if !cfg!(windows) {
        return value.to_string();
    }
    let lowered = value.to_lowercase().replace('\\', "/");
    for suffix in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(stem) = lowered.strip_suffix(suffix) {
            return stem.to_string();
        }
    }
    lowered
}
