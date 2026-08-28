// Git helpers for capturing workspace state before/after a run.
//
// All operations are best-effort: if git is not installed or the workspace is
// not a git repository, the functions return `None` rather than erroring.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `git rev-parse HEAD` in `workspace_root`.
///
/// Returns `Some(sha)` when inside a git repo, `None` otherwise.
pub fn get_head_sha(workspace_root: &Path) -> Option<String> {
    let output = Command::new("git")
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

/// A scratch index file, deleted when the guard drops.
struct ScratchIndex(PathBuf);

impl ScratchIndex {
    fn new() -> Self {
        Self(
            std::env::temp_dir()
                .join(format!("rustyagent-index-{}", uuid::Uuid::new_v4())),
        )
    }
}

impl Drop for ScratchIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Unified diff of everything in `workspace_root` that differs from
/// `before_sha` — **including files the run created**.
///
/// # Why this is not just `git diff <sha>`
///
/// It used to be, and untracked files were therefore missing from every
/// recorded diff. Creating a file is the most common thing a coding agent
/// does, so an operator reviewing a run saw an incomplete picture with nothing
/// telling them it was incomplete.
///
/// The usual fix is `git add -N .` before the diff. That is not used here
/// because this function also runs against the user's live checkout, in the
/// fallback where a run could not be isolated, and `git add -N` mutates the
/// real index: it makes untracked files show as staged additions in the user's
/// own `git status`, and changes what `git stash` and `git commit -a` do. A
/// function whose job is to *observe* must not leave that behind.
///
/// Instead the whole working tree is staged into a throwaway index — pointed
/// at by `GIT_INDEX_FILE`, deleted on the way out — and the diff is taken
/// against that. The repository's real index is never opened for writing.
/// `git add -A` honours `.gitignore`, so ignored build output stays out of the
/// diff exactly as it did before.
///
/// Returns `None` when there are no changes, or on any error — including
/// non-git workspaces and a missing git binary.
pub fn get_diff_since(workspace_root: &Path, before_sha: &str) -> Option<String> {
    let index = ScratchIndex::new();

    // Seed the scratch index with the working tree as it stands. Starting from
    // an absent (therefore empty) index means every file lands in it, tracked
    // or not.
    let add = Command::new("git")
        .args(["add", "-A", "--", "."])
        .current_dir(workspace_root)
        .env("GIT_INDEX_FILE", &index.0)
        .output()
        .ok()?;
    if !add.status.success() {
        return None;
    }

    // `<commit>` against the index: what the run turned `before_sha` into.
    let output = Command::new("git")
        .args(["diff", "--cached", before_sha])
        .current_dir(workspace_root)
        .env("GIT_INDEX_FILE", &index.0)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let diff = String::from_utf8_lossy(&output.stdout).to_string();
    if diff.is_empty() { None } else { Some(diff) }
}
