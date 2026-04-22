// Built-in file system tools: file_read, file_write, file_list.
//
// Security model:
// - When ctx.workspace_root is set, ALL paths are resolved relative to it and
//   must remain inside it (canonicalize-based check, immune to symlink tricks).
// - When workspace_root is None, paths must be absolute and ".." components
//   are blocked as a minimal guard.
// - The PermissionPolicy's allow_file_write_paths adds a second layer for writes.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::{Tool, ToolContext, ToolOutput};

// ---------------------------------------------------------------------------
// Path resolution helper
// ---------------------------------------------------------------------------

/// Resolve `requested` against `workspace_root` (if set) and verify the
/// result stays inside the workspace. Returns the absolute path or an error.
fn resolve_path(requested: &str, ctx: &ToolContext) -> Result<PathBuf, String> {
    if let Some(root) = &ctx.workspace_root {
        // Reject absolute paths when a workspace root is configured.
        if Path::new(requested).is_absolute() {
            return Err(
                "Absolute paths are not allowed when a workspace root is configured. \
                 Use a path relative to the workspace root (e.g. \"docs/output.md\").".into()
            );
        }
        // Strip any leading separators so join works correctly.
        let stripped = requested.trim_start_matches('/').trim_start_matches('\\');
        let candidate = root.join(stripped);
        let canonical_root = std::fs::canonicalize(root)
            .map(|p| {
                // On Windows, canonicalize adds a \\?\ prefix. Strip it so
                // the starts_with check works against normalize_path output.
                let s = p.to_string_lossy();
                let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
                PathBuf::from(stripped)
            })
            .unwrap_or_else(|_| root.clone());
        // Lexically normalise (canonicalize would fail for non-existent files).
        let normalised = normalize_path(&candidate);
        if !normalised.starts_with(&canonical_root) {
            return Err(format!(
                "Path '{}' resolves outside the workspace root. \
                 Only paths inside the workspace are permitted.",
                requested
            ));
        }
        Ok(normalised)
    } else {
        // No workspace root — require absolute paths, block ".." components.
        let p = Path::new(requested);
        if !p.is_absolute() {
            return Err(
                "Path must be absolute when no workspace root is configured.".into()
            );
        }
        if p.components().any(|c| c.as_os_str() == "..") {
            return Err("Path must not contain '..' components.".into());
        }
        Ok(p.to_path_buf())
    }
}

/// Lexically normalise a path (resolves `.` and `..` without touching the FS).
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// file_read
// ---------------------------------------------------------------------------

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str { "file_read" }

    fn description(&self) -> &str {
        "Read the text content of a file. Provide a path relative to the workspace \
         root (e.g. \"docs/report.md\"). Absolute paths are rejected when a workspace \
         root is configured."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the workspace root."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolOutput {
        let requested = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolOutput::err("Missing required parameter: path"),
        };
        let abs_path = match resolve_path(requested, ctx) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        match tokio::fs::read_to_string(&abs_path).await {
            Ok(content) => ToolOutput::ok(content),
            Err(e) => ToolOutput::err(format!("Failed to read '{}': {e}", abs_path.display())),
        }
    }
}

// ---------------------------------------------------------------------------
// file_write
// ---------------------------------------------------------------------------

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str { "file_write" }

    fn description(&self) -> &str {
        "Write text content to a file, creating it and any parent directories if \
         needed. Overwrites existing content. Provide a path relative to the \
         workspace root (e.g. \"docs/report.md\")."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the workspace root."
                },
                "content": {
                    "type": "string",
                    "description": "Text content to write to the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolOutput {
        let requested = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolOutput::err("Missing required parameter: path"),
        };
        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => return ToolOutput::err("Missing required parameter: content"),
        };
        let abs_path = match resolve_path(requested, ctx) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        if let Some(parent) = abs_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolOutput::err(format!(
                    "Failed to create directories for '{}': {e}", abs_path.display()
                ));
            }
        }
        match tokio::fs::write(&abs_path, content).await {
            Ok(()) => ToolOutput::ok(format!("Successfully wrote to '{}'", abs_path.display())),
            Err(e) => ToolOutput::err(format!("Failed to write '{}': {e}", abs_path.display())),
        }
    }
}

// ---------------------------------------------------------------------------
// file_list
// ---------------------------------------------------------------------------

pub struct FileListTool;

#[async_trait]
impl Tool for FileListTool {
    fn name(&self) -> &str { "file_list" }

    fn description(&self) -> &str {
        "List files and subdirectories inside a directory. Provide a path relative \
         to the workspace root, or use \".\" to list the workspace root itself."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path relative to the workspace root. Use \".\" for the root."
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolOutput {
        let requested = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolOutput::err("Missing required parameter: path"),
        };
        let abs_path = match resolve_path(requested, ctx) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        let mut rd = match tokio::fs::read_dir(&abs_path).await {
            Ok(rd) => rd,
            Err(e) => return ToolOutput::err(format!(
                "Failed to read directory '{}': {e}", abs_path.display()
            )),
        };
        let mut entries = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                    entries.push(if is_dir { format!("{name}/") } else { name });
                }
                Ok(None) => break,
                Err(e) => return ToolOutput::err(format!("Error reading entry: {e}")),
            }
        }
        entries.sort();
        ToolOutput::ok(entries.join("\n"))
    }
}
