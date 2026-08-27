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
            .map(|p| strip_unc(&p))
            .unwrap_or_else(|_| normalize_path(root));
        // Resolve symlinks as far as the path actually exists — a purely
        // lexical normalisation would let a link inside the workspace point out
        // of it.
        let resolved = resolve_existing_prefix(&candidate);
        if !resolved.starts_with(&canonical_root) {
            return Err(format!(
                "Path '{}' resolves outside the workspace root. \
                 Only paths inside the workspace are permitted.",
                requested
            ));
        }
        Ok(resolved)
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

/// On Windows `canonicalize` returns a `\\?\`-prefixed path; strip it so both
/// sides of the containment check are in the same form.
fn strip_unc(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

/// Canonicalise the deepest existing ancestor of `path`, then re-append the
/// components that do not exist yet.
///
/// `canonicalize` alone fails outright for a file being created for the first
/// time, but skipping it entirely (a purely lexical normalisation) would let a
/// symlink inside the workspace resolve to a target outside it.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let normalised = normalize_path(path);
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = normalised.as_path();

    loop {
        if let Ok(real) = std::fs::canonicalize(probe) {
            let mut out = strip_unc(&real);
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (probe.parent(), probe.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                probe = parent;
            }
            // Reached the root without finding anything that exists.
            _ => return normalised,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{make_ctx, make_test_pool};
    use tempfile::TempDir;

    /// A context rooted at a real temp directory.
    ///
    /// The root is canonicalised up front so the assertions compare like with
    /// like — on Windows `std::env::temp_dir()` can hand back an 8.3 short name.
    async fn rooted_ctx() -> (TempDir, ToolContext) {
        let dir = TempDir::new().expect("temp dir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize root");
        let mut ctx = make_ctx(make_test_pool().await);
        ctx.workspace_root = Some(strip_unc(&root));
        (dir, ctx)
    }

    async fn rootless_ctx() -> ToolContext {
        make_ctx(make_test_pool().await)
    }

    fn root_of(ctx: &ToolContext) -> PathBuf {
        ctx.workspace_root.clone().expect("workspace root")
    }

    // -- containment: the paths that must be allowed ------------------------

    #[tokio::test]
    async fn a_relative_path_inside_the_workspace_resolves() {
        let (_dir, ctx) = rooted_ctx().await;

        let resolved = resolve_path("docs/report.md", &ctx).expect("should resolve");

        assert_eq!(resolved, root_of(&ctx).join("docs").join("report.md"));
    }

    #[tokio::test]
    async fn a_leading_slash_is_stripped_rather_than_treated_as_absolute() {
        let (_dir, ctx) = rooted_ctx().await;

        let resolved = resolve_path("/docs/x.md", &ctx).expect("should resolve");

        assert_eq!(resolved, root_of(&ctx).join("docs").join("x.md"));
    }

    #[tokio::test]
    async fn a_dot_path_resolves_to_the_workspace_root() {
        let (_dir, ctx) = rooted_ctx().await;

        let resolved = resolve_path(".", &ctx).expect("should resolve");

        assert_eq!(resolved, root_of(&ctx));
    }

    #[tokio::test]
    async fn interior_dotdot_that_stays_inside_is_allowed() {
        let (_dir, ctx) = rooted_ctx().await;

        let resolved = resolve_path("docs/../src/main.rs", &ctx).expect("should resolve");

        assert_eq!(resolved, root_of(&ctx).join("src").join("main.rs"));
    }

    // -- containment: the paths that must be rejected ------------------------

    #[tokio::test]
    async fn an_absolute_path_is_rejected_when_a_workspace_root_is_set() {
        let (_dir, ctx) = rooted_ctx().await;
        let absolute = if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "/etc/passwd"
        };

        let err = resolve_path(absolute, &ctx).expect_err("should be rejected");

        assert!(err.contains("Absolute paths are not allowed"), "got {err}");
    }

    #[tokio::test]
    async fn parent_traversal_is_rejected() {
        let (_dir, ctx) = rooted_ctx().await;

        for attempt in [
            "../secrets.txt",
            "a/../../secrets.txt",
            "./../../etc/passwd",
            "docs/../../../../../../etc/passwd",
        ] {
            match resolve_path(attempt, &ctx) {
                Ok(p) => panic!("{attempt} escaped to {}", p.display()),
                Err(e) => assert!(e.contains("outside the workspace root"), "got {e}"),
            }
        }
    }

    #[tokio::test]
    async fn a_sibling_directory_sharing_the_root_prefix_is_rejected() {
        // Root /w/proj must not admit /w/project-evil. `Path::starts_with` is
        // component-wise, so this holds — pinned so a refactor to a string
        // comparison cannot silently regress it.
        let dir = TempDir::new().expect("temp dir");
        let base = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let root = base.join("proj");
        let sibling = base.join("proj-evil");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&sibling).expect("mkdir sibling");

        let mut ctx = make_ctx(make_test_pool().await);
        ctx.workspace_root = Some(strip_unc(&root));

        let err = resolve_path("../proj-evil/loot.txt", &ctx).expect_err("should be rejected");

        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    #[tokio::test]
    async fn backslash_traversal_never_escapes_the_workspace() {
        // Windows treats `\` as a separator; POSIX treats it as an ordinary
        // filename character. Either way the result must stay inside the root.
        let (_dir, ctx) = rooted_ctx().await;
        let root = root_of(&ctx);

        match resolve_path("..\\..\\secrets.txt", &ctx) {
            Ok(resolved) => assert!(
                resolved.starts_with(&root),
                "{} escaped the workspace",
                resolved.display()
            ),
            Err(e) => assert!(e.contains("outside the workspace root"), "got {e}"),
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_symlink_pointing_out_of_the_workspace_is_rejected() {
        // The module header promises the containment check is "immune to
        // symlink tricks"; without resolving the existing prefix it was not.
        let (_dir, ctx) = rooted_ctx().await;
        let root = root_of(&ctx);
        let outside = TempDir::new().expect("outside dir");
        std::os::unix::fs::symlink(outside.path(), root.join("escape")).expect("symlink");

        let err = resolve_path("escape/loot.txt", &ctx).expect_err("should be rejected");

        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn a_directory_junction_pointing_out_of_the_workspace_is_rejected() {
        // The Windows counterpart to the symlink case above. Junctions (unlike
        // symlinks) need no elevation, so this runs on an ordinary dev box.
        let (_dir, ctx) = rooted_ctx().await;
        let root = root_of(&ctx);
        let outside = TempDir::new().expect("outside dir");

        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(root.join("escape"))
            .arg(outside.path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !made {
            eprintln!("skipping: could not create a directory junction here");
            return;
        }

        let err = resolve_path("escape/loot.txt", &ctx).expect_err("should be rejected");

        assert!(err.contains("outside the workspace root"), "got {err}");
    }

    #[tokio::test]
    async fn a_non_canonical_workspace_root_still_admits_inner_paths() {
        // A root reached via a `..` hop canonicalises to something different
        // from the literal path; both sides of the check must be normalised or
        // every path inside the workspace is denied.
        let dir = TempDir::new().expect("temp dir");
        let base = std::fs::canonicalize(dir.path()).expect("canonicalize");
        std::fs::create_dir_all(base.join("ws").join("sub")).expect("mkdir");

        let mut ctx = make_ctx(make_test_pool().await);
        ctx.workspace_root = Some(base.join("ws").join("sub").join(".."));

        let resolved = resolve_path("notes.md", &ctx).expect("should resolve");

        assert_eq!(resolved, strip_unc(&base).join("ws").join("notes.md"));
    }

    // -- no workspace root ---------------------------------------------------

    #[tokio::test]
    async fn without_a_workspace_root_a_relative_path_is_rejected() {
        let ctx = rootless_ctx().await;

        let err = resolve_path("docs/x.md", &ctx).expect_err("should be rejected");

        assert!(err.contains("must be absolute"), "got {err}");
    }

    #[tokio::test]
    async fn without_a_workspace_root_dotdot_is_blocked() {
        let ctx = rootless_ctx().await;
        let attempt = if cfg!(windows) {
            "C:\\tmp\\..\\secrets.txt"
        } else {
            "/tmp/../secrets.txt"
        };

        let err = resolve_path(attempt, &ctx).expect_err("should be rejected");

        assert!(err.contains("'..' components"), "got {err}");
    }

    #[tokio::test]
    async fn without_a_workspace_root_a_clean_absolute_path_is_allowed() {
        let ctx = rootless_ctx().await;
        let attempt = if cfg!(windows) {
            "C:\\tmp\\notes.md"
        } else {
            "/tmp/notes.md"
        };

        assert_eq!(
            resolve_path(attempt, &ctx).expect("should resolve"),
            PathBuf::from(attempt)
        );
    }

    // -- tool behaviour ------------------------------------------------------

    #[tokio::test]
    async fn file_write_creates_parent_directories_then_file_read_returns_the_content() {
        let (_dir, ctx) = rooted_ctx().await;

        let write = FileWriteTool
            .execute(
                json!({ "path": "deep/nested/notes.md", "content": "hello" }),
                &ctx,
            )
            .await;
        assert!(!write.is_error, "got {:?}", write.content);

        let read = FileReadTool
            .execute(json!({ "path": "deep/nested/notes.md" }), &ctx)
            .await;
        assert!(!read.is_error, "got {:?}", read.content);
        assert_eq!(read.content, "hello");
    }

    #[tokio::test]
    async fn file_write_refuses_to_escape_the_workspace() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileWriteTool
            .execute(json!({ "path": "../pwned.txt", "content": "x" }), &ctx)
            .await;

        assert!(out.is_error);
        assert!(
            !root_of(&ctx).parent().unwrap().join("pwned.txt").exists(),
            "the file was written outside the workspace"
        );
    }

    #[tokio::test]
    async fn file_read_on_a_missing_file_is_an_error_not_a_panic() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileReadTool
            .execute(json!({ "path": "nope.md" }), &ctx)
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("Failed to read"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn each_tool_reports_a_missing_path_parameter() {
        let (_dir, ctx) = rooted_ctx().await;

        for out in [
            FileReadTool.execute(json!({}), &ctx).await,
            FileWriteTool.execute(json!({ "content": "x" }), &ctx).await,
            FileListTool.execute(json!({}), &ctx).await,
        ] {
            assert!(out.is_error);
            assert!(out.content.contains("path"), "got {:?}", out.content);
        }
    }

    #[tokio::test]
    async fn file_write_reports_a_missing_content_parameter() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileWriteTool.execute(json!({ "path": "a.md" }), &ctx).await;

        assert!(out.is_error);
        assert!(out.content.contains("content"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_list_marks_directories_with_a_trailing_slash_and_sorts() {
        let (_dir, ctx) = rooted_ctx().await;
        let root = root_of(&ctx);
        std::fs::create_dir_all(root.join("zeta_dir")).expect("mkdir");
        std::fs::create_dir_all(root.join("alpha_dir")).expect("mkdir");
        std::fs::write(root.join("mid.txt"), "x").expect("write");

        let out = FileListTool.execute(json!({ "path": "." }), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(
            out.content.lines().collect::<Vec<_>>(),
            vec!["alpha_dir/", "mid.txt", "zeta_dir/"]
        );
    }

    #[tokio::test]
    async fn file_list_on_a_missing_directory_is_an_error() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileListTool
            .execute(json!({ "path": "no_such_dir" }), &ctx)
            .await;

        assert!(out.is_error);
        assert!(
            out.content.contains("Failed to read directory"),
            "got {:?}",
            out.content
        );
    }
}
