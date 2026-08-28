// Built-in file system tools: file_read, file_write, file_edit, file_list.
//
// Cost model:
// - Tool results are re-sent on every subsequent turn of a run, so an
//   unbounded read or a whole-file rewrite is charged to the context window
//   again and again. file_read is therefore capped and pageable, and file_edit
//   exists so that changing one line does not cost two copies of the file.
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

use crate::paths::{is_within, normalize_path, resolve_existing_prefix, strip_unc};
use crate::{Tool, ToolContext, ToolOutput, ToolPermissionInfo};

// ---------------------------------------------------------------------------
// Path resolution helper
// ---------------------------------------------------------------------------

/// Resolve `requested` against `workspace_root` (if set) and verify the
/// result stays inside the workspace. Returns the absolute path or an error.
fn resolve_path(requested: &str, ctx: &ToolContext) -> Result<PathBuf, String> {
    if let Some(root) = &ctx.workspace_root {
        // Reject absolute paths when a workspace root is configured.
        //
        // A leading separator is rejected too, and deliberately so: `is_absolute`
        // is platform-dependent — "/docs/x.md" is absolute on Unix but not on
        // Windows, which wants a drive prefix. Rejecting the leading separator
        // outright gives one rule on every platform, and an explicit error beats
        // silently reinterpreting "/etc/passwd" as "<root>/etc/passwd".
        if Path::new(requested).is_absolute() || requested.starts_with(['/', '\\']) {
            return Err(
                "Absolute paths are not allowed when a workspace root is configured. \
                 Use a path relative to the workspace root, with no leading separator \
                 (e.g. \"docs/output.md\").".into()
            );
        }
        let candidate = root.join(requested);
        let canonical_root = std::fs::canonicalize(root)
            .map(|p| strip_unc(&p))
            .unwrap_or_else(|_| normalize_path(root));
        // Resolve symlinks as far as the path actually exists — a purely
        // lexical normalisation would let a link inside the workspace point out
        // of it.
        let resolved = resolve_existing_prefix(&candidate);
        if !is_within(&resolved, &canonical_root) {
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

// ---------------------------------------------------------------------------
// file_read
// ---------------------------------------------------------------------------

/// Cap on the bytes a single `file_read` returns.
///
/// Mirrors `shell::MAX_OUTPUT_BYTES`, and for the same reason: tool output is
/// appended to the conversation and re-sent on every subsequent turn, so one
/// unbounded read of a lockfile, a minified bundle or a log can exhaust the
/// context window by itself. The value is deliberately the same 32 KB, so the
/// codebase has a single number for "more tool output than a turn can carry".
const MAX_READ_BYTES: usize = 32 * 1024;

/// Split `content` into lines that still carry their original terminator.
///
/// `str::lines` drops the terminator, and re-joining with "\n" would hand back
/// a CRLF file as LF. That is not cosmetic: the text the model reads here is
/// the text it quotes back as `file_edit`'s `old_string`, so a silent CRLF to
/// LF conversion on the way out guarantees the byte-exact match on the way in
/// will fail. `split_inclusive` keeps the bytes exactly as they are on disk.
fn lines_with_endings(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

/// Read an optional 1-based positive integer parameter.
fn optional_positive(input: &Value, key: &str) -> Result<Option<usize>, String> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => Ok(Some(n as usize)),
            _ => Err(format!(
                "Parameter '{key}' must be a positive integer (1-based line numbers); got {v}."
            )),
        },
    }
}

/// Largest index `<= max` that is a UTF-8 character boundary in `s`.
///
/// Slicing a `str` at a fixed byte offset panics when the offset falls
/// mid-codepoint, which any file containing non-ASCII text can trigger.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    let mut cut = max.min(s.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Cap `body` at `MAX_READ_BYTES`, cutting on a line boundary when there is
/// one. Returns `None` when `body` already fits.
///
/// The cut is moved back to the last newline inside the cap deliberately: half
/// a line handed to the model is half a line it will later quote back as an
/// `old_string` that does not exist on disk. A file with no newline at all in
/// its first 32 KB — a minified bundle — has no line boundary to find, so the
/// cut falls back to the nearest character boundary.
fn truncate_for_read(body: &str) -> Option<&str> {
    if body.len() <= MAX_READ_BYTES {
        return None;
    }
    let head = &body[..floor_char_boundary(body, MAX_READ_BYTES)];
    let cut = match head.rfind('\n') {
        Some(i) => i + 1,
        None => head.len(),
    };
    Some(&body[..cut])
}

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str { "file_read" }

    fn permission_info(&self) -> ToolPermissionInfo {
        ToolPermissionInfo {
            reads_files: true,
            path_inputs: &["path"],
            ..Default::default()
        }
    }

    fn description(&self) -> &str {
        "Read the text content of a file. Provide a path relative to the workspace \
         root (e.g. \"docs/report.md\"). Absolute paths are rejected when a workspace \
         root is configured. Output is capped at 32 KB; when the file is larger the \
         reply ends with an explicit truncation marker giving the file's real size \
         and the line to continue from. Use the optional 1-based `offset` and `limit` \
         parameters to page through a large file deliberately. Line endings are \
         returned exactly as they are on disk, so the text can be quoted back to \
         file_edit verbatim."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the workspace root."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional 1-based line number to start reading from. Defaults to 1."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional maximum number of lines to return. Defaults to the rest of the file."
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
        let offset = match optional_positive(&input, "offset") {
            Ok(v) => v,
            Err(e) => return ToolOutput::err(e),
        };
        let limit = match optional_positive(&input, "limit") {
            Ok(v) => v,
            Err(e) => return ToolOutput::err(e),
        };
        let abs_path = match resolve_path(requested, ctx) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        // `read_to_string`, not a lossy byte read: a non-UTF8 file must fail
        // loudly rather than arrive as replacement characters that the model
        // would then try to edit.
        let content = match tokio::fs::read_to_string(&abs_path).await {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::err(format!("Failed to read '{}': {e}", abs_path.display()))
            }
        };

        let lines = lines_with_endings(&content);
        let total_lines = lines.len();
        let total_bytes = content.len();

        // Select the requested line range as a byte slice of the original
        // content, so what comes back is byte-identical to what is on disk.
        let first_line = offset.unwrap_or(1);
        // `.max(1)` so that an empty file reads back as empty rather than as an
        // out-of-range error: it has no line 1, but asking for line 1 of it is
        // not a mistake.
        if first_line > total_lines.max(1) {
            return ToolOutput::err(format!(
                "Parameter 'offset' is {first_line} but '{requested}' has {total_lines} line(s)."
            ));
        }
        let last_line = match limit {
            Some(n) => (first_line - 1 + n).min(total_lines),
            None => total_lines,
        };
        let start: usize = lines[..first_line - 1].iter().map(|l| l.len()).sum();
        let end: usize =
            start + lines[first_line - 1..last_line].iter().map(|l| l.len()).sum::<usize>();
        let body = &content[start..end];

        let (shown, truncated) = match truncate_for_read(body) {
            Some(head) => (head, true),
            None => (body, false),
        };
        let shown_lines = lines_with_endings(shown).len();
        let last_shown = first_line + shown_lines.saturating_sub(1);

        if !truncated && first_line == 1 && last_line == total_lines {
            return ToolOutput::ok(shown.to_string());
        }

        // Anything short of the whole file carries a marker. It is bracketed,
        // prefixed with the tool name and phrased as a statement about the read
        // rather than about the subject matter, so it cannot be mistaken for a
        // line of the file.
        let mut out = String::with_capacity(shown.len() + 256);
        out.push_str(shown);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if truncated {
            // What is left to fetch, which is what the reader is about to act
            // on — not `total_bytes - shown.len()`. With an `offset`, `shown`
            // starts at byte `start`, so that form counts the skipped prefix as
            // outstanding and overstates the remainder by exactly the bytes
            // already behind the reader.
            let remaining = total_bytes.saturating_sub(start + shown.len());
            out.push_str(&format!(
                "\n[file_read TRUNCATED: the text above is NOT the complete file. \
                 '{requested}' is {total_bytes} bytes / {total_lines} lines; this reply \
                 carries lines {first_line}-{last_shown} ({} bytes) and {remaining} bytes \
                 remain after it. Call file_read again with \"offset\": {} to continue.]",
                shown.len(),
                last_shown + 1,
            ));
        } else {
            out.push_str(&format!(
                "\n[file_read PARTIAL: the text above is lines {first_line}-{last_shown} of \
                 {total_lines} in '{requested}', not the complete file.]"
            ));
        }
        ToolOutput::ok(out)
    }
}

