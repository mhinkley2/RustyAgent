//! Workspace filesystem — reads only.
//!
//! Both tools delegate to `commands::filesystem`, which canonicalizes the path
//! and verifies it stays inside the active workspace root (resolving symlinks
//! and Windows junctions first). No write, rename, duplicate, create, or delete
//! command is exposed: those are available in the app, and an MCP client has no
//! need to reach for them.
//!
//! Both also cap what they return. `read_file` shares its cap, its 1-based
//! `offset` / `limit` semantics and its truncation marker with the internal
//! `file_read` agent tool, via [`tools::read_cap`] — an external agent that has
//! learned one of the two read tools must not be surprised by the other.

use serde_json::json;
use tools::{
    read_cap::{optional_positive, read_page},
    ToolOutput,
};

use crate::{
    mcp_tool,
    paging::{paged_rows, page_request, NO_FULLER_FORM},
    registry::{json_ok, str_arg},
};

/// Directory entries per page by default, and the ceiling on `limit`.
///
/// Entries are small and fixed-shape — a name, a path, a size — so the byte
/// budget rarely bites here and the row limit is what does the bounding. A
/// generated or vendored directory with tens of thousands of children is the
/// case this exists for; the build and VCS directories that usually hold them
/// are already skipped, but "usually" is not a bound.
const DIR_DEFAULT_LIMIT: usize = 200;
const DIR_MAX_LIMIT: usize = 1000;

mcp_tool! {
    pub ListDirectoryTool,
    name        = "list_directory",
    description = "List the immediate children of a directory inside the active workspace. \
                   Directories sort first. Build and VCS directories (.git, node_modules, \
                   target, dist, …) are skipped. The reply is paged: it returns at most 200 \
                   entries by default (1000 with an explicit `limit`) and reports `total`, \
                   `complete` and a `next_offset` to continue from.",
    schema      = {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute path inside the workspace" },
            "offset": {
                "type": "integer",
                "minimum": 1,
                "description": "1-based index of the first entry to return. Defaults to 1."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": "Maximum entries to return. Defaults to 200, clamped to 1000."
            }
        },
        "required": ["path"]
    },
    |input, ctx| {
        let Some(path) = str_arg(&input, "path") else {
            return ToolOutput::err("Missing required field: path");
        };
        let request = match page_request(&input, DIR_DEFAULT_LIMIT, DIR_MAX_LIMIT) {
            Ok(request) => request,
            Err(error) => return ToolOutput::err(error),
        };
        let entries = match commands::filesystem::list_directory(path.clone(), &ctx.db).await {
            Ok(entries) => entries,
            Err(error) => return ToolOutput::err(error),
        };
        // No text fields to cap: every column is a path or a number, so the
        // row limit alone bounds this one.
        match paged_rows(
            entries,
            request,
            "list_directory",
            "entries",
            &format!("'{path}'"),
            &[],
            NO_FULLER_FORM,
        ) {
            Ok(envelope) => json_ok(envelope),
            Err(error) => ToolOutput::err(error),
        }
    }
}

mcp_tool! {
    pub ReadFileTool,
    name        = "read_file",
    description = "Read a UTF-8 text file inside the active workspace. Two separate limits \
                   apply. Files over 10 MB are refused outright. Below that, the reply is \
                   capped at 32 KB of file text: when the file is larger, the content ends \
                   with an explicit [read_file TRUNCATED: …] marker giving the file's real \
                   size in bytes and lines, the line range this reply carries, and the \
                   `offset` to call read_file with to continue. Use the optional 1-based \
                   `offset` and `limit` parameters to page through a large file deliberately. \
                   The reply also carries `truncated`, `complete` and `next_offset` fields for \
                   programmatic paging. Line endings are returned exactly as they are on disk.",
    schema      = {
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute path inside the workspace" },
            "offset": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional 1-based line number to start reading from. Defaults to 1."
            },
            "limit": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional maximum number of lines to return. Defaults to the rest \
                                of the file, subject to the 32 KB cap."
            }
        },
        "required": ["path"]
    },
    |input, ctx| {
        let Some(path) = str_arg(&input, "path") else {
            return ToolOutput::err("Missing required field: path");
        };
        let offset = match optional_positive(&input, "offset") {
            Ok(value) => value,
            Err(error) => return ToolOutput::err(error),
        };
        let limit = match optional_positive(&input, "limit") {
            Ok(value) => value,
            Err(error) => return ToolOutput::err(error),
        };

        // `read_file_text` keeps the workspace confinement check and the 10 MB
        // refusal. That refusal is a *memory* guard — it stops this process
        // allocating half a gigabyte — and it is not a substitute for the
        // context cap applied just below, which stops a 9 MB file that passes
        // the memory guard from flooding the caller's context window. Both are
        // load-bearing; deleting either leaves a real hole.
        //
        // The whole file is read and then capped rather than seeked into. That
        // costs at most the 10 MB the memory guard already permits, and it is
        // what keeps this path byte-identical to the internal `file_read` — a
        // second, cleverer range reader here is precisely the drift this story
        // exists to prevent.
        let content = match commands::filesystem::read_file_text(path.clone(), &ctx.db).await {
            Ok(content) => content,
            Err(error) => return ToolOutput::err(error),
        };

        match read_page(&content, offset, limit, "read_file", &path) {
            // `content` carries the marker inline so a model reading the text
            // block cannot miss it; the sibling fields say the same thing in a
            // form a program can branch on.
            Ok(page) => json_ok(json!({
                "path": path,
                "content": page.text,
                "truncated": page.truncated,
                "complete": !page.partial,
                "first_line": page.first_line,
                "last_line": page.last_line,
                "total_lines": page.total_lines,
                "total_bytes": page.total_bytes,
                "next_offset": page.next_offset,
            })),
            Err(error) => ToolOutput::err(error),
        }
    }
}
