// Git helpers for capturing workspace state before/after a run.
//
// All operations are best-effort: if git is not installed or the workspace is
// not a git repository, the functions return `None` rather than erroring.

use std::path::Path;

/// Run `git rev-parse HEAD` in `workspace_root`.
///
/// Returns `Some(sha)` when inside a git repo, `None` otherwise.
pub fn get_head_sha(workspace_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

/// Run `git diff <before_sha>` in `workspace_root`.
///
/// Returns the unified diff text, or `None` if there are no changes or on
/// error (including non-git workspaces and missing git).
pub fn get_diff_since(workspace_root: &Path, before_sha: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["diff", before_sha])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    if diff.is_empty() { None } else { Some(diff) }
}