// ---------------------------------------------------------------------------
// file_write
// ---------------------------------------------------------------------------

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str { "file_write" }

    fn permission_info(&self) -> ToolPermissionInfo {
        ToolPermissionInfo {
            writes_files: true,
            path_inputs: &["path"],
            ..Default::default()
        }
    }

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
// file_edit
// ---------------------------------------------------------------------------

/// Rewrite CRLF as LF. Used *only* to diagnose a failed match — never to
/// perform one. See `FileEditTool::execute`.
fn to_lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

pub struct FileEditTool;

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str { "file_edit" }

    fn description(&self) -> &str {
        "Change part of an existing file by replacing an exact substring, without \
         transporting the whole file. Prefer this over file_write for every edit to \
         a file that already exists. `old_string` must occur exactly once unless \
         `replace_all` is true; zero matches and ambiguous matches both fail without \
         touching the file. The match is byte-exact — indentation, trailing \
         whitespace and line endings all count — so quote `old_string` verbatim from \
         file_read output. Provide a path relative to the workspace root (e.g. \
         \"src/main.rs\")."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit, relative to the workspace root."
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to find, copied verbatim from the file. Include enough surrounding lines to make it unique."
                },
                "new_string": {
                    "type": "string",
                    "description": "Text to put in its place. Use an empty string to delete the matched text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence instead of requiring exactly one. Defaults to false."
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolOutput {
        let requested = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolOutput::err("Missing required parameter: path"),
        };
        let old_string = match input.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolOutput::err("Missing required parameter: old_string"),
        };
        let new_string = match input.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolOutput::err("Missing required parameter: new_string"),
        };
        let replace_all = match input.get("replace_all") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(v) => {
                return ToolOutput::err(format!(
                    "Parameter 'replace_all' must be a boolean; got {v}."
                ))
            }
        };

        if old_string.is_empty() {
            return ToolOutput::err(
                "Parameter 'old_string' must not be empty: an empty string matches at \
                 every position in the file. Supply the exact text to replace.",
            );
        }
        if old_string == new_string {
            return ToolOutput::err(
                "'old_string' and 'new_string' are identical, so the edit would change \
                 nothing. The file was not touched.",
            );
        }

        let abs_path = match resolve_path(requested, ctx) {
            Ok(p) => p,
            Err(e) => return ToolOutput::err(e),
        };
        // Same containment rules and the same explicit non-UTF8 failure as
        // file_read. file_edit never creates a file: an edit to something that
        // is not there is a mistake, not a create.
        let content = match tokio::fs::read_to_string(&abs_path).await {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::err(format!(
                    "Failed to read '{}' for editing: {e}",
                    abs_path.display()
                ))
            }
        };

        let count = content.matches(old_string).count();

        if count == 0 {
            let mut msg = format!(
                "file_edit found no match for old_string in '{requested}', so nothing was \
                 changed. The match is byte-exact: check indentation, trailing whitespace \
                 and line endings, and quote the text verbatim from file_read output."
            );
            // Diagnose the line-ending trap without papering over it. Matching
            // on a normalised form would let the model land an edit it did not
            // write, and rewriting the file's endings would produce a diff
            // touching every line — so say what is wrong and refuse.
            let normalised = to_lf(&content).matches(to_lf(old_string).as_str()).count();
            if normalised > 0 {
                msg.push_str(&format!(
                    " It would match {normalised} time(s) if line endings were ignored: \
                     the file uses {} line endings and old_string does not. Re-send \
                     old_string with the file's line endings. file_edit will not convert \
                     them for you, because that would rewrite every line of the file.",
                    if content.contains("\r\n") { "CRLF" } else { "LF" }
                ));
            }
            return ToolOutput::err(msg);
        }

        if count > 1 && !replace_all {
            return ToolOutput::err(format!(
                "file_edit: old_string matched {count} times in '{requested}', so nothing \
                 was changed — an edit that could land in more than one place is never \
                 resolved by guessing. Add surrounding lines to old_string to make it \
                 unique, or set \"replace_all\": true to replace all {count} occurrences."
            ));
        }

        // A plain substring replacement over the bytes read: every byte outside
        // the match — line endings and the trailing newline included — is
        // carried through untouched, so there is no normalisation step that
        // could turn a one-line edit into a whole-file diff.
        let updated = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        match tokio::fs::write(&abs_path, &updated).await {
            Ok(()) => ToolOutput::ok(format!(
                "Edited '{}': replaced {count} occurrence{} ({} bytes -> {} bytes).",
                abs_path.display(),
                if count == 1 { "" } else { "s" },
                content.len(),
                updated.len()
            )),
            Err(e) => ToolOutput::err(format!(
                "Failed to write '{}': {e}. The file is unchanged.",
                abs_path.display()
            )),
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

    fn permission_info(&self) -> ToolPermissionInfo {
        ToolPermissionInfo {
            reads_files: true,
            path_inputs: &["path"],
            ..Default::default()
        }
    }

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

    /// A leading separator is rejected on every platform rather than stripped.
    /// `is_absolute` alone would not do this: "/docs/x.md" is absolute on Unix
    /// but not on Windows, so relying on it made the same input behave
    /// differently per platform.
    #[tokio::test]
    async fn a_leading_separator_is_rejected_on_every_platform() {
        let (_dir, ctx) = rooted_ctx().await;

        for attempt in ["/docs/x.md", "\\docs\\x.md"] {
            let err = resolve_path(attempt, &ctx)
                .expect_err("a leading separator should be rejected");
            assert!(err.contains("Absolute paths are not allowed"), "got {err}");
        }
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

    // -- file_read: the size cap ---------------------------------------------

    /// Seed a file whose lines are exactly 64 bytes each, so the byte cap lands
    /// on a line boundary at a number the assertions can name.
    fn seed_padded_lines(root: &Path, name: &str, count: usize) -> String {
        let content: String = (0..count)
            .map(|i| format!("line {i:04} {}\n", "-".repeat(53)))
            .collect();
        assert_eq!(content.len(), count * 64, "line width assumption broke");
        std::fs::write(root.join(name), &content).expect("seed file");
        content
    }

    #[tokio::test]
    async fn file_read_returns_a_file_that_fits_the_cap_verbatim_with_no_marker() {
        let (_dir, ctx) = rooted_ctx().await;
        let content = seed_padded_lines(&root_of(&ctx), "small.txt", 100);

        let out = FileReadTool.execute(json!({ "path": "small.txt" }), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(out.content, content);
        assert!(!out.content.contains("file_read"), "an intact read must carry no marker");
    }

    #[tokio::test]
    async fn file_read_over_the_cap_truncates_and_states_the_real_size() {
        let (_dir, ctx) = rooted_ctx().await;
        // 600 * 64 = 38400 bytes, comfortably past the 32 KB cap.
        let content = seed_padded_lines(&root_of(&ctx), "big.txt", 600);

        let out = FileReadTool.execute(json!({ "path": "big.txt" }), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        // The head is a byte-exact prefix of the file, cut on a line boundary.
        let head = &content[..MAX_READ_BYTES];
        assert!(out.content.starts_with(head), "the head was not returned verbatim");
        assert!(head.ends_with('\n'));
        // ...and the model is told, in terms it cannot read as file content,
        // that this is not the whole file.
        assert!(out.content.contains("[file_read TRUNCATED:"), "got {:?}", out.content);
        assert!(out.content.contains("is NOT the complete file"));
        assert!(out.content.contains("38400 bytes / 600 lines"), "got {:?}", out.content);
        assert!(out.content.contains("lines 1-512"), "got {:?}", out.content);
        assert!(out.content.contains("5632 bytes remain after it"), "got {:?}", out.content);
        // ...and how to ask for the rest.
        assert!(out.content.contains("\"offset\": 513"), "got {:?}", out.content);
    }

    /// The remainder must be counted from the end of what was sent, not from
    /// the start of the file.
    ///
    /// With an `offset`, the returned slice begins partway in, so
    /// `total_bytes - shown.len()` counts the skipped prefix as still
    /// outstanding — overstating what is left by exactly the bytes the caller
    /// has already moved past, in the one number a paginating reader acts on.
    #[tokio::test]
    async fn a_truncated_read_at_an_offset_counts_only_what_follows_it() {
        let (_dir, ctx) = rooted_ctx().await;
        // 1200 * 64 = 76800 bytes, so a read starting at line 100 still has
        // more than the cap left to give.
        seed_padded_lines(&root_of(&ctx), "big.txt", 1200);

        let out = FileReadTool
            .execute(json!({ "path": "big.txt", "offset": 100 }), &ctx)
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert!(out.content.contains("[file_read TRUNCATED:"), "got {:?}", out.content);

        // start = 99 * 64 = 6336; shown = 32768 (512 whole lines).
        // Remaining is 76800 - 6336 - 32768 = 37696, not 76800 - 32768 = 44032.
        assert!(
            out.content.contains("37696 bytes remain after it"),
            "remainder must exclude the skipped prefix; got {:?}",
            out.content
        );
        assert!(
            !out.content.contains("44032"),
            "counted from the start of the file rather than the end of the reply: {:?}",
            out.content
        );
    }

    #[tokio::test]
    async fn file_read_never_cuts_mid_line() {
        let (_dir, ctx) = rooted_ctx().await;
        // 40 bytes per line divides 32768 unevenly, so a naive byte cut would
        // land in the middle of a line — and the model would quote that
        // fragment back to file_edit as text that is not on disk.
        const PER_LINE: usize = 40;
        let content: String = (0..1200).map(|i| format!("{i:04}{}\n", "z".repeat(35))).collect();
        assert_eq!(content.len(), 1200 * PER_LINE, "line width assumption broke");
        std::fs::write(root_of(&ctx).join("ragged.txt"), &content).expect("seed");

        let out = FileReadTool.execute(json!({ "path": "ragged.txt" }), &ctx).await;

        let whole_lines = MAX_READ_BYTES / PER_LINE;
        let cut = whole_lines * PER_LINE;
        assert!(cut < MAX_READ_BYTES, "the ragged-cut assumption broke");
        assert!(out.content.starts_with(&content[..cut]), "the head was not returned verbatim");
        assert!(
            !out.content.starts_with(&content[..cut + 1]),
            "the cut landed mid-line"
        );
        assert!(out.content.contains(&format!("lines 1-{whole_lines}")), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_read_truncation_does_not_panic_on_a_multibyte_boundary() {
        let (_dir, ctx) = rooted_ctx().await;
        // A minified-bundle shape: no newline anywhere, and a 3-byte codepoint
        // straddling the 32768-byte mark. Slicing there would panic.
        let content = "\u{20AC}".repeat(20_000);
        std::fs::write(root_of(&ctx).join("bundle.min.js"), &content).expect("seed");

        let out = FileReadTool.execute(json!({ "path": "bundle.min.js" }), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert!(out.content.contains("[file_read TRUNCATED:"), "got {:?}", out.content);
        let cut = (MAX_READ_BYTES / 3) * 3;
        assert!(cut < MAX_READ_BYTES, "the straddle assumption broke");
        assert!(out.content.starts_with(&content[..cut]));
        assert!(!out.content.starts_with(&content[..cut + 3]));
    }

    // -- file_read: offset / limit -------------------------------------------

    #[tokio::test]
    async fn file_read_offset_and_limit_return_exactly_that_line_range() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(
            root_of(&ctx).join("five.txt"),
            "alpha\nbravo\ncharlie\ndelta\necho\n",
        )
        .expect("seed");

        for (offset, limit, expected, span) in [
            (2, 2, "bravo\ncharlie\n", "lines 2-3 of 5"),
            (1, 1, "alpha\n", "lines 1-1 of 5"),
            (4, 99, "delta\necho\n", "lines 4-5 of 5"),
            (5, 1, "echo\n", "lines 5-5 of 5"),
        ] {
            let out = FileReadTool
                .execute(
                    json!({ "path": "five.txt", "offset": offset, "limit": limit }),
                    &ctx,
                )
                .await;

            assert!(!out.is_error, "got {:?}", out.content);
            assert!(
                out.content.starts_with(expected),
                "offset {offset} limit {limit} gave {:?}",
                out.content
            );
            assert!(out.content.contains("[file_read PARTIAL:"), "got {:?}", out.content);
            assert!(out.content.contains(span), "got {:?}", out.content);
        }
    }

    #[tokio::test]
    async fn file_read_offset_alone_reads_to_the_end() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("five.txt"), "a\nb\nc\nd\ne\n").expect("seed");

        let out = FileReadTool
            .execute(json!({ "path": "five.txt", "offset": 3 }), &ctx)
            .await;

        assert!(out.content.starts_with("c\nd\ne\n"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn a_range_covering_the_whole_file_is_not_marked_partial() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("five.txt"), "a\nb\nc\n").expect("seed");

        let out = FileReadTool
            .execute(json!({ "path": "five.txt", "offset": 1, "limit": 3 }), &ctx)
            .await;

        assert_eq!(out.content, "a\nb\nc\n");
    }

    #[tokio::test]
    async fn file_read_preserves_crlf_line_endings_in_a_range() {
        // The read is where a CRLF file would silently become an LF one, and
        // the model would then build a file_edit old_string that cannot match.
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("crlf.txt"), "alpha\r\nbravo\r\ncharlie\r\n")
            .expect("seed");

        let whole = FileReadTool.execute(json!({ "path": "crlf.txt" }), &ctx).await;
        assert_eq!(whole.content, "alpha\r\nbravo\r\ncharlie\r\n");

        let ranged = FileReadTool
            .execute(json!({ "path": "crlf.txt", "offset": 2, "limit": 1 }), &ctx)
            .await;
        assert!(ranged.content.starts_with("bravo\r\n"), "got {:?}", ranged.content);
    }

    #[tokio::test]
    async fn file_read_offset_past_the_end_is_an_error_naming_the_line_count() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("three.txt"), "a\nb\nc\n").expect("seed");

        let out = FileReadTool
            .execute(json!({ "path": "three.txt", "offset": 9 }), &ctx)
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("has 3 line(s)"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_read_rejects_offsets_and_limits_that_are_not_positive_integers() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("three.txt"), "a\nb\nc\n").expect("seed");

        for bad in [
            json!({ "path": "three.txt", "offset": 0 }),
            json!({ "path": "three.txt", "offset": -1 }),
            json!({ "path": "three.txt", "offset": "2" }),
            json!({ "path": "three.txt", "limit": 0 }),
            json!({ "path": "three.txt", "limit": 1.5 }),
        ] {
            let out = FileReadTool.execute(bad.clone(), &ctx).await;
            assert!(out.is_error, "{bad} was accepted");
            assert!(
                out.content.contains("must be a positive integer"),
                "{bad} gave {:?}",
                out.content
            );
        }
    }

    #[tokio::test]
    async fn file_read_on_an_empty_file_returns_nothing_without_a_marker() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("empty.txt"), "").expect("seed");

        let out = FileReadTool.execute(json!({ "path": "empty.txt" }), &ctx).await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(out.content, "");
    }

    // -- file_edit -----------------------------------------------------------

    /// Read a file back from disk without going through the tool.
    fn on_disk(ctx: &ToolContext, name: &str) -> String {
        std::fs::read_to_string(root_of(ctx).join(name)).expect("read back")
    }

    #[tokio::test]
    async fn file_edit_replaces_one_occurrence_and_leaves_the_rest_byte_identical() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(
            root_of(&ctx).join("main.rs"),
            "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n",
        )
        .expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "main.rs",
                    "old_string": "    let x = 1;\n",
                    "new_string": "    let x = 42;\n",
                }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(
            on_disk(&ctx, "main.rs"),
            "fn main() {\n    let x = 42;\n    println!(\"{x}\");\n}\n"
        );
        assert!(out.content.contains("replaced 1 occurrence"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_edit_can_delete_text_with_an_empty_new_string() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("a.txt"), "keep\ndrop\nkeep2\n").expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "a.txt", "old_string": "drop\n", "new_string": "" }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), "keep\nkeep2\n");
    }

    #[tokio::test]
    async fn file_edit_with_no_match_says_not_found_and_leaves_the_file_alone() {
        let (_dir, ctx) = rooted_ctx().await;
        let original = "alpha\nbravo\n";
        std::fs::write(root_of(&ctx).join("a.txt"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "a.txt", "old_string": "charlie", "new_string": "delta" }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("found no match"), "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), original, "the file was modified anyway");
    }

    #[tokio::test]
    async fn file_edit_with_several_matches_names_the_count_and_changes_nothing() {
        let (_dir, ctx) = rooted_ctx().await;
        let original = "todo\nkeep\ntodo\nkeep\ntodo\n";
        std::fs::write(root_of(&ctx).join("a.txt"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "a.txt", "old_string": "todo", "new_string": "done" }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("matched 3 times"), "got {:?}", out.content);
        // The error has to be actionable enough to retry without re-reading.
        assert!(out.content.contains("replace_all"), "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), original, "an ambiguous edit was applied");
    }

    #[tokio::test]
    async fn replace_all_replaces_every_occurrence_and_reports_the_count() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("a.txt"), "todo\nkeep\ntodo\nkeep\ntodo\n")
            .expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "a.txt",
                    "old_string": "todo",
                    "new_string": "done",
                    "replace_all": true,
                }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), "done\nkeep\ndone\nkeep\ndone\n");
        assert!(out.content.contains("replaced 3 occurrences"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn replace_all_is_off_by_default() {
        // Pinned separately from the multi-match error: the dangerous path must
        // stay opt-in even if the parameter parsing is reworked.
        let (_dir, ctx) = rooted_ctx().await;
        let original = "x\nx\n";
        std::fs::write(root_of(&ctx).join("a.txt"), original).expect("seed");

        let out = FileEditTool
            .execute(json!({ "path": "a.txt", "old_string": "x", "new_string": "y" }), &ctx)
            .await;

        assert!(out.is_error);
        assert_eq!(on_disk(&ctx, "a.txt"), original);
    }

    #[tokio::test]
    async fn file_edit_preserves_crlf_endings_and_the_trailing_newline() {
        // A tool that normalises line endings turns a one-line change into a
        // diff touching every line of the file.
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("crlf.txt"), "alpha\r\nbravo\r\ncharlie\r\n")
            .expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "crlf.txt", "old_string": "bravo", "new_string": "BRAVO" }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "crlf.txt"), "alpha\r\nBRAVO\r\ncharlie\r\n");
    }

    #[tokio::test]
    async fn file_edit_does_not_add_a_trailing_newline_to_a_file_without_one() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("a.txt"), "alpha\nbravo").expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "a.txt", "old_string": "bravo", "new_string": "charlie" }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), "alpha\ncharlie");
    }

    #[tokio::test]
    async fn a_line_ending_mismatch_is_explained_rather_than_silently_normalised() {
        // The CRLF trap: an LF old_string against a CRLF file. Matching on a
        // normalised form would apply an edit the model did not write, so the
        // tool refuses — but it says exactly why, so the retry is one call.
        let (_dir, ctx) = rooted_ctx().await;
        let original = "fn a() {\r\n    one();\r\n    two();\r\n}\r\n";
        std::fs::write(root_of(&ctx).join("crlf.rs"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "crlf.rs",
                    "old_string": "    one();\n    two();\n",
                    "new_string": "    two();\n    one();\n",
                }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("CRLF"), "got {:?}", out.content);
        assert!(
            out.content.contains("line endings were ignored"),
            "got {:?}",
            out.content
        );
        assert_eq!(on_disk(&ctx, "crlf.rs"), original);

        // The same edit with the file's own line endings goes through.
        let retry = FileEditTool
            .execute(
                json!({
                    "path": "crlf.rs",
                    "old_string": "    one();\r\n    two();\r\n",
                    "new_string": "    two();\r\n    one();\r\n",
                }),
                &ctx,
            )
            .await;
        assert!(!retry.is_error, "got {:?}", retry.content);
        assert_eq!(on_disk(&ctx, "crlf.rs"), "fn a() {\r\n    two();\r\n    one();\r\n}\r\n");
    }

    #[tokio::test]
    async fn file_edit_matches_bytes_not_a_trimmed_form() {
        // Whitespace-insensitive matching would make this succeed against the
        // differently-indented line, which is not what was asked for.
        let (_dir, ctx) = rooted_ctx().await;
        let original = "        deeply_indented();\n";
        std::fs::write(root_of(&ctx).join("a.rs"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "a.rs",
                    "old_string": "deeply_indented();\n    ",
                    "new_string": "x();\n",
                }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert_eq!(on_disk(&ctx, "a.rs"), original);
    }

    #[tokio::test]
    async fn file_edit_cannot_escape_the_workspace_root() {
        // The whole containment suite above exercises `resolve_path` directly;
        // this pins that file_edit actually goes through it rather than around
        // it, by putting a real file just outside the root and checking it
        // survives untouched.
        let dir = TempDir::new().expect("temp dir");
        let base = std::fs::canonicalize(dir.path()).expect("canonicalize");
        let root = base.join("proj");
        std::fs::create_dir_all(&root).expect("mkdir root");
        let victim = base.join("victim.txt");
        std::fs::write(&victim, "secret\n").expect("seed outside");

        let mut ctx = make_ctx(make_test_pool().await);
        ctx.workspace_root = Some(strip_unc(&root));

        let out = FileEditTool
            .execute(
                json!({
                    "path": "../victim.txt",
                    "old_string": "secret",
                    "new_string": "pwned",
                }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("outside the workspace root"), "got {:?}", out.content);
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "secret\n");
    }

    #[tokio::test]
    async fn file_edit_rejects_an_absolute_path_like_the_other_file_tools() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileEditTool
            .execute(
                json!({ "path": "/etc/passwd", "old_string": "root", "new_string": "x" }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("Absolute paths are not allowed"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_edit_reports_each_missing_parameter_by_name() {
        let (_dir, ctx) = rooted_ctx().await;

        for (input, expected) in [
            (json!({ "old_string": "a", "new_string": "b" }), "path"),
            (json!({ "path": "a.txt", "new_string": "b" }), "old_string"),
            (json!({ "path": "a.txt", "old_string": "a" }), "new_string"),
        ] {
            let out = FileEditTool.execute(input.clone(), &ctx).await;
            assert!(out.is_error, "{input} was accepted");
            assert!(out.content.contains(expected), "{input} gave {:?}", out.content);
        }
    }

    #[tokio::test]
    async fn file_edit_rejects_an_empty_old_string() {
        // An empty needle matches between every pair of characters; with
        // replace_all it would splice new_string across the whole file.
        let (_dir, ctx) = rooted_ctx().await;
        let original = "alpha\n";
        std::fs::write(root_of(&ctx).join("a.txt"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "a.txt",
                    "old_string": "",
                    "new_string": "X",
                    "replace_all": true,
                }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("must not be empty"), "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), original);
    }

    #[tokio::test]
    async fn file_edit_rejects_an_edit_that_would_change_nothing() {
        let (_dir, ctx) = rooted_ctx().await;
        std::fs::write(root_of(&ctx).join("a.txt"), "alpha\n").expect("seed");

        let out = FileEditTool
            .execute(
                json!({ "path": "a.txt", "old_string": "alpha", "new_string": "alpha" }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("identical"), "got {:?}", out.content);
    }

    #[tokio::test]
    async fn file_edit_rejects_a_non_boolean_replace_all() {
        let (_dir, ctx) = rooted_ctx().await;
        let original = "x\nx\n";
        std::fs::write(root_of(&ctx).join("a.txt"), original).expect("seed");

        let out = FileEditTool
            .execute(
                json!({
                    "path": "a.txt",
                    "old_string": "x",
                    "new_string": "y",
                    "replace_all": "true",
                }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("must be a boolean"), "got {:?}", out.content);
        assert_eq!(on_disk(&ctx, "a.txt"), original);
    }

    #[tokio::test]
    async fn file_edit_does_not_create_a_missing_file() {
        let (_dir, ctx) = rooted_ctx().await;

        let out = FileEditTool
            .execute(
                json!({ "path": "nope.txt", "old_string": "a", "new_string": "b" }),
                &ctx,
            )
            .await;

        assert!(out.is_error);
        assert!(out.content.contains("Failed to read"), "got {:?}", out.content);
        assert!(!root_of(&ctx).join("nope.txt").exists(), "file_edit created the file");
    }

    #[tokio::test]
    async fn editing_a_large_file_transports_the_changed_strings_and_not_the_body() {
        // The whole point of the tool: the reply that lands in the transcript
        // must not carry the file.
        let (_dir, ctx) = rooted_ctx().await;
        let content = seed_padded_lines(&root_of(&ctx), "big.txt", 600);
        let target = "line 0300 ";

        let out = FileEditTool
            .execute(
                json!({
                    "path": "big.txt",
                    "old_string": target,
                    "new_string": "LINE 0300 ",
                }),
                &ctx,
            )
            .await;

        assert!(!out.is_error, "got {:?}", out.content);
        assert!(
            out.content.len() < 256,
            "the reply carried {} bytes for a 38400-byte file: {:?}",
            out.content.len(),
            out.content
        );
        assert!(!out.content.contains("line 0299"), "the reply echoed the file body");

        let after = on_disk(&ctx, "big.txt");
        assert_eq!(after.len(), content.len());
        assert_eq!(after.replace("LINE 0300 ", "line 0300 "), content);
    }
}
